use crate::errors::EscrowError;
use crate::events::EscrowCreated;
use crate::state::{Escrow, EscrowStatus, ESCROW_ACCOUNT_VERSION};
use anchor_lang::prelude::*;

/// Accounts required for creating a new escrow
#[derive(Accounts)]
#[instruction(
    escrow_id: [u8; 32],
    escrow_group_id: [u8; 32],
    sequence_in_group: u8,
    total_in_group: u8,
    amount: u64,
    is_native: bool,
    commission_rate_bps: u16,
    auto_release_at: i64,
)]
pub struct CreateEscrow<'info> {
    /// The escrow account to be created (PDA)
    #[account(
        init,
        payer = employer,
        space = Escrow::SPACE,
        seeds = [Escrow::ESCROW_SEED, escrow_id.as_ref()],
        bump
    )]
    pub escrow: Account<'info, Escrow>,

    /// The employer creating and funding the escrow
    #[account(mut)]
    pub employer: Signer<'info>,

    /// The employee who will receive payment
    /// CHECK: We only store this pubkey, no data access needed
    pub employee: UncheckedAccount<'info>,

    /// The platform authority for dispute resolution
    /// CHECK: We only store this pubkey, no data access needed
    pub platform_authority: UncheckedAccount<'info>,

    /// The token mint (SystemProgram ID for native SOL)
    /// CHECK: Validated based on is_native flag
    pub token_mint: UncheckedAccount<'info>,

    /// System program for account creation
    pub system_program: Program<'info, System>,
}

/// Creates a new escrow account for a hire / milestone.
///
/// # Arguments
/// * `escrow_id` - Unique identifier (typically SHA256 of milestone id)
/// * `escrow_group_id` - 32-byte group id (typically SHA256 of hire id) so
///   indexers can collect all milestone escrows of a hire. Pass [0u8; 32]
///   when the escrow is not part of a group.
/// * `sequence_in_group` - 1-indexed milestone position (1..=total_in_group).
///   Pass 0 if ungrouped.
/// * `total_in_group` - total milestones in the group. Pass 0 if ungrouped.
/// * `amount` - worker payment in lamports / token base units (must be > 0)
/// * `is_native` - true for SOL, false for SPL Token
/// * `commission_rate_bps` - basis points (max 1000)
/// * `auto_release_at` - unix ts after which anyone may auto-release to
///   employee. 0 disables auto-release.
#[allow(clippy::too_many_arguments)]
pub fn handler(
    ctx: Context<CreateEscrow>,
    escrow_id: [u8; 32],
    escrow_group_id: [u8; 32],
    sequence_in_group: u8,
    total_in_group: u8,
    amount: u64,
    is_native: bool,
    commission_rate_bps: u16,
    auto_release_at: i64,
) -> Result<()> {
    let employer_key = ctx.accounts.employer.key();
    let employee_key = ctx.accounts.employee.key();
    let platform_key = ctx.accounts.platform_authority.key();
    let token_mint_key = ctx.accounts.token_mint.key();

    // Validate amount is non-zero
    require!(amount > 0, EscrowError::InvalidAmount);

    // Validate the commission rate doesn't exceed maximum
    require!(
        commission_rate_bps <= Escrow::MAX_COMMISSION_RATE_BPS,
        EscrowError::InvalidCommissionRate
    );

    // Parties must be distinct
    require!(employer_key != employee_key, EscrowError::EmployeeIsEmployer);
    require!(
        platform_key != employer_key && platform_key != employee_key,
        EscrowError::PlatformAuthorityConflict
    );

    // is_native ↔ token_mint must be consistent. Native escrows pin the
    // mint field to System Program ID for clarity; token escrows must not.
    if is_native {
        require!(
            token_mint_key == anchor_lang::system_program::ID,
            EscrowError::IsNativeMintMismatch
        );
    } else {
        require!(
            token_mint_key != anchor_lang::system_program::ID,
            EscrowError::IsNativeMintMismatch
        );
    }

    // Group sequence sanity. If grouped (total > 0), seq must be in [1, total].
    if total_in_group > 0 {
        require!(
            sequence_in_group >= 1 && sequence_in_group <= total_in_group,
            EscrowError::InvalidGroupSequence
        );
    } else {
        require!(sequence_in_group == 0, EscrowError::InvalidGroupSequence);
    }

    // Calculate commission (0 if rate is 0)
    let commission_amount = Escrow::calculate_commission(amount, commission_rate_bps);

    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;

    // Auto-release must be in the future and within MAX_AUTO_RELEASE_DURATION
    // if configured. Far-future deadlines lock funds without recourse.
    if auto_release_at != 0 {
        require!(
            auto_release_at > clock.unix_timestamp,
            EscrowError::InvalidAutoReleaseTime
        );
        require!(
            auto_release_at - clock.unix_timestamp <= Escrow::MAX_AUTO_RELEASE_DURATION,
            EscrowError::AutoReleaseTooFar
        );
    }

    // Derive vault bump for later use
    let (_, vault_bump) = Pubkey::find_program_address(
        &[Escrow::VAULT_SEED, escrow.key().as_ref()],
        ctx.program_id,
    );

    // Initialize escrow account
    escrow.version = ESCROW_ACCOUNT_VERSION;
    escrow.escrow_id = escrow_id;
    escrow.escrow_group_id = escrow_group_id;
    escrow.sequence_in_group = sequence_in_group;
    escrow.total_in_group = total_in_group;
    escrow.employer = employer_key;
    escrow.employee = employee_key;
    escrow.platform_authority = platform_key;
    escrow.amount = amount;
    escrow.commission_amount = commission_amount;
    escrow.commission_rate_bps = commission_rate_bps;
    escrow.released_to_employee = 0;
    escrow.token_mint = ctx.accounts.token_mint.key();
    escrow.is_native = is_native;
    escrow.status = EscrowStatus::Created;
    escrow.employer_confirmed = false;
    escrow.employee_confirmed = false;
    escrow.created_at = clock.unix_timestamp;
    escrow.funded_at = 0;
    escrow.completed_at = 0;
    escrow.auto_release_at = auto_release_at;
    escrow.release_initiator = Pubkey::default();
    escrow.dispute_reason = [0u8; Escrow::MAX_DISPUTE_REASON_LEN];
    escrow.dispute_raised_by = Pubkey::default();
    escrow.dispute_raised_at = 0;
    escrow.dispute_deadline = 0;
    escrow.dispute_resolved_by = Pubkey::default();
    escrow.dispute_resolved_at = 0;
    escrow.employee_share_resolved = 0;
    escrow.employer_share_resolved = 0;
    escrow.cancellation_reason = [0u8; Escrow::MAX_CANCELLATION_REASON_LEN];
    escrow.cancelled_by = Pubkey::default();
    escrow.bump = ctx.bumps.escrow;
    escrow.vault_bump = vault_bump;

    emit!(EscrowCreated {
        escrow_id,
        escrow_group_id,
        sequence_in_group,
        total_in_group,
        employer: employer_key,
        employee: employee_key,
        platform_authority: platform_key,
        amount,
        commission_amount,
        commission_rate_bps,
        is_native,
        token_mint: ctx.accounts.token_mint.key(),
        auto_release_at,
    });

    msg!(
        "Escrow created v{} id={:?} amount={} commission={} ({}bps) auto_release_at={}",
        ESCROW_ACCOUNT_VERSION,
        escrow_id,
        amount,
        commission_amount,
        commission_rate_bps,
        auto_release_at
    );

    Ok(())
}

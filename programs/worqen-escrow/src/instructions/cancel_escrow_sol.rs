use crate::errors::EscrowError;
use crate::events::EscrowCancelled;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts required for cancelling a SOL escrow
#[derive(Accounts)]
pub struct CancelEscrowSol<'info> {
    /// The escrow account
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Created || escrow.status == EscrowStatus::Funded @ EscrowError::InvalidStatus,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
    )]
    pub escrow: Account<'info, Escrow>,

    /// The vault PDA holding the SOL
    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: This is a PDA that holds SOL
    pub escrow_vault: UncheckedAccount<'info>,

    /// The employer receiving the refund
    #[account(
        mut,
        constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.employer
    pub employer: UncheckedAccount<'info>,

    /// The signer (employer or platform authority)
    pub signer: Signer<'info>,

    /// System program
    pub system_program: Program<'info, System>,
}

/// Cancels escrow and refunds employer (full vault balance).
///
/// **v2 authorization rules:**
/// - In `Created` state (no funds deposited): employer or platform_authority.
/// - In `Funded` state: **platform_authority only**. The employer cannot
///   unilaterally pull a funded escrow back — once money is in the vault,
///   the worker may have started in good faith. The employer must raise
///   a dispute and have the platform mediate.
///
/// # Arguments
/// * `reason` - UTF-8 cancellation reason (max 128 bytes)
pub fn handler(ctx: Context<CancelEscrowSol>, reason: Vec<u8>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let signer_key = ctx.accounts.signer.key();

    require!(
        signer_key == escrow.employer || signer_key == escrow.platform_authority,
        EscrowError::Unauthorized
    );

    // Once funded, only the platform can cancel. Employer must dispute.
    if escrow.status == EscrowStatus::Funded {
        require!(
            signer_key == escrow.platform_authority,
            EscrowError::EmployerCancelAfterFundedDisallowed
        );
    }

    require!(
        reason.len() <= Escrow::MAX_CANCELLATION_REASON_LEN,
        EscrowError::CancellationReasonTooLong
    );

    let clock = Clock::get()?;
    let vault_balance = ctx.accounts.escrow_vault.lamports();

    let worker_amount = escrow.amount;
    let commission_amount = escrow.commission_amount;

    let escrow_key = escrow.key();
    let vault_seeds = &[
        Escrow::VAULT_SEED,
        escrow_key.as_ref(),
        &[escrow.vault_bump],
    ];
    let signer_seeds = &[&vault_seeds[..]];

    if vault_balance > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employer.to_account_info(),
                },
                signer_seeds,
            ),
            vault_balance,
        )?;
    }

    let mut buf = [0u8; Escrow::MAX_CANCELLATION_REASON_LEN];
    buf[..reason.len()].copy_from_slice(&reason);

    escrow.status = EscrowStatus::Cancelled;
    escrow.completed_at = clock.unix_timestamp;
    escrow.cancellation_reason = buf;
    escrow.cancelled_by = signer_key;

    emit!(EscrowCancelled {
        escrow_id: escrow.escrow_id,
        cancelled_by: signer_key,
        refunded_to: escrow.employer,
        amount_refunded: worker_amount,
        commission_refunded: commission_amount,
        is_native: true,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Escrow cancelled by {:?}, {} lamports refunded to employer",
        signer_key,
        vault_balance
    );

    Ok(())
}

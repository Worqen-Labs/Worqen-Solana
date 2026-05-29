use crate::errors::EscrowError;
use crate::events::EscrowReleased;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts required for releasing SOL from escrow
#[derive(Accounts)]
pub struct ReleaseSol<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::PendingRelease || escrow.status == EscrowStatus::Funded @ EscrowError::InvalidStatus,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
    )]
    pub escrow: Account<'info, Escrow>,

    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: This is a PDA that holds SOL
    pub escrow_vault: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.employee
    pub employee: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = fee_recipient.key() == escrow.fee_recipient @ EscrowError::InvalidFeeRecipient,
    )]
    /// CHECK: Verified against escrow.fee_recipient
    pub fee_recipient: UncheckedAccount<'info>,

    /// Employer, platform authority, or worker (see handler for the rules).
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// Releases escrowed SOL: the worker amount to the employee and the remaining
/// balance (commission plus any dust) to the treasury. The worker self-release
/// path (both parties confirmed) covers the "employer confirmed then ghosted"
/// case without a dispute. The vault is drained to its actual balance, not the
/// recorded amounts, so dust deposits cannot strand it below rent-exempt minimum.
pub fn handler(ctx: Context<ReleaseSol>, ref_id: [u8; 32]) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let authority_key = ctx.accounts.authority.key();

    // Authorization: employer-with-confirmation, platform_authority, or
    // worker after both parties confirmed.
    let is_employer_authorized = authority_key == escrow.employer && escrow.employer_confirmed;
    let is_platform_authorized = authority_key == escrow.platform_authority;
    let is_worker_authorized =
        authority_key == escrow.employee && escrow.employer_confirmed && escrow.employee_confirmed;

    require!(
        is_employer_authorized || is_platform_authorized || is_worker_authorized,
        EscrowError::ReleaseNotAuthorized
    );

    let clock = Clock::get()?;

    let worker_amount = escrow.remaining_worker_amount();
    require!(worker_amount > 0, EscrowError::InsufficientFunds);

    let escrow_key = escrow.key();
    let vault_seeds = &[
        Escrow::VAULT_SEED,
        escrow_key.as_ref(),
        &[escrow.vault_bump],
    ];
    let signer_seeds = &[&vault_seeds[..]];

    transfer(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.escrow_vault.to_account_info(),
                to: ctx.accounts.employee.to_account_info(),
            },
            signer_seeds,
        ),
        worker_amount,
    )?;

    // Drain the rest (commission + any dust deposit) to the treasury.
    // This guarantees the vault ends at exactly 0, which Solana requires
    // for any sub-rent-exempt remainder.
    let commission_amount = ctx.accounts.escrow_vault.lamports();
    if commission_amount > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.fee_recipient.to_account_info(),
                },
                signer_seeds,
            ),
            commission_amount,
        )?;
    }

    escrow.released_to_employee = escrow.amount;
    escrow.status = EscrowStatus::Released;
    escrow.completed_at = clock.unix_timestamp;
    escrow.release_initiator = authority_key;

    emit!(EscrowReleased {
        escrow_id: escrow.escrow_id,
        recipient: escrow.employee,
        amount: worker_amount,
        commission_amount,
        commission_recipient: escrow.fee_recipient,
        is_native: true,
        token_mint: escrow.token_mint,
        initiator: authority_key,
        is_partial: false,
        remaining_worker_amount: 0,
        ref_id,
    });

    msg!(
        "Released {} lamports to employee, {} lamports to treasury",
        worker_amount,
        commission_amount
    );

    Ok(())
}

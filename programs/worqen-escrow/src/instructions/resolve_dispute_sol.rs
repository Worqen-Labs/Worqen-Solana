use crate::errors::EscrowError;
use crate::events::DisputeResolved;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts required for resolving a SOL dispute
#[derive(Accounts)]
pub struct ResolveDisputeSol<'info> {
    /// The escrow account
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Disputed @ EscrowError::InvalidStatus,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
        constraint = escrow.platform_authority == platform_authority.key() @ EscrowError::Unauthorized,
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

    /// The employer receiving refund portion (+ commission refund)
    #[account(
        mut,
        constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.employer
    pub employer: UncheckedAccount<'info>,

    /// The employee receiving their portion
    #[account(
        mut,
        constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.employee
    pub employee: UncheckedAccount<'info>,

    /// The platform authority
    pub platform_authority: Signer<'info>,

    /// System program
    pub system_program: Program<'info, System>,
}

/// Platform resolves dispute by splitting the *remaining* worker payment
/// between parties. Commission is refunded to employer (proportional to the
/// remaining worker amount); platform forfeits commission on dispute.
///
/// The vault is drained to its actual balance, not the recorded amounts,
/// so any dust deposit is swept to the employer along with their share.
/// This avoids the rent-exempt-min DoS that would otherwise brick resolve.
///
/// # Arguments
/// * `employee_share` - lamports from the remaining worker payment to send
///   to the employee. `employer_share = remaining_worker - employee_share`.
pub fn handler(ctx: Context<ResolveDisputeSol>, employee_share: u64) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;

    let remaining_worker = escrow.remaining_worker_amount();
    let remaining_commission = escrow.remaining_commission();

    require!(
        employee_share <= remaining_worker,
        EscrowError::InvalidEmployeeShare
    );

    let employer_share = remaining_worker - employee_share;

    let escrow_key = escrow.key();
    let vault_seeds = &[
        Escrow::VAULT_SEED,
        escrow_key.as_ref(),
        &[escrow.vault_bump],
    ];
    let signer_seeds = &[&vault_seeds[..]];

    // Pay the worker their share first.
    if employee_share > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employee.to_account_info(),
                },
                signer_seeds,
            ),
            employee_share,
        )?;
    }

    // Drain everything else (employer share + commission refund + any dust)
    // to the employer. Vault ends at exactly 0.
    let total_to_employer = ctx.accounts.escrow_vault.lamports();
    if total_to_employer > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employer.to_account_info(),
                },
                signer_seeds,
            ),
            total_to_employer,
        )?;
    }

    escrow.released_to_employee = escrow.released_to_employee.saturating_add(employee_share);
    escrow.status = EscrowStatus::Resolved;
    escrow.completed_at = clock.unix_timestamp;
    escrow.dispute_resolved_by = ctx.accounts.platform_authority.key();
    escrow.dispute_resolved_at = clock.unix_timestamp;
    escrow.employee_share_resolved = employee_share;
    escrow.employer_share_resolved = employer_share;

    emit!(DisputeResolved {
        escrow_id: escrow.escrow_id,
        resolver: ctx.accounts.platform_authority.key(),
        employee_share,
        employer_share,
        commission_refunded: remaining_commission,
        is_native: true,
        token_mint: escrow.token_mint,
        forced: false,
    });

    msg!(
        "Dispute resolved: {} to employee, {} to employer (incl {} commission refund)",
        employee_share,
        total_to_employer,
        remaining_commission
    );

    Ok(())
}

use crate::errors::EscrowError;
use crate::events::DisputeResolved;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Anyone-can-call platform-failure safety net for SOL escrows: when a
/// Disputed escrow's `dispute_deadline` has passed, force-resolves in favor
/// of the worker (remaining worker amount paid out, commission refunded to
/// the employer — same policy as `resolve_dispute_sol`). The platform
/// forfeits commission on dispute, so it has no incentive to stall.
#[derive(Accounts)]
pub struct TriggerAutoReleaseSol<'info> {
    #[account(
        mut,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
        constraint = escrow.status == EscrowStatus::Disputed @ EscrowError::InvalidStatus,
    )]
    pub escrow: Account<'info, Escrow>,

    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: PDA vault
    pub escrow_vault: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized,
    )]
    /// CHECK: verified
    pub employee: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized,
    )]
    /// CHECK: verified
    pub employer: UncheckedAccount<'info>,

    /// Anyone can pay the gas to trigger this
    pub caller: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<TriggerAutoReleaseSol>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;
    let caller_key = ctx.accounts.caller.key();

    // dispute_deadline is mandatory in v2 (raise_dispute enforces > 0).
    // The check is defensive in case a v1 escrow with deadline=0 still
    // exists at upgrade time.
    require!(
        escrow.dispute_deadline != 0,
        EscrowError::AutoReleaseNotConfigured
    );
    require!(
        clock.unix_timestamp >= escrow.dispute_deadline,
        EscrowError::DisputeDeadlineNotReached
    );

    let escrow_key = escrow.key();
    let vault_seeds = &[
        Escrow::VAULT_SEED,
        escrow_key.as_ref(),
        &[escrow.vault_bump],
    ];
    let signer_seeds = &[&vault_seeds[..]];

    let remaining_worker = escrow.remaining_worker_amount();
    let remaining_commission = escrow.remaining_commission();

    if remaining_worker > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employee.to_account_info(),
                },
                signer_seeds,
            ),
            remaining_worker,
        )?;
    }

    // Drain everything else (commission refund + any dust) to the employer.
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

    escrow.released_to_employee = escrow.released_to_employee.saturating_add(remaining_worker);
    escrow.status = EscrowStatus::Resolved;
    escrow.completed_at = clock.unix_timestamp;
    escrow.dispute_resolved_by = caller_key;
    escrow.dispute_resolved_at = clock.unix_timestamp;
    escrow.employee_share_resolved = remaining_worker;
    escrow.employer_share_resolved = 0;

    emit!(DisputeResolved {
        escrow_id: escrow.escrow_id,
        resolver: caller_key,
        employee_share: remaining_worker,
        employer_share: 0,
        commission_refunded: remaining_commission,
        is_native: true,
        token_mint: escrow.token_mint,
        forced: true,
    });

    msg!("Dispute force-resolved by {:?} after deadline", caller_key);

    Ok(())
}

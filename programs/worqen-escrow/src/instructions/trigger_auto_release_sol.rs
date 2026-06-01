use crate::errors::EscrowError;
use crate::events::DisputeResolved;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts required for triggering auto-release of a SOL escrow.
#[derive(Accounts)]
pub struct TriggerAutoReleaseSol<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Disputed @ EscrowError::InvalidStatus,
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
        constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.employer
    pub employer: UncheckedAccount<'info>,

    /// Platform treasury — receives the commission (the platform keeps its fee;
    /// it is no longer refunded to the employer on auto-release).
    #[account(
        mut,
        constraint = fee_recipient.key() == escrow.fee_recipient @ EscrowError::InvalidFeeRecipient,
    )]
    /// CHECK: Verified against escrow.fee_recipient
    pub fee_recipient: UncheckedAccount<'info>,

    pub caller: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// After the dispute deadline passes, anyone can trigger release of the FULL
/// remaining worker amount to the employee (the worker delivered; the employer
/// failed to confirm or resolve in time). The platform keeps its commission
/// (routed to the treasury); only any dust is swept to the employer.
///
/// This is the permissionless safety valve so funds are never stranded by an
/// unresponsive platform. The vault is drained to actual balance.
pub fn handler(ctx: Context<TriggerAutoReleaseSol>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;

    // dispute_deadline is mandatory (raise_dispute enforces > 0). The first
    // check is defensive in case a legacy escrow with deadline=0 still exists.
    require!(
        escrow.dispute_deadline != 0,
        EscrowError::AutoReleaseNotConfigured
    );
    require!(
        clock.unix_timestamp >= escrow.dispute_deadline,
        EscrowError::DisputeDeadlineNotReached
    );

    let remaining_worker = escrow.remaining_worker_amount();
    let remaining_commission = escrow.remaining_commission();

    let escrow_key = escrow.key();
    let vault_seeds = &[
        Escrow::VAULT_SEED,
        escrow_key.as_ref(),
        &[escrow.vault_bump],
    ];
    let signer_seeds = &[&vault_seeds[..]];

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

    // Platform keeps its commission: route the remaining commission to the
    // treasury before sweeping any dust to the employer.
    let commission_to_treasury = remaining_commission.min(ctx.accounts.escrow_vault.lamports());
    if commission_to_treasury > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.fee_recipient.to_account_info(),
                },
                signer_seeds,
            ),
            commission_to_treasury,
        )?;
    }

    // Sweep any remaining dust to the employer. Vault ends at exactly 0.
    let dust_to_employer = ctx.accounts.escrow_vault.lamports();
    if dust_to_employer > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employer.to_account_info(),
                },
                signer_seeds,
            ),
            dust_to_employer,
        )?;
    }

    escrow.released_to_employee = escrow.amount;
    escrow.status = EscrowStatus::Resolved;
    escrow.completed_at = clock.unix_timestamp;
    escrow.dispute_resolved_by = ctx.accounts.caller.key();
    escrow.dispute_resolved_at = clock.unix_timestamp;
    escrow.employee_share_resolved = remaining_worker;
    escrow.employer_share_resolved = 0;

    emit!(DisputeResolved {
        escrow_id: escrow.escrow_id,
        resolver: ctx.accounts.caller.key(),
        employee_share: remaining_worker,
        employer_share: 0,
        // Platform keeps its commission on auto-release; nothing is refunded.
        commission_refunded: 0,
        is_native: true,
        token_mint: escrow.token_mint,
        forced: true,
    });

    msg!(
        "Auto-release triggered: {} to employee, {} commission to treasury",
        remaining_worker,
        commission_to_treasury
    );

    Ok(())
}

use crate::errors::EscrowError;
use crate::events::EscrowSettled;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts for an amicable (mutual) cancellation of a SOL escrow. Both
/// parties sign, so the split is jointly authorized without platform involvement.
#[derive(Accounts)]
pub struct MutualCancelSol<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Funded
            || escrow.status == EscrowStatus::PendingRelease @ EscrowError::InvalidStatus,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
        constraint = escrow.employer == employer.key() @ EscrowError::Unauthorized,
        constraint = escrow.employee == employee.key() @ EscrowError::Unauthorized,
    )]
    pub escrow: Account<'info, Escrow>,

    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: This is a PDA that holds SOL
    pub escrow_vault: UncheckedAccount<'info>,

    /// The employer — signs to authorize the split and receives their share
    /// (plus any dust). The commission is NOT refunded here.
    #[account(mut)]
    pub employer: Signer<'info>,

    /// The employee — signs to authorize the split and receives their share.
    #[account(mut)]
    pub employee: Signer<'info>,

    /// Platform treasury — receives the commission. The platform keeps its fee
    /// on a mutual cancellation; it is no longer refunded to the employer.
    #[account(
        mut,
        constraint = fee_recipient.key() == escrow.fee_recipient @ EscrowError::InvalidFeeRecipient,
    )]
    /// CHECK: Verified against escrow.fee_recipient
    pub fee_recipient: UncheckedAccount<'info>,

    /// System program
    pub system_program: Program<'info, System>,
}

/// Employer + employee mutually cancel a funded escrow, splitting the remaining
/// worker payment between themselves; the platform keeps its commission (routed
/// to the treasury).
///
/// The vault is drained to its actual balance (not the recorded amounts), so the
/// employer's share and any dust sweep to the employer — avoiding a
/// rent-exempt-min DoS and leaving the vault at exactly 0. `employee_share` is
/// the lamports sent to the employee; `employer_share = remaining_worker - employee_share`.
pub fn handler(ctx: Context<MutualCancelSol>, employee_share: u64) -> Result<()> {
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

    // Pay the worker their agreed share first.
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

    // Platform keeps its commission: route the remaining commission to the
    // treasury before refunding the employer.
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

    // Drain everything else (employer share + any dust) to the employer.
    // Vault ends at exactly 0.
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

    escrow.released_to_employee = escrow
        .released_to_employee
        .checked_add(employee_share)
        .ok_or(EscrowError::InvalidAmount)?;
    escrow.status = EscrowStatus::Resolved;
    escrow.completed_at = clock.unix_timestamp;
    escrow.employee_share_resolved = employee_share;
    escrow.employer_share_resolved = employer_share;

    emit!(EscrowSettled {
        escrow_id: escrow.escrow_id,
        employee_share,
        employer_share,
        is_native: true,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Escrow settled: {} to employee, {} to employer, {} commission to treasury",
        employee_share,
        total_to_employer,
        commission_to_treasury
    );

    Ok(())
}

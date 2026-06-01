use crate::errors::EscrowError;
use crate::events::DisputeResolved;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts required for resolving a SOL dispute.
#[derive(Accounts)]
pub struct ResolveDisputeSol<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Disputed @ EscrowError::InvalidStatus,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
        constraint = escrow.platform_authority == platform_authority.key() @ EscrowError::Unauthorized,
    )]
    pub escrow: Account<'info, Escrow>,

    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: This is a PDA that holds SOL
    pub escrow_vault: UncheckedAccount<'info>,

    /// Receives the employer share of the worker amount (plus any dust). The
    /// commission is NOT refunded here — it goes to the treasury.
    #[account(
        mut,
        constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.employer
    pub employer: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.employee
    pub employee: UncheckedAccount<'info>,

    /// Platform treasury — receives the commission. The platform keeps its fee
    /// on dispute resolution; it is no longer refunded to the employer.
    #[account(
        mut,
        constraint = fee_recipient.key() == escrow.fee_recipient @ EscrowError::InvalidFeeRecipient,
    )]
    /// CHECK: Verified against escrow.fee_recipient
    pub fee_recipient: UncheckedAccount<'info>,

    pub platform_authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// Platform splits the remaining worker payment between the parties and KEEPS
/// the commission: the full remaining commission is routed to the treasury
/// (`fee_recipient`) and is never refunded to the employer.
///
/// The vault is drained to its actual balance rather than recorded amounts, so
/// any dust deposit is swept to the employer — avoiding a rent-exempt-min DoS
/// that would otherwise brick resolve. `employee_share` is the lamports paid to
/// the employee; the commission goes to the treasury and the remainder (the
/// employer's share of the worker amount plus any dust) goes to the employer.
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

    // Platform keeps its commission on a dispute: route the full remaining
    // commission to the treasury before refunding the employer. Capped at the
    // live vault balance for safety.
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
        // Platform now keeps its commission on disputes; nothing is refunded.
        commission_refunded: 0,
        is_native: true,
        token_mint: escrow.token_mint,
        forced: false,
    });

    msg!(
        "Dispute resolved: {} to employee, {} to employer, {} commission to treasury",
        employee_share,
        total_to_employer,
        commission_to_treasury
    );

    Ok(())
}

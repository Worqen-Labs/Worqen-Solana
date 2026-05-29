use crate::errors::EscrowError;
use crate::events::EscrowToppedUp;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts required for topping up an existing native SOL escrow
#[derive(Accounts)]
pub struct DepositMoreSol<'info> {
    /// The escrow account being topped up
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Funded @ EscrowError::TopUpNotFunded,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
        constraint = escrow.employer == employer.key() @ EscrowError::Unauthorized,
    )]
    pub escrow: Account<'info, Escrow>,

    /// The vault PDA that holds the SOL
    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: This is a PDA that holds SOL, no account data
    pub escrow_vault: UncheckedAccount<'info>,

    /// The employer adding funds
    #[account(mut)]
    pub employer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// Adds worker funds to an already-funded native SOL escrow. `additional_amount`
/// is the extra worker net; commission is recomputed on the new total and only the
/// delta is charged, so the employer deposits `additional_amount + commission_delta`.
pub fn handler(ctx: Context<DepositMoreSol>, additional_amount: u64) -> Result<()> {
    require!(additional_amount > 0, EscrowError::InvalidAmount);

    let escrow = &mut ctx.accounts.escrow;

    let new_amount = escrow
        .amount
        .checked_add(additional_amount)
        .ok_or(EscrowError::InvalidAmount)?;

    // Recompute commission on the new total and charge only the delta.
    let new_commission = Escrow::calculate_commission(new_amount, escrow.commission_rate_bps);
    let commission_delta = new_commission
        .checked_sub(escrow.commission_amount)
        .ok_or(EscrowError::InvalidAmount)?;

    let total_added = additional_amount
        .checked_add(commission_delta)
        .ok_or(EscrowError::InvalidAmount)?;

    transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.employer.to_account_info(),
                to: ctx.accounts.escrow_vault.to_account_info(),
            },
        ),
        total_added,
    )?;

    escrow.amount = new_amount;
    escrow.commission_amount = new_commission;

    emit!(EscrowToppedUp {
        escrow_id: escrow.escrow_id,
        additional_amount,
        additional_commission: commission_delta,
        new_amount,
        new_commission_amount: new_commission,
        total_added,
        is_native: true,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Topped up escrow with {} lamports (worker +{}, commission +{}); new worker: {}, new commission: {}",
        total_added,
        additional_amount,
        commission_delta,
        new_amount,
        new_commission
    );

    Ok(())
}

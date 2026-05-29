use crate::errors::EscrowError;
use crate::events::EscrowFunded;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts required for depositing native SOL into escrow
#[derive(Accounts)]
pub struct DepositSol<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Created @ EscrowError::InvalidStatus,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
        constraint = escrow.employer == employer.key() @ EscrowError::Unauthorized,
    )]
    pub escrow: Account<'info, Escrow>,

    /// PDA holding the deposited SOL.
    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: This is a PDA that holds SOL, no account data
    pub escrow_vault: UncheckedAccount<'info>,

    #[account(mut)]
    pub employer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// Deposits native SOL (worker amount + commission) into the escrow vault.
pub fn handler(ctx: Context<DepositSol>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;

    let total_deposit = escrow.total_deposit()?;

    transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.employer.to_account_info(),
                to: ctx.accounts.escrow_vault.to_account_info(),
            },
        ),
        total_deposit,
    )?;

    escrow.status = EscrowStatus::Funded;
    escrow.funded_at = clock.unix_timestamp;

    emit!(EscrowFunded {
        escrow_id: escrow.escrow_id,
        amount: escrow.amount,
        commission_amount: escrow.commission_amount,
        total_deposited: total_deposit,
        is_native: true,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Deposited {} lamports into escrow vault (worker: {}, commission: {})",
        total_deposit,
        escrow.amount,
        escrow.commission_amount
    );

    Ok(())
}

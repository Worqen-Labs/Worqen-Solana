use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
use crate::state::{Escrow, EscrowStatus};
use crate::errors::EscrowError;
use crate::events::EscrowFunded;

/// Accounts required for depositing native SOL into escrow
#[derive(Accounts)]
pub struct DepositSol<'info> {
    /// The escrow account
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Created @ EscrowError::InvalidStatus,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
        constraint = escrow.employer == employer.key() @ EscrowError::Unauthorized,
    )]
    pub escrow: Account<'info, Escrow>,

    /// The vault PDA that will hold the SOL
    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: This is a PDA that holds SOL, no account data
    pub escrow_vault: UncheckedAccount<'info>,

    /// The employer depositing funds
    #[account(mut)]
    pub employer: Signer<'info>,

    /// System program for the transfer
    pub system_program: Program<'info, System>,
}

/// Deposits native SOL into the escrow vault
/// Employer deposits: worker_amount + commission_amount
pub fn handler(ctx: Context<DepositSol>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;

    // Calculate total deposit (worker amount + commission)
    let total_deposit = escrow.total_deposit();

    // Transfer SOL from employer to vault
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

    // Update escrow status
    escrow.status = EscrowStatus::Funded;
    escrow.funded_at = clock.unix_timestamp;

    // Emit event
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

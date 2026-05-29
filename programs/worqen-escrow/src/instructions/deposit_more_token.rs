use crate::errors::EscrowError;
use crate::events::EscrowToppedUp;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Accounts required for topping up an already-funded SPL token escrow.
#[derive(Accounts)]
pub struct DepositMoreToken<'info> {
    /// The escrow account being topped up
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Funded @ EscrowError::TopUpNotFunded,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
        constraint = escrow.employer == employer.key() @ EscrowError::Unauthorized,
        constraint = escrow.token_mint == token_mint.key() @ EscrowError::InvalidTokenMint,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    /// The vault token account (ATA authority = escrow PDA)
    #[account(
        mut,
        constraint = vault_token_account.owner == escrow.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// The employer adding funds
    #[account(mut)]
    pub employer: Signer<'info>,

    /// The employer's token account
    #[account(
        mut,
        constraint = employer_token_account.owner == employer.key(),
        constraint = employer_token_account.mint == token_mint.key(),
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// The token mint
    pub token_mint: Box<Account<'info, Mint>>,

    /// SPL Token program
    pub token_program: Program<'info, Token>,

    /// System program
    pub system_program: Program<'info, System>,
}

/// Tops up a funded token escrow: extends the worker net by `additional_amount`,
/// recomputes the matching commission, and moves the delta into the vault.
/// `additional_amount` is the worker's NET; commission is charged on top.
pub fn handler(ctx: Context<DepositMoreToken>, additional_amount: u64) -> Result<()> {
    require!(additional_amount > 0, EscrowError::InvalidAmount);

    let escrow = &mut ctx.accounts.escrow;

    // Commission delta charged on top of the added worker amount
    let additional_commission =
        Escrow::calculate_commission(additional_amount, escrow.commission_rate_bps);
    let total_added = additional_amount
        .checked_add(additional_commission)
        .ok_or(EscrowError::InvalidAmount)?;

    let new_amount = escrow
        .amount
        .checked_add(additional_amount)
        .ok_or(EscrowError::InvalidAmount)?;
    let new_commission_amount = escrow
        .commission_amount
        .checked_add(additional_commission)
        .ok_or(EscrowError::InvalidAmount)?;

    // Employer signs the transfer directly (no PDA authority needed on deposit)
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.employer_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.employer.to_account_info(),
            },
        ),
        total_added,
    )?;

    escrow.amount = new_amount;
    escrow.commission_amount = new_commission_amount;

    emit!(EscrowToppedUp {
        escrow_id: escrow.escrow_id,
        additional_amount,
        additional_commission,
        new_amount,
        new_commission_amount,
        total_added,
        is_native: false,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Topped up token escrow by {} (worker +{}, commission +{}); new totals worker: {}, commission: {}",
        total_added,
        additional_amount,
        additional_commission,
        new_amount,
        new_commission_amount
    );

    Ok(())
}

use crate::errors::EscrowError;
use crate::events::EscrowFunded;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Accounts required for depositing SPL tokens into escrow
#[derive(Accounts)]
pub struct DepositToken<'info> {
    /// The escrow account
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Created @ EscrowError::InvalidStatus,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
        constraint = escrow.employer == employer.key() @ EscrowError::Unauthorized,
        constraint = escrow.token_mint == token_mint.key() @ EscrowError::InvalidTokenMint,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    /// Escrow-owned vault holding the deposited tokens; created on first deposit.
    #[account(
        init_if_needed,
        payer = employer,
        associated_token::mint = token_mint,
        associated_token::authority = escrow,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// The employer depositing tokens
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

    /// Associated Token program
    pub associated_token_program: Program<'info, AssociatedToken>,

    /// System program
    pub system_program: Program<'info, System>,
}

/// Deposits SPL tokens (worker amount + commission) into the escrow vault.
pub fn handler(ctx: Context<DepositToken>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;

    let total_deposit = escrow.total_deposit()?;

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.employer_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.employer.to_account_info(),
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
        is_native: false,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Deposited {} tokens into escrow vault (worker: {}, commission: {})",
        total_deposit,
        escrow.amount,
        escrow.commission_amount
    );

    Ok(())
}

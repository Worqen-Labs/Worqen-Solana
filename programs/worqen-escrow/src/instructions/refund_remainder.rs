use crate::errors::EscrowError;
use crate::events::HourlyRemainderRefunded;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct RefundRemainder<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == hourly_period.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against hourly_period.employer
    #[account(constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized)]
    pub employer: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = signer,
        associated_token::mint = token_mint,
        associated_token::authority = employer,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    #[account(constraint = token_mint.key() == hourly_period.token_mint @ EscrowError::InvalidTokenMint)]
    pub token_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = signer.key() == hourly_period.employer
            || signer.key() == hourly_period.platform_authority @ EscrowError::Unauthorized,
    )]
    pub signer: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<RefundRemainder>) -> Result<()> {
    let vault_balance = ctx.accounts.vault_token_account.amount;
    let period = &mut ctx.accounts.hourly_period;

    let liabilities = period
        .live_liabilities()
        .ok_or(EscrowError::InsufficientFunds)?;
    let refundable = vault_balance
        .checked_sub(liabilities)
        .ok_or(EscrowError::InsufficientFunds)?;

    if refundable > 0 {
        let hire_id = period.hire_id;
        let bump = period.bump;
        let idx_le = period.period_index.to_le_bytes();
        let period_seeds = &[
            HourlyPeriod::HOURLY_SEED,
            hire_id.as_ref(),
            idx_le.as_ref(),
            &[bump],
        ];
        let signer_seeds = &[&period_seeds[..]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employer_token_account.to_account_info(),
                    authority: period.to_account_info(),
                },
                signer_seeds,
            ),
            refundable,
        )?;
    }

    period.status = if liabilities == 0 {
        HourlyStatus::Refunded
    } else {
        HourlyStatus::Settling
    };

    emit!(HourlyRemainderRefunded {
        hire_id: period.hire_id,
        period_index: period.period_index,
        refunded: refundable,
        liabilities_outstanding: liabilities,
        token_mint: period.token_mint,
    });

    Ok(())
}

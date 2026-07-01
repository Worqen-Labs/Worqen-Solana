use crate::errors::EscrowError;
use crate::events::HourlyPeriodClosed;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct ClosePeriod<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        close = employer,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == hourly_period.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = employer_token_account.owner == hourly_period.employer @ EscrowError::Unauthorized,
        constraint = employer_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against hourly_period.employer; receives rent refund
    #[account(
        mut,
        constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized,
    )]
    pub employer: UncheckedAccount<'info>,

    #[account(
        constraint = signer.key() == hourly_period.employer
            || signer.key() == hourly_period.platform_authority @ EscrowError::Unauthorized,
    )]
    pub signer: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<ClosePeriod>) -> Result<()> {
    require!(
        !ctx.accounts.hourly_period.has_live_tranche(),
        EscrowError::HourlyPeriodNotTerminal
    );

    let hire_id = ctx.accounts.hourly_period.hire_id;
    let period_index = ctx.accounts.hourly_period.period_index;
    let bump = ctx.accounts.hourly_period.bump;
    let idx_le = period_index.to_le_bytes();
    let period_seeds = &[
        HourlyPeriod::HOURLY_SEED,
        hire_id.as_ref(),
        idx_le.as_ref(),
        &[bump],
    ];
    let signer_seeds = &[&period_seeds[..]];

    let vault_balance = ctx.accounts.vault_token_account.amount;
    if vault_balance > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employer_token_account.to_account_info(),
                    authority: ctx.accounts.hourly_period.to_account_info(),
                },
                signer_seeds,
            ),
            vault_balance,
        )?;
    }

    token::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        CloseAccount {
            account: ctx.accounts.vault_token_account.to_account_info(),
            destination: ctx.accounts.employer.to_account_info(),
            authority: ctx.accounts.hourly_period.to_account_info(),
        },
        signer_seeds,
    ))?;

    emit!(HourlyPeriodClosed {
        hire_id,
        period_index,
        tokens_swept: vault_balance,
    });

    Ok(())
}

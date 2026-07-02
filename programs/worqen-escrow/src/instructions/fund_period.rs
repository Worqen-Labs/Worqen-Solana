use crate::errors::EscrowError;
use crate::events::HourlyPeriodFunded;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct FundPeriod<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

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

    #[account(
        mut,
        constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized,
    )]
    pub employer: Signer<'info>,

    #[account(
        mut,
        constraint = employer_token_account.owner == employer.key(),
        constraint = employer_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    #[account(constraint = token_mint.key() == hourly_period.token_mint @ EscrowError::InvalidTokenMint)]
    pub token_mint: Box<Account<'info, Mint>>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<FundPeriod>) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);

    let period = &mut ctx.accounts.hourly_period;
    require!(
        matches!(
            period.status,
            HourlyStatus::Open | HourlyStatus::Funded | HourlyStatus::Active
        ),
        EscrowError::InvalidStatus
    );

    let commission =
        Escrow::calculate_commission(period.weekly_cap_net, period.commission_rate_bps);
    let cap_gross = period
        .weekly_cap_net
        .checked_add(commission)
        .ok_or(EscrowError::InvalidAmount)?;
    let to_fund = cap_gross
        .checked_sub(period.funded_amount)
        .ok_or(EscrowError::PeriodFullyFunded)?;
    require!(to_fund > 0, EscrowError::PeriodFullyFunded);

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.employer_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.employer.to_account_info(),
            },
        ),
        to_fund,
    )?;

    period.funded_amount = period
        .funded_amount
        .checked_add(to_fund)
        .ok_or(EscrowError::InvalidAmount)?;
    if period.status == HourlyStatus::Open {
        period.status = HourlyStatus::Funded;
        period.funded_at = Clock::get()?.unix_timestamp;
    }

    emit!(HourlyPeriodFunded {
        hire_id: period.hire_id,
        period_index: period.period_index,
        amount_funded: to_fund,
        total_funded: period.funded_amount,
        cap_gross,
        via_delegate: false,
        token_mint: period.token_mint,
    });

    Ok(())
}

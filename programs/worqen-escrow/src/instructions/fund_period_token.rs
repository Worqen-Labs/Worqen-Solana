use crate::errors::EscrowError;
use crate::events::HourlyPeriodFunded;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct FundPeriodToken<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = !hourly_period.is_native @ EscrowError::NotTokenEscrow,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(constraint = token_mint.key() == hourly_period.token_mint @ EscrowError::InvalidTokenMint)]
    pub token_mint: Box<Account<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = employer,
        associated_token::mint = token_mint,
        associated_token::authority = hourly_period,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized,
    )]
    pub employer: Signer<'info>,

    #[account(
        mut,
        constraint = employer_token_account.owner == employer.key() @ EscrowError::Unauthorized,
        constraint = employer_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<FundPeriodToken>, max_fund_amount: u64) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);

    let now = Clock::get()?.unix_timestamp;
    let period = &mut ctx.accounts.hourly_period;
    require!(
        matches!(
            period.status,
            HourlyStatus::Open | HourlyStatus::Funded | HourlyStatus::Active
        ),
        EscrowError::InvalidStatus
    );
    require!(now < period.period_end_at, EscrowError::PeriodEnded);

    let cap_gross = period.cap_gross().ok_or(EscrowError::InvalidAmount)?;
    let to_fund = cap_gross
        .checked_sub(period.funded_gross)
        .ok_or(EscrowError::PeriodFullyFunded)?;
    require!(to_fund > 0, EscrowError::PeriodFullyFunded);
    require!(to_fund <= max_fund_amount, EscrowError::FundExceedsMax);

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

    period.funded_gross = period
        .funded_gross
        .checked_add(to_fund)
        .ok_or(EscrowError::InvalidAmount)?;
    if period.status == HourlyStatus::Open {
        period.status = HourlyStatus::Funded;
        period.funded_at = now;
    }

    emit!(HourlyPeriodFunded {
        hire_id: period.hire_id,
        period_index: period.period_index,
        amount_funded: to_fund,
        total_funded: period.funded_gross,
        cap_gross,
        is_native: false,
        token_mint: period.token_mint,
    });

    Ok(())
}

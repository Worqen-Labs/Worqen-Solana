use crate::errors::EscrowError;
use crate::events::HourlyPeriodFunded;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct PullFundPeriod<'info> {
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
        constraint = employer_token_account.owner == hourly_period.employer @ EscrowError::Unauthorized,
        constraint = employer_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: CPI signer only; PDA seeds = [DELEGATE_AUTH_SEED]
    #[account(seeds = [HourlyPeriod::DELEGATE_AUTH_SEED], bump)]
    pub delegate_authority: UncheckedAccount<'info>,

    #[account(constraint = token_mint.key() == hourly_period.token_mint @ EscrowError::InvalidTokenMint)]
    pub token_mint: Box<Account<'info, Mint>>,

    #[account(mut)]
    pub caller: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<PullFundPeriod>) -> Result<()> {
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
    let pull = cap_gross
        .checked_sub(period.funded_amount)
        .ok_or(EscrowError::PeriodFullyFunded)?;
    require!(pull > 0, EscrowError::PeriodFullyFunded);

    let delegate_seeds = &[
        HourlyPeriod::DELEGATE_AUTH_SEED,
        &[ctx.bumps.delegate_authority],
    ];
    let signer_seeds = &[&delegate_seeds[..]];

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.employer_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.delegate_authority.to_account_info(),
            },
            signer_seeds,
        ),
        pull,
    )?;

    period.funded_amount = period
        .funded_amount
        .checked_add(pull)
        .ok_or(EscrowError::InvalidAmount)?;
    if period.status == HourlyStatus::Open {
        period.status = HourlyStatus::Funded;
        period.funded_at = Clock::get()?.unix_timestamp;
    }

    emit!(HourlyPeriodFunded {
        hire_id: period.hire_id,
        period_index: period.period_index,
        amount_funded: pull,
        total_funded: period.funded_amount,
        cap_gross,
        via_delegate: true,
        token_mint: period.token_mint,
    });

    Ok(())
}

use crate::errors::EscrowError;
use crate::events::HourlyPeriodFunded;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

#[derive(Accounts)]
pub struct FundPeriodSol<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = hourly_period.is_native @ EscrowError::NotNativeEscrow,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    /// CHECK: PDA lamport vault, no data
    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, hourly_period.key().as_ref()],
        bump = hourly_period.vault_bump,
    )]
    pub escrow_vault: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized,
    )]
    pub employer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<FundPeriodSol>, max_fund_amount: u64) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);

    let now = Clock::get()?.unix_timestamp;
    let reserve = Rent::get()?.minimum_balance(0);
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

    let seed_reserve = if period.vault_rent_reserve == 0 {
        reserve
    } else {
        0
    };
    let transfer_amount = to_fund
        .checked_add(seed_reserve)
        .ok_or(EscrowError::InvalidAmount)?;

    transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.employer.to_account_info(),
                to: ctx.accounts.escrow_vault.to_account_info(),
            },
        ),
        transfer_amount,
    )?;

    period.funded_gross = period
        .funded_gross
        .checked_add(to_fund)
        .ok_or(EscrowError::InvalidAmount)?;
    if seed_reserve > 0 {
        period.vault_rent_reserve = seed_reserve;
    }
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
        is_native: true,
        token_mint: period.token_mint,
    });

    Ok(())
}

use crate::errors::EscrowError;
use crate::events::TrancheStaged;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

#[derive(Accounts)]
pub struct StageTranche<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = hourly_period.platform_authority == platform_authority.key() @ EscrowError::Unauthorized,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        constraint = vault_token_account.owner == hourly_period.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    pub platform_authority: Signer<'info>,
}

pub fn handler(ctx: Context<StageTranche>, amount: u64) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);
    require!(amount > 0, EscrowError::InvalidAmount);

    let vault_balance = ctx.accounts.vault_token_account.amount;
    let period = &mut ctx.accounts.hourly_period;

    require!(
        (period.tranche_count as usize) < HourlyPeriod::MAX_TRANCHES,
        EscrowError::TrancheLimitReached
    );
    require!(
        matches!(period.status, HourlyStatus::Funded | HourlyStatus::Active),
        EscrowError::InvalidStatus
    );

    let new_total = period
        .total_staged_net
        .checked_add(amount)
        .ok_or(EscrowError::InvalidAmount)?;
    require!(
        new_total <= period.weekly_cap_net,
        EscrowError::WeeklyCapExceeded
    );

    let cum_before = Escrow::calculate_commission(period.total_staged_net, period.commission_rate_bps);
    let cum_after = Escrow::calculate_commission(new_total, period.commission_rate_bps);
    let commission = cum_after.saturating_sub(cum_before);

    let liabilities = period
        .live_liabilities()
        .ok_or(EscrowError::VaultUnderfunded)?;
    let needed = liabilities
        .checked_add(amount)
        .ok_or(EscrowError::VaultUnderfunded)?
        .checked_add(commission)
        .ok_or(EscrowError::VaultUnderfunded)?;
    require!(needed <= vault_balance, EscrowError::VaultUnderfunded);

    let clock = Clock::get()?;
    let release_at = clock.unix_timestamp + period.review_window_secs;
    let idx = period.tranche_count as usize;
    period.tranches[idx] = Tranche {
        amount,
        commission,
        staged_at: clock.unix_timestamp,
        release_at,
        dispute_deadline: 0,
        status: TrancheStatus::Frozen,
    };
    period.tranche_count += 1;
    period.total_staged_net = new_total;
    if period.status == HourlyStatus::Funded {
        period.status = HourlyStatus::Active;
    }

    emit!(TrancheStaged {
        hire_id: period.hire_id,
        period_index: period.period_index,
        tranche_index: idx as u8,
        amount,
        commission,
        release_at,
        total_staged_net: new_total,
    });

    Ok(())
}

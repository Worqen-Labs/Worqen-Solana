use crate::errors::EscrowError;
use crate::events::HourlyCapRaised;
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct RaiseWeeklyCap<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        constraint = authority.key() == hourly_period.employer
            || authority.key() == hourly_period.platform_authority @ EscrowError::Unauthorized,
    )]
    pub authority: Signer<'info>,
}

pub fn handler(ctx: Context<RaiseWeeklyCap>, new_weekly_cap_net: u64) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);

    let period = &mut ctx.accounts.hourly_period;
    require!(
        matches!(
            period.status,
            HourlyStatus::Open | HourlyStatus::Funded | HourlyStatus::Active
        ),
        EscrowError::InvalidStatus
    );
    require!(
        new_weekly_cap_net >= period.weekly_cap_net,
        EscrowError::CapCannotDecrease
    );
    require!(
        new_weekly_cap_net >= period.total_staged_net,
        EscrowError::CapBelowStaged
    );

    let old_cap_net = period.weekly_cap_net;
    period.weekly_cap_net = new_weekly_cap_net;

    emit!(HourlyCapRaised {
        hire_id: period.hire_id,
        period_index: period.period_index,
        old_cap_net,
        new_cap_net: new_weekly_cap_net,
    });

    Ok(())
}

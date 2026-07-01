use crate::errors::EscrowError;
use crate::events::HourlyDisputeRaised;
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct RaiseHourlyDispute<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = signer.key() == hourly_period.employer
            || signer.key() == hourly_period.employee
            || signer.key() == hourly_period.platform_authority @ EscrowError::Unauthorized,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    pub signer: Signer<'info>,
}

pub fn handler(
    ctx: Context<RaiseHourlyDispute>,
    index: u8,
    dispute_deadline: i64,
    reason: Vec<u8>,
) -> Result<()> {
    require!(
        reason.len() <= Escrow::MAX_DISPUTE_REASON_LEN,
        EscrowError::DisputeReasonTooLong
    );

    let idx = index as usize;
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let period = &mut ctx.accounts.hourly_period;
    require!(
        idx < period.tranche_count as usize,
        EscrowError::InvalidTrancheIndex
    );
    let t = period.tranches[idx];
    require!(t.status == TrancheStatus::Frozen, EscrowError::TrancheNotFrozen);
    require!(now < t.release_at, EscrowError::DisputeWindowClosed);

    require!(dispute_deadline > 0, EscrowError::DisputeDeadlineRequired);
    require!(dispute_deadline > now, EscrowError::InvalidDisputeDeadline);
    require!(
        dispute_deadline - now >= Escrow::MIN_DISPUTE_DEADLINE_DURATION,
        EscrowError::DisputeWindowTooShort
    );
    require!(
        dispute_deadline - now <= Escrow::MAX_DISPUTE_DEADLINE_DURATION,
        EscrowError::DisputeDeadlineTooLong
    );

    period.tranches[idx].status = TrancheStatus::Disputed;
    period.tranches[idx].dispute_deadline = dispute_deadline;

    emit!(HourlyDisputeRaised {
        hire_id: period.hire_id,
        period_index: period.period_index,
        tranche_index: index,
        raised_by: ctx.accounts.signer.key(),
        raised_at: now,
        dispute_deadline,
    });

    Ok(())
}

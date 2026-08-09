use crate::errors::EscrowError;
use crate::events::HourlyInvoiceDisputeRaised;
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct RaiseInvoiceDispute<'info> {
    #[account(
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        mut,
        constraint = invoice.period == hourly_period.key() @ EscrowError::InvoicePeriodMismatch,
    )]
    pub invoice: Box<Account<'info, HourlyInvoice>>,

    #[account(
        constraint = signer.key() == hourly_period.employer
            || signer.key() == hourly_period.employee
            || signer.key() == hourly_period.platform_authority @ EscrowError::Unauthorized,
    )]
    pub signer: Signer<'info>,
}

pub fn handler(
    ctx: Context<RaiseInvoiceDispute>,
    dispute_deadline: i64,
    reason: Vec<u8>,
) -> Result<()> {
    require!(
        reason.len() <= Escrow::MAX_DISPUTE_REASON_LEN,
        EscrowError::DisputeReasonTooLong
    );

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let invoice = &mut ctx.accounts.invoice;
    require!(
        invoice.status == InvoiceStatus::Staged,
        EscrowError::InvoiceNotStaged
    );
    require!(now < invoice.release_at, EscrowError::DisputeWindowClosed);

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

    invoice.status = InvoiceStatus::Disputed;
    invoice.dispute_deadline = dispute_deadline;
    invoice.disputed_by = ctx.accounts.signer.key();

    emit!(HourlyInvoiceDisputeRaised {
        hire_id: ctx.accounts.hourly_period.hire_id,
        period_index: ctx.accounts.hourly_period.period_index,
        invoice_index: invoice.invoice_index,
        ref_id: invoice.ref_id,
        raised_by: ctx.accounts.signer.key(),
        raised_at: now,
        dispute_deadline,
        reason,
    });

    Ok(())
}

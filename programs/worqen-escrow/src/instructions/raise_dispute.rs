use crate::errors::EscrowError;
use crate::events::DisputeRaised;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;

/// Accounts required for raising a dispute
#[derive(Accounts)]
pub struct RaiseDispute<'info> {
    /// The escrow account
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Funded || escrow.status == EscrowStatus::PendingRelease @ EscrowError::InvalidStatus,
    )]
    pub escrow: Account<'info, Escrow>,

    /// The signer (employer or employee, with state-dependent rules)
    pub signer: Signer<'info>,
}

/// Raises a dispute, freezing funds.
///
/// In `Funded`: either party may dispute. In `PendingRelease`: only the
/// employer, since the worker is already committed by the prior confirm.
/// `dispute_deadline` is mandatory and bounded by `MAX_DISPUTE_DEADLINE_DURATION`
/// (90 days); past it, anyone may `trigger_auto_release_*` in the worker's favor.
pub fn handler(ctx: Context<RaiseDispute>, reason: Vec<u8>, dispute_deadline: i64) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let signer_key = ctx.accounts.signer.key();
    let clock = Clock::get()?;

    require!(
        signer_key == escrow.employer || signer_key == escrow.employee,
        EscrowError::Unauthorized
    );

    // In PendingRelease the worker is already committed by the prior confirm,
    // so only the employer may dispute.
    if escrow.status == EscrowStatus::PendingRelease {
        require!(
            signer_key == escrow.employer,
            EscrowError::DisputeLockedAfterConfirm
        );
    }

    require!(
        reason.len() <= Escrow::MAX_DISPUTE_REASON_LEN,
        EscrowError::DisputeReasonTooLong
    );

    // Mandatory, future, bounded deadline.
    require!(dispute_deadline > 0, EscrowError::DisputeDeadlineRequired);
    require!(
        dispute_deadline > clock.unix_timestamp,
        EscrowError::InvalidDisputeDeadline
    );
    require!(
        dispute_deadline - clock.unix_timestamp >= Escrow::MIN_DISPUTE_DEADLINE_DURATION,
        EscrowError::DisputeWindowTooShort
    );
    require!(
        dispute_deadline - clock.unix_timestamp <= Escrow::MAX_DISPUTE_DEADLINE_DURATION,
        EscrowError::DisputeDeadlineTooLong
    );

    let mut buf = [0u8; Escrow::MAX_DISPUTE_REASON_LEN];
    buf[..reason.len()].copy_from_slice(&reason);

    escrow.status = EscrowStatus::Disputed;
    escrow.dispute_reason = buf;
    escrow.dispute_raised_by = signer_key;
    escrow.dispute_raised_at = clock.unix_timestamp;
    escrow.dispute_deadline = dispute_deadline;

    emit!(DisputeRaised {
        escrow_id: escrow.escrow_id,
        raised_by: signer_key,
        raised_at: clock.unix_timestamp,
        dispute_deadline,
    });

    msg!(
        "Dispute raised by {:?} deadline={}",
        signer_key,
        dispute_deadline
    );

    Ok(())
}

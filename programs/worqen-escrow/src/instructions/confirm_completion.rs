use crate::errors::EscrowError;
use crate::events::CompletionConfirmed;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;

/// Accounts required for confirming completion
#[derive(Accounts)]
pub struct ConfirmCompletion<'info> {
    /// The escrow account
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Funded || escrow.status == EscrowStatus::PendingRelease @ EscrowError::InvalidStatus,
    )]
    pub escrow: Account<'info, Escrow>,

    /// The signer (employer or employee)
    pub signer: Signer<'info>,
}

/// Either party confirms work completion
pub fn handler(ctx: Context<ConfirmCompletion>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let signer_key = ctx.accounts.signer.key();

    require!(
        signer_key == escrow.employer || signer_key == escrow.employee,
        EscrowError::Unauthorized
    );

    if signer_key == escrow.employer {
        require!(!escrow.employer_confirmed, EscrowError::AlreadyConfirmed);
        escrow.employer_confirmed = true;
    } else {
        require!(!escrow.employee_confirmed, EscrowError::AlreadyConfirmed);
        escrow.employee_confirmed = true;
    }

    // First confirmation advances the escrow to PendingRelease.
    if escrow.status == EscrowStatus::Funded {
        escrow.status = EscrowStatus::PendingRelease;
    }

    emit!(CompletionConfirmed {
        escrow_id: escrow.escrow_id,
        confirmer: signer_key,
        employer_confirmed: escrow.employer_confirmed,
        employee_confirmed: escrow.employee_confirmed,
    });

    msg!(
        "Completion confirmed by {:?}. Employer: {}, Employee: {}",
        signer_key,
        escrow.employer_confirmed,
        escrow.employee_confirmed
    );

    Ok(())
}

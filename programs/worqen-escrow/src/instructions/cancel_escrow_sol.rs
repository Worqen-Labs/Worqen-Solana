use crate::errors::EscrowError;
use crate::events::EscrowCancelled;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts required for cancelling a SOL escrow
#[derive(Accounts)]
pub struct CancelEscrowSol<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Created || escrow.status == EscrowStatus::Funded @ EscrowError::InvalidStatus,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
    )]
    pub escrow: Account<'info, Escrow>,

    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: This is a PDA that holds SOL
    pub escrow_vault: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.employer
    pub employer: UncheckedAccount<'info>,

    /// Platform treasury — receives the commission. The platform keeps its fee
    /// even on cancellation of a funded escrow; only the worker deposit is
    /// refunded to the employer. On `Created` the vault is empty, so nothing is
    /// collected.
    #[account(
        mut,
        constraint = fee_recipient.key() == escrow.fee_recipient @ EscrowError::InvalidFeeRecipient,
    )]
    /// CHECK: Verified against escrow.fee_recipient
    pub fee_recipient: UncheckedAccount<'info>,

    /// The signer (employer or platform authority)
    pub signer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// Cancels a SOL escrow. The employer is refunded the worker deposit; the
/// platform keeps its commission (routed to the treasury) even on cancellation.
///
/// Authorization: in `Created` state, employer or platform_authority; in
/// `Funded` state, platform_authority only (the employer must dispute rather
/// than unilaterally reclaiming funds a worker may have started against).
pub fn handler(ctx: Context<CancelEscrowSol>, reason: Vec<u8>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let signer_key = ctx.accounts.signer.key();

    require!(
        signer_key == escrow.employer || signer_key == escrow.platform_authority,
        EscrowError::Unauthorized
    );

    // Once funded, only the platform can cancel. Employer must dispute.
    if escrow.status == EscrowStatus::Funded {
        require!(
            signer_key == escrow.platform_authority,
            EscrowError::EmployerCancelAfterFundedDisallowed
        );
    }

    require!(
        reason.len() <= Escrow::MAX_CANCELLATION_REASON_LEN,
        EscrowError::CancellationReasonTooLong
    );

    let clock = Clock::get()?;
    let remaining_commission = escrow.remaining_commission();

    let escrow_key = escrow.key();
    let vault_seeds = &[
        Escrow::VAULT_SEED,
        escrow_key.as_ref(),
        &[escrow.vault_bump],
    ];
    let signer_seeds = &[&vault_seeds[..]];

    // Platform keeps its commission on cancellation: route the remaining
    // commission to the treasury first. Capped at the live vault balance so a
    // `Created` (unfunded) escrow with an empty vault collects nothing.
    let commission_to_treasury = remaining_commission.min(ctx.accounts.escrow_vault.lamports());
    if commission_to_treasury > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.fee_recipient.to_account_info(),
                },
                signer_seeds,
            ),
            commission_to_treasury,
        )?;
    }

    // Refund the rest of the vault (worker deposit + any dust) to the employer.
    // Drained to actual balance so the vault ends at exactly 0.
    let refund_amount = ctx.accounts.escrow_vault.lamports();
    if refund_amount > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employer.to_account_info(),
                },
                signer_seeds,
            ),
            refund_amount,
        )?;
    }

    let mut buf = [0u8; Escrow::MAX_CANCELLATION_REASON_LEN];
    buf[..reason.len()].copy_from_slice(&reason);

    escrow.status = EscrowStatus::Cancelled;
    escrow.completed_at = clock.unix_timestamp;
    escrow.cancellation_reason = buf;
    escrow.cancelled_by = signer_key;

    emit!(EscrowCancelled {
        escrow_id: escrow.escrow_id,
        cancelled_by: signer_key,
        refunded_to: escrow.employer,
        amount_refunded: refund_amount,
        // Platform keeps its commission on cancellation; nothing is refunded.
        commission_refunded: 0,
        is_native: true,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Escrow cancelled by {:?}: {} lamports to employer, {} commission to treasury",
        signer_key,
        refund_amount,
        commission_to_treasury
    );

    Ok(())
}

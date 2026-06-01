use crate::errors::EscrowError;
use crate::events::EscrowCancelled;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

/// Accounts required for cancelling a token escrow.
///
/// **Authorization rules:**
/// - In `Created` state: employer or platform_authority.
/// - In `Funded` state: platform_authority only.
///
/// Token accounts are constrained on `mint` and `owner` to block redirection.
#[derive(Accounts)]
pub struct CancelEscrowToken<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Created || escrow.status == EscrowStatus::Funded @ EscrowError::InvalidStatus,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == escrow.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// The employer receiving the refund
    /// CHECK: Verified against escrow.employer
    #[account(constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized)]
    pub employer: UncheckedAccount<'info>,

    /// The employer's token account — constrained on owner + mint.
    #[account(
        mut,
        constraint = employer_token_account.owner == escrow.employer @ EscrowError::Unauthorized,
        constraint = employer_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Verified against escrow.fee_recipient
    #[account(constraint = fee_recipient.key() == escrow.fee_recipient @ EscrowError::InvalidFeeRecipient)]
    pub fee_recipient: UncheckedAccount<'info>,

    /// Treasury token account — receives the commission. Constrained on owner +
    /// mint so the platform fee cannot be redirected. The platform keeps its fee
    /// on cancellation of a funded escrow.
    #[account(
        mut,
        constraint = platform_token_account.owner == escrow.fee_recipient @ EscrowError::Unauthorized,
        constraint = platform_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    /// The signer (employer in Created, platform_authority in Funded)
    pub signer: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

/// Cancels a token escrow. The employer is refunded the worker deposit; the
/// platform keeps its commission (routed to the treasury) even on cancellation.
pub fn handler(ctx: Context<CancelEscrowToken>, reason: Vec<u8>) -> Result<()> {
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
    let vault_amount = ctx.accounts.vault_token_account.amount;
    // Platform keeps its commission; the employer is refunded only the rest.
    let commission_to_treasury = remaining_commission.min(vault_amount);
    let refund_amount = vault_amount.saturating_sub(commission_to_treasury);

    let escrow_id = escrow.escrow_id;
    let bump = escrow.bump;
    let escrow_seeds = &[Escrow::ESCROW_SEED, escrow_id.as_ref(), &[bump]];
    let signer_seeds = &[&escrow_seeds[..]];

    // Commission to the treasury first.
    if commission_to_treasury > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.platform_token_account.to_account_info(),
                    authority: escrow.to_account_info(),
                },
                signer_seeds,
            ),
            commission_to_treasury,
        )?;
    }

    // Refund the rest (worker deposit + any dust) to the employer.
    if refund_amount > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employer_token_account.to_account_info(),
                    authority: escrow.to_account_info(),
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
        is_native: false,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Escrow cancelled by {:?}: {} tokens to employer, {} commission to treasury",
        signer_key,
        refund_amount,
        commission_to_treasury
    );

    Ok(())
}

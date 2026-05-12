use crate::errors::EscrowError;
use crate::events::EscrowCancelled;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

/// Accounts required for cancelling a token escrow.
///
/// **v2 authorization rules:**
/// - In `Created` state: employer or platform_authority.
/// - In `Funded` state: platform_authority only.
///
/// Token accounts are constrained on `mint` and `owner` to block redirection.
#[derive(Accounts)]
pub struct CancelEscrowToken<'info> {
    /// The escrow account
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Created || escrow.status == EscrowStatus::Funded @ EscrowError::InvalidStatus,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    /// The vault token account
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

    /// The signer (employer in Created, platform_authority in Funded)
    pub signer: Signer<'info>,

    /// SPL Token program
    pub token_program: Program<'info, Token>,
}

/// Cancels token escrow, refunds full vault to employer.
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
    let vault_balance = ctx.accounts.vault_token_account.amount;

    let worker_amount = escrow.amount;
    let commission_amount = escrow.commission_amount;

    let escrow_id = escrow.escrow_id;
    let bump = escrow.bump;
    let escrow_seeds = &[Escrow::ESCROW_SEED, escrow_id.as_ref(), &[bump]];
    let signer_seeds = &[&escrow_seeds[..]];

    if vault_balance > 0 {
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
            vault_balance,
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
        amount_refunded: worker_amount,
        commission_refunded: commission_amount,
        is_native: false,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Escrow cancelled by {:?}, {} tokens refunded to employer",
        signer_key,
        vault_balance
    );

    Ok(())
}

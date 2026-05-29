use crate::errors::EscrowError;
use crate::events::EscrowReleased;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Accounts for releasing tokens from escrow. Every token account is
/// constrained on both `mint` and `owner` so a malicious authority cannot
/// redirect funds by swapping in an account they control.
#[derive(Accounts)]
pub struct ReleaseToken<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::PendingRelease || escrow.status == EscrowStatus::Funded @ EscrowError::InvalidStatus,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    /// The mint this escrow is denominated in.
    #[account(
        constraint = token_mint.key() == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub token_mint: Box<Account<'info, Mint>>,

    /// The vault token account — owner must be the escrow PDA, mint must match.
    #[account(
        mut,
        constraint = vault_token_account.owner == escrow.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Verified against escrow.employee
    #[account(constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized)]
    pub employee: UncheckedAccount<'info>,

    /// Employee's ATA, created on demand (authority pays rent) so 0-SOL
    /// employees can still be paid.
    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = token_mint,
        associated_token::authority = employee,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Verified against escrow.fee_recipient
    #[account(constraint = fee_recipient.key() == escrow.fee_recipient @ EscrowError::InvalidFeeRecipient)]
    pub fee_recipient: UncheckedAccount<'info>,

    /// Treasury token account. Constrained on owner + mint to prevent
    /// commission redirection.
    #[account(
        mut,
        constraint = platform_token_account.owner == escrow.fee_recipient @ EscrowError::Unauthorized,
        constraint = platform_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    /// Employer (confirmed), platform_authority, or employee (both confirmed).
    /// Also pays for employee ATA init when needed.
    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Releases the remaining escrowed tokens: worker gets their recorded amount,
/// the rest of the vault (commission + dust) goes to platform. Authorized for
/// employer (confirmed), platform_authority, or employee (both parties confirmed).
pub fn handler(ctx: Context<ReleaseToken>, ref_id: [u8; 32]) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let authority_key = ctx.accounts.authority.key();

    let is_employer_authorized = authority_key == escrow.employer && escrow.employer_confirmed;
    let is_platform_authorized = authority_key == escrow.platform_authority;
    let is_worker_authorized =
        authority_key == escrow.employee && escrow.employer_confirmed && escrow.employee_confirmed;

    require!(
        is_employer_authorized || is_platform_authorized || is_worker_authorized,
        EscrowError::ReleaseNotAuthorized
    );

    let clock = Clock::get()?;

    let worker_amount = escrow.remaining_worker_amount();
    require!(worker_amount > 0, EscrowError::InsufficientFunds);

    let escrow_id = escrow.escrow_id;
    let bump = escrow.bump;
    let escrow_seeds = &[Escrow::ESCROW_SEED, escrow_id.as_ref(), &[bump]];
    let signer_seeds = &[&escrow_seeds[..]];

    // Pay the worker first.
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.vault_token_account.to_account_info(),
                to: ctx.accounts.employee_token_account.to_account_info(),
                authority: escrow.to_account_info(),
            },
            signer_seeds,
        ),
        worker_amount,
    )?;

    // Drain the rest of the vault to platform (commission + any dust).
    ctx.accounts.vault_token_account.reload()?;
    let commission_amount = ctx.accounts.vault_token_account.amount;
    if commission_amount > 0 {
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
            commission_amount,
        )?;
    }

    escrow.released_to_employee = escrow.amount;
    escrow.status = EscrowStatus::Released;
    escrow.completed_at = clock.unix_timestamp;
    escrow.release_initiator = authority_key;

    emit!(EscrowReleased {
        escrow_id: escrow.escrow_id,
        recipient: escrow.employee,
        amount: worker_amount,
        commission_amount,
        commission_recipient: escrow.fee_recipient,
        is_native: false,
        token_mint: escrow.token_mint,
        initiator: authority_key,
        is_partial: false,
        remaining_worker_amount: 0,
        ref_id,
    });

    msg!(
        "Released {} tokens to employee, {} tokens to platform",
        worker_amount,
        commission_amount
    );

    Ok(())
}

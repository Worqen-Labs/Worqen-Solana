use crate::errors::EscrowError;
use crate::events::EscrowReleased;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Accounts required for releasing tokens from escrow.
///
/// Every token-account input is constrained on both `mint` (must equal
/// `escrow.token_mint`) and `owner` (must equal the corresponding wallet),
/// so a malicious authority cannot redirect funds by swapping in a token
/// account they control.
///
/// `employee_token_account` uses `init_if_needed` so a 0-SOL employee can
/// be paid: the caller (authority) covers the ~0.002 SOL of ATA rent. The
/// rent is refundable later via `close_escrow_token`.
#[derive(Accounts)]
pub struct ReleaseToken<'info> {
    /// The escrow account
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

    /// The employee pubkey
    /// CHECK: Verified against escrow.employee
    #[account(constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized)]
    pub employee: UncheckedAccount<'info>,

    /// The employee's token account. Created on demand if missing — the
    /// authority pays the ~0.002 SOL rent. This unblocks 0-SOL employees.
    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = token_mint,
        associated_token::authority = employee,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    /// The platform authority pubkey
    /// CHECK: Verified against escrow.platform_authority
    #[account(constraint = platform_authority.key() == escrow.platform_authority @ EscrowError::Unauthorized)]
    pub platform_authority: UncheckedAccount<'info>,

    /// The platform's token account. Constrained on owner + mint to prevent
    /// commission redirection.
    #[account(
        mut,
        constraint = platform_token_account.owner == escrow.platform_authority @ EscrowError::Unauthorized,
        constraint = platform_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    /// The authority — employer (with confirmation), platform_authority,
    /// or employee (when both parties confirmed). Pays for ATA init if
    /// employee_token_account doesn't exist yet.
    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Releases the remaining tokens in escrow.
///
/// Authorization (any one of):
///   - employer + `employer_confirmed`
///   - platform_authority
///   - employee, when both `employer_confirmed` and `employee_confirmed`
///     are true (worker self-release after mutual agreement)
///
/// Drains the vault token account to its actual balance to absorb any
/// dust transfers; the worker gets their recorded amount, the rest goes
/// to platform.
pub fn handler(ctx: Context<ReleaseToken>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let authority_key = ctx.accounts.authority.key();

    let is_employer_authorized = authority_key == escrow.employer && escrow.employer_confirmed;
    let is_platform_authorized = authority_key == escrow.platform_authority;
    let is_worker_authorized = authority_key == escrow.employee
        && escrow.employer_confirmed
        && escrow.employee_confirmed;

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
        commission_recipient: escrow.platform_authority,
        is_native: false,
        token_mint: escrow.token_mint,
        initiator: authority_key,
        is_partial: false,
        remaining_worker_amount: 0,
    });

    msg!(
        "Released {} tokens to employee, {} tokens to platform",
        worker_amount,
        commission_amount
    );

    Ok(())
}

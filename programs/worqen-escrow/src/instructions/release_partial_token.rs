use crate::errors::EscrowError;
use crate::events::EscrowReleased;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Accounts required for a partial token release.
///
/// All token accounts are constrained on `mint` and `owner` so a malicious
/// authority cannot redirect funds. `employee_token_account` uses
/// `init_if_needed` so a 0-SOL employee can receive a partial.
#[derive(Accounts)]
pub struct ReleasePartialToken<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Funded @ EscrowError::InvalidStatus,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        constraint = token_mint.key() == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub token_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == escrow.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Verified against escrow.employee
    #[account(constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized)]
    pub employee: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = token_mint,
        associated_token::authority = employee,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Verified against escrow.platform_authority
    #[account(constraint = platform_authority.key() == escrow.platform_authority @ EscrowError::Unauthorized)]
    pub platform_authority: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = platform_token_account.owner == escrow.platform_authority @ EscrowError::Unauthorized,
        constraint = platform_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<ReleasePartialToken>, amount: u64) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let authority_key = ctx.accounts.authority.key();

    let is_employer = authority_key == escrow.employer;
    let is_platform = authority_key == escrow.platform_authority;
    require!(is_employer || is_platform, EscrowError::ReleaseNotAuthorized);

    require!(amount > 0, EscrowError::InvalidAmount);

    let remaining = escrow.remaining_worker_amount();
    require!(amount <= remaining, EscrowError::PartialReleaseTooLarge);

    // Cumulative-delta commission math (matches the SOL partial path).
    let bps = escrow.commission_rate_bps;
    let cumulative_before = Escrow::calculate_commission(escrow.released_to_employee, bps);
    let new_released = escrow
        .released_to_employee
        .checked_add(amount)
        .ok_or(EscrowError::InvalidAmount)?;
    let cumulative_after = Escrow::calculate_commission(new_released, bps);
    let commission_slice = cumulative_after.saturating_sub(cumulative_before);

    let new_remaining = remaining
        .checked_sub(amount)
        .ok_or(EscrowError::PartialReleaseTooLarge)?;
    let is_final = new_remaining == 0;

    let escrow_id = escrow.escrow_id;
    let bump = escrow.bump;
    let escrow_seeds = &[Escrow::ESCROW_SEED, escrow_id.as_ref(), &[bump]];
    let signer_seeds = &[&escrow_seeds[..]];

    // Worker leg
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
        amount,
    )?;

    // Platform leg.
    // - Final slice: drain the entire remaining vault to platform (covers
    //   commission + any dust transfer).
    // - Non-final: pay only the recorded commission_slice.
    let platform_payment = if is_final {
        ctx.accounts.vault_token_account.reload()?;
        ctx.accounts.vault_token_account.amount
    } else {
        commission_slice
    };

    if platform_payment > 0 {
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
            platform_payment,
        )?;
    }

    escrow.released_to_employee = new_released;

    if is_final {
        let clock = Clock::get()?;
        escrow.status = EscrowStatus::Released;
        escrow.completed_at = clock.unix_timestamp;
        escrow.release_initiator = authority_key;
    }

    emit!(EscrowReleased {
        escrow_id: escrow.escrow_id,
        recipient: escrow.employee,
        amount,
        commission_amount: platform_payment,
        commission_recipient: escrow.platform_authority,
        is_native: false,
        token_mint: escrow.token_mint,
        initiator: authority_key,
        is_partial: !is_final,
        remaining_worker_amount: new_remaining,
    });

    msg!(
        "Partial token release: {} to employee ({} remaining), {} to platform",
        amount,
        new_remaining,
        platform_payment
    );

    Ok(())
}

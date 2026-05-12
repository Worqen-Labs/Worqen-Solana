use crate::errors::EscrowError;
use crate::events::DirectPaymentMade;
use crate::state::Escrow;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Direct one-shot SPL token payment with platform commission.
///
/// Token variant of `pay_with_commission_sol`. Same semantics: no state,
/// no PDAs, two atomic `token::transfer` CPIs signed by the payer.
///
/// Note on ATAs: recipient and platform token accounts MUST exist before
/// calling this instruction. If a caller doesn't know whether they exist,
/// they should prepend `create_associated_token_account_idempotent`
/// instructions in the same transaction — cheaper than an extra tx.
#[derive(Accounts)]
pub struct PayWithCommissionToken<'info> {
    /// The employer / payer.
    pub payer: Signer<'info>,

    /// Payer's token account — source of all funds being moved.
    #[account(
        mut,
        constraint = payer_token_account.owner == payer.key(),
        constraint = payer_token_account.mint == token_mint.key() @ EscrowError::InvalidTokenMint,
    )]
    pub payer_token_account: Account<'info, TokenAccount>,

    /// The worker receiving the main amount.
    /// CHECK: pubkey only; we don't read its data.
    pub recipient: UncheckedAccount<'info>,

    /// Worker's token account. Must exist and match the mint.
    #[account(
        mut,
        constraint = recipient_token_account.owner == recipient.key(),
        constraint = recipient_token_account.mint == token_mint.key() @ EscrowError::InvalidTokenMint,
    )]
    pub recipient_token_account: Account<'info, TokenAccount>,

    /// Commission recipient pubkey.
    /// CHECK: pubkey only; we don't read its data.
    pub platform_authority: UncheckedAccount<'info>,

    /// Platform's token account for receiving commission. Must exist.
    #[account(
        mut,
        constraint = platform_token_account.owner == platform_authority.key(),
        constraint = platform_token_account.mint == token_mint.key() @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Account<'info, TokenAccount>,

    /// The mint all three token accounts belong to.
    pub token_mint: Account<'info, Mint>,

    /// SPL Token program.
    pub token_program: Program<'info, Token>,
}

pub fn handler(
    ctx: Context<PayWithCommissionToken>,
    hire_id: [u8; 32],
    amount: u64,
    commission_bps: u16,
) -> Result<()> {
    require!(amount > 0, EscrowError::InvalidAmount);
    require!(
        commission_bps <= Escrow::MAX_COMMISSION_RATE_BPS,
        EscrowError::InvalidCommissionRate
    );
    require!(
        ctx.accounts.recipient.key() != ctx.accounts.payer.key(),
        EscrowError::SelfPaymentNotAllowed
    );
    require!(
        ctx.accounts.platform_authority.key() != ctx.accounts.payer.key()
            && ctx.accounts.platform_authority.key() != ctx.accounts.recipient.key(),
        EscrowError::PlatformAuthorityConflict
    );

    let commission_amount = Escrow::calculate_commission(amount, commission_bps);
    let worker_amount = amount
        .checked_sub(commission_amount)
        .ok_or(EscrowError::InvalidAmount)?;

    // Worker leg
    if worker_amount > 0 {
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer_token_account.to_account_info(),
                    to: ctx.accounts.recipient_token_account.to_account_info(),
                    authority: ctx.accounts.payer.to_account_info(),
                },
            ),
            worker_amount,
        )?;
    }

    // Commission leg
    if commission_amount > 0 {
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer_token_account.to_account_info(),
                    to: ctx.accounts.platform_token_account.to_account_info(),
                    authority: ctx.accounts.payer.to_account_info(),
                },
            ),
            commission_amount,
        )?;
    }

    let clock = Clock::get()?;
    emit!(DirectPaymentMade {
        hire_id,
        payer: ctx.accounts.payer.key(),
        recipient: ctx.accounts.recipient.key(),
        platform_authority: ctx.accounts.platform_authority.key(),
        amount,
        worker_amount,
        commission_amount,
        commission_bps,
        is_native: false,
        token_mint: ctx.accounts.token_mint.key(),
        paid_at: clock.unix_timestamp,
    });

    msg!(
        "DirectPaymentToken hire={:?} amount={} worker={} commission={} ({}bps) mint={}",
        hire_id,
        amount,
        worker_amount,
        commission_amount,
        commission_bps,
        ctx.accounts.token_mint.key()
    );

    Ok(())
}

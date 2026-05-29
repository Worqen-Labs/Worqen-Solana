use crate::errors::EscrowError;
use crate::events::DirectPaymentMade;
use crate::state::Escrow;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Direct one-shot SPL token payment with platform commission (token variant
/// of `pay_with_commission_sol`).
///
/// Recipient and platform token accounts MUST already exist; callers unsure
/// should prepend `create_associated_token_account_idempotent` in the same tx.
#[derive(Accounts)]
pub struct PayWithCommissionToken<'info> {
    /// Platform config (pause flag, fee recipient, mint allowlist).
    #[account(seeds = [crate::state::CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, crate::state::Config>>,

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

    /// Treasury that receives the commission.
    /// CHECK: pubkey only; identity is enforced via the token account owner.
    pub fee_recipient: UncheckedAccount<'info>,

    /// Treasury's token account for receiving commission. Must exist and be
    /// owned by the configured fee recipient.
    #[account(
        mut,
        constraint = platform_token_account.owner == config.fee_recipient,
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
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);
    require!(
        ctx.accounts
            .config
            .is_mint_allowed(&ctx.accounts.token_mint.key(), false),
        EscrowError::MintNotAllowed
    );
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
        ctx.accounts.fee_recipient.key() != ctx.accounts.payer.key()
            && ctx.accounts.fee_recipient.key() != ctx.accounts.recipient.key(),
        EscrowError::PlatformAuthorityConflict
    );

    // Fee-on-top: `amount` is the worker net. Commission is added on top and
    // routed to the treasury; total = amount + commission_amount.
    let commission_amount = Escrow::calculate_commission(amount, commission_bps);
    let total = amount
        .checked_add(commission_amount)
        .ok_or(EscrowError::InvalidAmount)?;

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.payer_token_account.to_account_info(),
                to: ctx.accounts.recipient_token_account.to_account_info(),
                authority: ctx.accounts.payer.to_account_info(),
            },
        ),
        amount,
    )?;

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
        fee_recipient: ctx.accounts.config.fee_recipient,
        total,
        worker_amount: amount,
        commission_amount,
        commission_bps,
        is_native: false,
        token_mint: ctx.accounts.token_mint.key(),
        paid_at: clock.unix_timestamp,
    });

    msg!(
        "DirectPaymentToken hire={:?} total={} worker={} commission={} ({}bps) mint={}",
        hire_id,
        total,
        amount,
        commission_amount,
        commission_bps,
        ctx.accounts.token_mint.key()
    );

    Ok(())
}

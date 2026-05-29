use crate::errors::EscrowError;
use crate::events::BatchPaymentMade;
use crate::state::Escrow;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Hard cap on recipients per batch. Keeps the transaction within the account
/// and compute limits — each recipient ATA is an extra remaining_account plus a
/// `token::transfer` CPI.
const MAX_BATCH_RECIPIENTS: usize = 16;

/// Stateless one-shot SPL token payment split across many recipients with a
/// single platform commission. Recipient token accounts are passed via
/// `ctx.remaining_accounts`, positionally aligned with `amounts`; all token
/// accounts (recipients and treasury) must already exist.
#[derive(Accounts)]
pub struct BatchPayWithCommissionToken<'info> {
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
    pub payer_token_account: Box<Account<'info, TokenAccount>>,

    /// Treasury's token account for receiving the (single) commission. Must
    /// exist and be owned by the configured fee recipient.
    #[account(
        mut,
        constraint = fee_token_account.owner == config.fee_recipient @ EscrowError::InvalidFeeRecipient,
        constraint = fee_token_account.mint == token_mint.key() @ EscrowError::InvalidTokenMint,
    )]
    pub fee_token_account: Box<Account<'info, TokenAccount>>,

    /// The mint all token accounts belong to.
    pub token_mint: Box<Account<'info, Mint>>,

    /// SPL Token program.
    pub token_program: Program<'info, Token>,
}

pub fn handler<'info>(
    ctx: Context<'_, '_, '_, 'info, BatchPayWithCommissionToken<'info>>,
    hire_id: [u8; 32],
    amounts: Vec<u64>,
    commission_bps: u16,
) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);
    require!(
        ctx.accounts
            .config
            .is_mint_allowed(&ctx.accounts.token_mint.key(), false),
        EscrowError::MintNotAllowed
    );
    require!(
        commission_bps <= Escrow::MAX_COMMISSION_RATE_BPS,
        EscrowError::InvalidCommissionRate
    );
    require!(!amounts.is_empty(), EscrowError::EmptyBatch);

    // Recipient ATAs come in via remaining_accounts, positionally aligned with
    // `amounts`.
    let recips = ctx.remaining_accounts;
    require!(
        recips.len() == amounts.len(),
        EscrowError::RecipientCountMismatch
    );
    require!(
        recips.len() <= MAX_BATCH_RECIPIENTS,
        EscrowError::TooManyRecipients
    );

    // Fee-on-top: every `amounts[i]` is a worker net. Commission is computed on
    // the summed worker total and routed once to the treasury.
    let mut total_worker: u64 = 0;
    for amount in amounts.iter() {
        total_worker = total_worker
            .checked_add(*amount)
            .ok_or(EscrowError::InvalidAmount)?;
    }
    require!(total_worker > 0, EscrowError::InvalidAmount);

    let commission_amount = Escrow::calculate_commission(total_worker, commission_bps);

    let token_mint_key = ctx.accounts.token_mint.key();

    // Worker legs — one transfer per recipient, payer-signed.
    for (i, recip) in recips.iter().enumerate() {
        // Validate the remaining account is a TokenAccount of the expected mint.
        // Scope the borrow so the data ref drops before the CPI re-borrows it.
        {
            let ta = anchor_spl::token::TokenAccount::try_deserialize(
                &mut &recip.try_borrow_data()?[..],
            )
            .map_err(|_| EscrowError::InvalidTokenMint)?;
            require!(ta.mint == token_mint_key, EscrowError::InvalidTokenMint);
        }

        require!(amounts[i] > 0, EscrowError::InvalidAmount);

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer_token_account.to_account_info(),
                    to: recip.clone(),
                    authority: ctx.accounts.payer.to_account_info(),
                },
            ),
            amounts[i],
        )?;
    }

    // Single commission leg on the batch total.
    if commission_amount > 0 {
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer_token_account.to_account_info(),
                    to: ctx.accounts.fee_token_account.to_account_info(),
                    authority: ctx.accounts.payer.to_account_info(),
                },
            ),
            commission_amount,
        )?;
    }

    let clock = Clock::get()?;
    emit!(BatchPaymentMade {
        hire_id,
        payer: ctx.accounts.payer.key(),
        fee_recipient: ctx.accounts.config.fee_recipient,
        recipient_count: recips.len() as u8,
        total_worker_amount: total_worker,
        commission_amount,
        commission_bps,
        is_native: false,
        token_mint: token_mint_key,
        paid_at: clock.unix_timestamp,
    });

    msg!(
        "BatchPaymentToken hire={:?} recipients={} total_worker={} commission={} ({}bps) mint={}",
        hire_id,
        recips.len(),
        total_worker,
        commission_amount,
        commission_bps,
        token_mint_key
    );

    Ok(())
}

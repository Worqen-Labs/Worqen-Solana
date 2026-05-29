use crate::errors::EscrowError;
use crate::events::BatchPaymentMade;
use crate::state::{Config, Escrow, CONFIG_SEED};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Hard cap on recipients per batch, keeping the tx within Solana's per-tx
/// account and compute budgets (one writable slot + one transfer CPI each).
pub const MAX_BATCH_RECIPIENTS: usize = 16;

/// Direct, non-escrow SOL fan-out: pays each recipient their net `amounts[i]`
/// plus one commission charged on top of the total. No state persisted.
/// Recipients are passed (writable) via `ctx.remaining_accounts`, positionally
/// aligned with `amounts`; no recipient may equal the payer (prevents
/// self-paying).
#[derive(Accounts)]
pub struct BatchPayWithCommissionSol<'info> {
    /// The employer / payer. Funds come out of this wallet.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// Platform config PDA. Gates the pay path (pause + fee recipient).
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    /// Commission recipient, constrained to equal `config.fee_recipient`.
    /// CHECK: Arbitrary SOL recipient; we only transfer to it.
    #[account(
        mut,
        constraint = fee_recipient.key() == config.fee_recipient @ EscrowError::InvalidFeeRecipient
    )]
    pub fee_recipient: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// Pays each recipient `amounts[i]` in full and charges `commission_bps` once
/// on the total. `hire_id` is an opaque tag for off-chain indexers.
pub fn handler<'c: 'info, 'info>(
    ctx: Context<'_, '_, 'c, 'info, BatchPayWithCommissionSol<'info>>,
    hire_id: [u8; 32],
    amounts: Vec<u64>,
    commission_bps: u16,
) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);
    require!(
        commission_bps <= Escrow::MAX_COMMISSION_RATE_BPS,
        EscrowError::InvalidCommissionRate
    );
    require!(!amounts.is_empty(), EscrowError::EmptyBatch);

    let recips = ctx.remaining_accounts;
    require!(
        recips.len() == amounts.len(),
        EscrowError::RecipientCountMismatch
    );
    require!(
        amounts.len() <= MAX_BATCH_RECIPIENTS,
        EscrowError::TooManyRecipients
    );

    let total_worker = amounts
        .iter()
        .try_fold(0u64, |acc, &a| acc.checked_add(a))
        .ok_or(EscrowError::InvalidAmount)?;
    require!(total_worker > 0, EscrowError::InvalidAmount);

    let commission_amount = Escrow::calculate_commission(total_worker, commission_bps);

    let payer_key = ctx.accounts.payer.key();

    for (i, recip) in recips.iter().enumerate() {
        require!(recip.key() != payer_key, EscrowError::SelfPaymentNotAllowed);
        require!(amounts[i] > 0, EscrowError::InvalidAmount);

        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: recip.clone(),
                },
            ),
            amounts[i],
        )?;
    }

    // Skip the commission transfer when it rounds to zero to save fees.
    if commission_amount > 0 {
        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.fee_recipient.to_account_info(),
                },
            ),
            commission_amount,
        )?;
    }

    let clock = Clock::get()?;
    emit!(BatchPaymentMade {
        hire_id,
        payer: payer_key,
        fee_recipient: ctx.accounts.fee_recipient.key(),
        recipient_count: amounts.len() as u8,
        total_worker_amount: total_worker,
        commission_amount,
        commission_bps,
        is_native: true,
        token_mint: Pubkey::default(),
        paid_at: clock.unix_timestamp,
    });

    msg!(
        "BatchPaymentSol hire={:?} recipients={} total_worker={} commission={} ({}bps)",
        hire_id,
        amounts.len(),
        total_worker,
        commission_amount,
        commission_bps
    );

    Ok(())
}

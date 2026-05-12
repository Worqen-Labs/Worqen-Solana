use crate::errors::EscrowError;
use crate::events::DirectPaymentMade;
use crate::state::Escrow;
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Direct one-shot SOL payment with platform commission.
///
/// This is the non-escrow, no-lock pay path. No PDAs created, no state
/// persisted; the program just atomically splits `amount` into a worker
/// share (sent to `recipient`) and a commission share (sent to
/// `platform_authority`). Emits `DirectPaymentMade` so off-chain indexers
/// can attribute the tx to a hire row.
///
/// Backend policy decides `commission_bps` per call — a regular hire
/// might pass 150 or 200; a tip might pass 50. The contract just enforces
/// the cap and performs the split.
///
/// Usage: signed by the employer. Recipient must differ from payer
/// (prevents self-paying the commission to yourself).
#[derive(Accounts)]
pub struct PayWithCommissionSol<'info> {
    /// The employer / payer. Funds come out of this wallet.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The worker receiving the main amount.
    /// CHECK: Arbitrary SOL recipient; we only transfer to it.
    #[account(mut)]
    pub recipient: UncheckedAccount<'info>,

    /// Platform commission recipient. Typically the platform_authority used
    /// by the escrow flow — both paths funnel commission to the same key.
    /// CHECK: Arbitrary SOL recipient; we only transfer to it.
    #[account(mut)]
    pub platform_authority: UncheckedAccount<'info>,

    /// System program for the two transfer CPIs.
    pub system_program: Program<'info, System>,
}

/// # Arguments
/// * `hire_id` - 32-byte identifier used by off-chain indexers to attribute
///   this payment to a Worqen hire. The contract itself never interprets it.
/// * `amount` - total lamports the payer is sending, including commission.
/// * `commission_bps` - basis points to route to `platform_authority`.
///   Capped at `Escrow::MAX_COMMISSION_RATE_BPS` (1000 = 10%).
pub fn handler(
    ctx: Context<PayWithCommissionSol>,
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
        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.recipient.to_account_info(),
                },
            ),
            worker_amount,
        )?;
    }

    // Commission leg — skip if 0 to save fees
    if commission_amount > 0 {
        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.platform_authority.to_account_info(),
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
        is_native: true,
        token_mint: Pubkey::default(),
        paid_at: clock.unix_timestamp,
    });

    msg!(
        "DirectPaymentSol hire={:?} amount={} worker={} commission={} ({}bps)",
        hire_id,
        amount,
        worker_amount,
        commission_amount,
        commission_bps
    );

    Ok(())
}

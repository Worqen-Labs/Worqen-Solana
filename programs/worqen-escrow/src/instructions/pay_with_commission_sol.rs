use crate::errors::EscrowError;
use crate::events::DirectPaymentMade;
use crate::state::{Config, Escrow, CONFIG_SEED};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Direct one-shot SOL payment with platform commission (non-escrow path).
///
/// Atomically pays the worker (`recipient`) the full `amount` and a commission
/// on top to the treasury (`fee_recipient`); persists no state and emits
/// `DirectPaymentMade` for off-chain attribution. Signed by the employer.
#[derive(Accounts)]
pub struct PayWithCommissionSol<'info> {
    /// The employer / payer. Funds come out of this wallet.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: Arbitrary SOL recipient; we only transfer to it.
    #[account(mut)]
    pub recipient: UncheckedAccount<'info>,

    /// Platform commission recipient. Must equal the treasury configured in
    /// `config.fee_recipient`; both pay paths funnel commission to that key.
    /// CHECK: Arbitrary SOL recipient; we only transfer to it.
    #[account(mut, constraint = fee_recipient.key() == config.fee_recipient @ EscrowError::InvalidFeeRecipient)]
    pub fee_recipient: UncheckedAccount<'info>,

    /// Platform config PDA. Gates the pay path (pause + fee recipient).
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    /// System program for the two transfer CPIs.
    pub system_program: Program<'info, System>,
}

/// Pays the worker `amount` (their net) plus `commission_bps` charged on top to
/// the treasury. `hire_id` is opaque to the contract; commission is capped at
/// `Escrow::MAX_COMMISSION_RATE_BPS`.
pub fn handler(
    ctx: Context<PayWithCommissionSol>,
    hire_id: [u8; 32],
    amount: u64,
    commission_bps: u16,
) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);
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

    // Fee-on-top: `amount` is the worker's net; commission is charged on top.
    let commission_amount = Escrow::calculate_commission(amount, commission_bps);
    let total = amount
        .checked_add(commission_amount)
        .ok_or(EscrowError::InvalidAmount)?;

    transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.payer.to_account_info(),
                to: ctx.accounts.recipient.to_account_info(),
            },
        ),
        amount,
    )?;

    // Skip the commission CPI when zero to save fees.
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
    emit!(DirectPaymentMade {
        hire_id,
        payer: ctx.accounts.payer.key(),
        recipient: ctx.accounts.recipient.key(),
        fee_recipient: ctx.accounts.fee_recipient.key(),
        total,
        worker_amount: amount,
        commission_amount,
        commission_bps,
        is_native: true,
        token_mint: Pubkey::default(),
        paid_at: clock.unix_timestamp,
    });

    msg!(
        "DirectPaymentSol hire={:?} total={} worker={} commission={} ({}bps)",
        hire_id,
        total,
        amount,
        commission_amount,
        commission_bps
    );

    Ok(())
}

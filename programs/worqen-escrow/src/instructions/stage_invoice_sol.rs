use crate::errors::EscrowError;
use crate::events::HourlyInvoiceStaged;
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct StageInvoiceSol<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = hourly_period.is_native @ EscrowError::NotNativeEscrow,
        constraint = hourly_period.platform_authority == platform_authority.key() @ EscrowError::Unauthorized,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        init,
        payer = platform_authority,
        space = HourlyInvoice::SPACE,
        seeds = [
            HourlyInvoice::INVOICE_SEED,
            hourly_period.key().as_ref(),
            &hourly_period.invoice_count.to_le_bytes(),
        ],
        bump
    )]
    pub invoice: Box<Account<'info, HourlyInvoice>>,

    /// CHECK: PDA lamport vault, no data
    #[account(
        seeds = [Escrow::VAULT_SEED, hourly_period.key().as_ref()],
        bump = hourly_period.vault_bump,
    )]
    pub escrow_vault: UncheckedAccount<'info>,

    #[account(mut)]
    pub platform_authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<StageInvoiceSol>, amount_net: u64, ref_id: [u8; 32]) -> Result<()> {
    let vault_balance = ctx
        .accounts
        .escrow_vault
        .lamports()
        .saturating_sub(ctx.accounts.hourly_period.vault_rent_reserve);
    stage_common(
        &ctx.accounts.config,
        &mut ctx.accounts.hourly_period,
        &mut ctx.accounts.invoice,
        ctx.bumps.invoice,
        ctx.accounts.platform_authority.key(),
        vault_balance,
        amount_net,
        ref_id,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stage_common(
    config: &Config,
    period: &mut Account<'_, HourlyPeriod>,
    invoice: &mut Account<'_, HourlyInvoice>,
    invoice_bump: u8,
    rent_payer: Pubkey,
    vault_balance: u64,
    amount_net: u64,
    ref_id: [u8; 32],
) -> Result<()> {
    require!(!config.paused, EscrowError::ProgramPaused);
    require!(amount_net > 0, EscrowError::InvalidAmount);
    require!(
        matches!(period.status, HourlyStatus::Funded | HourlyStatus::Active),
        EscrowError::InvalidStatus
    );

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    require!(now >= period.period_start_at, EscrowError::PeriodNotStarted);
    require!(now < period.period_end_at, EscrowError::PeriodEnded);

    let new_total = period
        .total_staged_net
        .checked_add(amount_net)
        .ok_or(EscrowError::InvalidAmount)?;
    require!(
        new_total <= period.weekly_cap_net,
        EscrowError::WeeklyCapExceeded
    );

    let commission = period
        .marginal_commission(amount_net)
        .ok_or(EscrowError::InvalidAmount)?;
    let outstanding = period
        .outstanding_total()
        .ok_or(EscrowError::VaultUnderfunded)?;
    let needed = outstanding
        .checked_add(amount_net)
        .ok_or(EscrowError::VaultUnderfunded)?
        .checked_add(commission)
        .ok_or(EscrowError::VaultUnderfunded)?;
    require!(needed <= vault_balance, EscrowError::VaultUnderfunded);

    let invoice_index = period.invoice_count;
    let release_at = now
        .checked_add(period.review_window_secs)
        .ok_or(EscrowError::InvalidAmount)?;

    invoice.version = HOURLY_INVOICE_VERSION;
    invoice.period = period.key();
    invoice.invoice_index = invoice_index;
    invoice.ref_id = ref_id;
    invoice.amount_net = amount_net;
    invoice.commission = commission;
    invoice.staged_at = now;
    invoice.release_at = release_at;
    invoice.dispute_deadline = 0;
    invoice.disputed_by = Pubkey::default();
    invoice.status = InvoiceStatus::Staged;
    invoice.rent_payer = rent_payer;
    invoice.bump = invoice_bump;
    invoice.reserved = [0u8; 32];

    period
        .register_staged(amount_net, commission)
        .ok_or(EscrowError::InvoiceIndexOverflow)?;
    if period.status == HourlyStatus::Funded {
        period.status = HourlyStatus::Active;
    }

    emit!(HourlyInvoiceStaged {
        hire_id: period.hire_id,
        period_index: period.period_index,
        invoice_index,
        ref_id,
        amount_net,
        commission,
        release_at,
        total_staged_net: period.total_staged_net,
        is_native: period.is_native,
        token_mint: period.token_mint,
    });

    Ok(())
}

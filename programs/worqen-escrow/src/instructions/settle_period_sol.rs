use crate::errors::EscrowError;
use crate::events::HourlyPeriodSettled;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

#[derive(Accounts)]
pub struct SettlePeriodSol<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = hourly_period.is_native @ EscrowError::NotNativeEscrow,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    /// CHECK: PDA lamport vault, no data
    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, hourly_period.key().as_ref()],
        bump = hourly_period.vault_bump,
    )]
    pub escrow_vault: UncheckedAccount<'info>,

    /// CHECK: matched against hourly_period.employer; receives the refund
    #[account(
        mut,
        constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized,
    )]
    pub employer: UncheckedAccount<'info>,

    pub caller: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<SettlePeriodSol>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let vault_balance = ctx.accounts.escrow_vault.lamports();
    let period = &ctx.accounts.hourly_period;
    require!(
        period.status != HourlyStatus::Settled,
        EscrowError::PeriodAlreadySettled
    );
    require!(now >= period.period_end_at, EscrowError::PeriodNotEnded);

    let outstanding = period
        .outstanding_total()
        .ok_or(EscrowError::InvalidAmount)?;
    let reserved = outstanding
        .checked_add(period.vault_rent_reserve)
        .ok_or(EscrowError::InvalidAmount)?;
    let refundable = vault_balance
        .checked_sub(reserved)
        .ok_or(EscrowError::InsufficientFunds)?;

    if refundable > 0 {
        let period_key = period.key();
        let vault_bump = period.vault_bump;
        let vault_seeds = &[Escrow::VAULT_SEED, period_key.as_ref(), &[vault_bump]];
        let signer_seeds = &[&vault_seeds[..]];
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employer.to_account_info(),
                },
                signer_seeds,
            ),
            refundable,
        )?;
    }

    let period = &mut ctx.accounts.hourly_period;
    period.status = HourlyStatus::Settled;
    period.settled_at = now;
    period.refunded_amount = period
        .refunded_amount
        .checked_add(refundable)
        .ok_or(EscrowError::InvalidAmount)?;

    emit!(HourlyPeriodSettled {
        hire_id: period.hire_id,
        period_index: period.period_index,
        refunded: refundable,
        outstanding_net: period.outstanding_net,
        outstanding_commission: period.outstanding_commission,
        is_native: true,
        token_mint: period.token_mint,
    });

    Ok(())
}

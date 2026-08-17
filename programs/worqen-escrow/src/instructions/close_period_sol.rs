use crate::errors::EscrowError;
use crate::events::HourlyPeriodClosed;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

#[derive(Accounts)]
pub struct ClosePeriodSol<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        close = rent_payer,
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

    /// CHECK: matched against hourly_period.employer; receives vault residue
    #[account(
        mut,
        constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized,
    )]
    pub employer: UncheckedAccount<'info>,

    /// CHECK: matched against hourly_period.rent_payer; receives period rent
    #[account(
        mut,
        constraint = rent_payer.key() == hourly_period.rent_payer @ EscrowError::Unauthorized,
    )]
    pub rent_payer: UncheckedAccount<'info>,

    pub caller: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<ClosePeriodSol>) -> Result<()> {
    let period = &ctx.accounts.hourly_period;
    require!(
        period.status == HourlyStatus::Settled,
        EscrowError::PeriodNotSettled
    );
    require!(
        period.live_invoices == 0,
        EscrowError::LiveInvoicesOutstanding
    );

    let swept = ctx.accounts.escrow_vault.lamports();
    if swept > 0 {
        let period_key = period.key();
        let vault_bump = period.vault_bump;
        let vault_seeds = &[Escrow::VAULT_SEED, period_key.as_ref(), &[vault_bump]];
        let signer_seeds = &[&vault_seeds[..]];
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.key(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employer.to_account_info(),
                },
                signer_seeds,
            ),
            swept,
        )?;
    }

    emit!(HourlyPeriodClosed {
        hire_id: period.hire_id,
        period_index: period.period_index,
        swept,
        is_native: true,
        token_mint: period.token_mint,
    });

    Ok(())
}

use crate::errors::EscrowError;
use crate::events::HourlyPeriodSettled;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct SettlePeriodToken<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = !hourly_period.is_native @ EscrowError::NotTokenEscrow,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(constraint = token_mint.key() == hourly_period.token_mint @ EscrowError::InvalidTokenMint)]
    pub token_mint: Box<Account<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = caller,
        associated_token::mint = token_mint,
        associated_token::authority = hourly_period,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against hourly_period.employer
    #[account(constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized)]
    pub employer: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = caller,
        associated_token::mint = token_mint,
        associated_token::authority = employer,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub caller: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<SettlePeriodToken>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let vault_balance = ctx.accounts.vault_token_account.amount;
    let period = &ctx.accounts.hourly_period;
    require!(
        period.status != HourlyStatus::Settled,
        EscrowError::PeriodAlreadySettled
    );
    require!(now >= period.period_end_at, EscrowError::PeriodNotEnded);

    let outstanding = period
        .outstanding_total()
        .ok_or(EscrowError::InvalidAmount)?;
    let refundable = vault_balance
        .checked_sub(outstanding)
        .ok_or(EscrowError::InsufficientFunds)?;

    if refundable > 0 {
        let hire_id = period.hire_id;
        let bump = period.bump;
        let idx_le = period.period_index.to_le_bytes();
        let period_seeds = &[
            HourlyPeriod::HOURLY_SEED,
            hire_id.as_ref(),
            idx_le.as_ref(),
            &[bump],
        ];
        let signer_seeds = &[&period_seeds[..]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employer_token_account.to_account_info(),
                    authority: ctx.accounts.hourly_period.to_account_info(),
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
        is_native: false,
        token_mint: period.token_mint,
    });

    Ok(())
}

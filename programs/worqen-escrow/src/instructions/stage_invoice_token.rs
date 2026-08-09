use crate::errors::EscrowError;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use super::stage_invoice_sol::stage_common;

#[derive(Accounts)]
pub struct StageInvoiceToken<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = !hourly_period.is_native @ EscrowError::NotTokenEscrow,
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

    #[account(
        constraint = vault_token_account.owner == hourly_period.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub platform_authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<StageInvoiceToken>, amount_net: u64, ref_id: [u8; 32]) -> Result<()> {
    let vault_balance = ctx.accounts.vault_token_account.amount;
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

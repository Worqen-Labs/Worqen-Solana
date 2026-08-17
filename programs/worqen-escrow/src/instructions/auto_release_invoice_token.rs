use crate::errors::EscrowError;
use crate::events::HourlyInvoiceResolved;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct AutoReleaseInvoiceToken<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = !hourly_period.is_native @ EscrowError::NotTokenEscrow,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        mut,
        close = rent_payer,
        constraint = invoice.period == hourly_period.key() @ EscrowError::InvoicePeriodMismatch,
    )]
    pub invoice: Box<Account<'info, HourlyInvoice>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == hourly_period.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = employee_token_account.owner == hourly_period.employee @ EscrowError::Unauthorized,
        constraint = employee_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = platform_token_account.owner == hourly_period.fee_recipient @ EscrowError::Unauthorized,
        constraint = platform_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against invoice.rent_payer; receives invoice rent
    #[account(
        mut,
        constraint = rent_payer.key() == invoice.rent_payer @ EscrowError::Unauthorized,
    )]
    pub rent_payer: UncheckedAccount<'info>,

    pub caller: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<AutoReleaseInvoiceToken>) -> Result<()> {
    let invoice = &ctx.accounts.invoice;
    require!(
        invoice.status == InvoiceStatus::Disputed,
        EscrowError::InvoiceNotDisputed
    );
    let clock = Clock::get()?;
    require!(
        invoice.dispute_deadline != 0 && clock.unix_timestamp >= invoice.dispute_deadline,
        EscrowError::DisputeDeadlineNotReached
    );

    let amount_net = invoice.amount_net;
    let commission = invoice.commission;
    let invoice_index = invoice.invoice_index;
    let ref_id = invoice.ref_id;

    let hire_id = ctx.accounts.hourly_period.hire_id;
    let bump = ctx.accounts.hourly_period.bump;
    let idx_le = ctx.accounts.hourly_period.period_index.to_le_bytes();
    let period_seeds = &[
        HourlyPeriod::HOURLY_SEED,
        hire_id.as_ref(),
        idx_le.as_ref(),
        &[bump],
    ];
    let signer_seeds = &[&period_seeds[..]];

    if amount_net > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employee_token_account.to_account_info(),
                    authority: ctx.accounts.hourly_period.to_account_info(),
                },
                signer_seeds,
            ),
            amount_net,
        )?;
    }
    if commission > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.platform_token_account.to_account_info(),
                    authority: ctx.accounts.hourly_period.to_account_info(),
                },
                signer_seeds,
            ),
            commission,
        )?;
    }

    let period = &mut ctx.accounts.hourly_period;
    period
        .register_settled_invoice(amount_net, commission, amount_net)
        .ok_or(EscrowError::InvalidAmount)?;

    emit!(HourlyInvoiceResolved {
        hire_id: period.hire_id,
        period_index: period.period_index,
        invoice_index,
        ref_id,
        employee_share: amount_net,
        employer_share: 0,
        commission_to_treasury: commission,
        commission_refunded: 0,
        forced: true,
        is_native: false,
        token_mint: period.token_mint,
    });

    Ok(())
}

use crate::errors::EscrowError;
use crate::events::HourlyInvoiceFinalized;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

#[derive(Accounts)]
pub struct FinalizeInvoiceSol<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = hourly_period.is_native @ EscrowError::NotNativeEscrow,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        mut,
        close = rent_payer,
        constraint = invoice.period == hourly_period.key() @ EscrowError::InvoicePeriodMismatch,
    )]
    pub invoice: Box<Account<'info, HourlyInvoice>>,

    /// CHECK: PDA lamport vault, no data
    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, hourly_period.key().as_ref()],
        bump = hourly_period.vault_bump,
    )]
    pub escrow_vault: UncheckedAccount<'info>,

    /// CHECK: matched against hourly_period.employee
    #[account(
        mut,
        constraint = employee.key() == hourly_period.employee @ EscrowError::Unauthorized,
    )]
    pub employee: UncheckedAccount<'info>,

    /// CHECK: matched against hourly_period.fee_recipient
    #[account(
        mut,
        constraint = fee_recipient.key() == hourly_period.fee_recipient @ EscrowError::InvalidFeeRecipient,
    )]
    pub fee_recipient: UncheckedAccount<'info>,

    /// CHECK: matched against invoice.rent_payer; receives invoice rent
    #[account(
        mut,
        constraint = rent_payer.key() == invoice.rent_payer @ EscrowError::Unauthorized,
    )]
    pub rent_payer: UncheckedAccount<'info>,

    pub caller: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<FinalizeInvoiceSol>) -> Result<()> {
    let invoice = &ctx.accounts.invoice;
    require!(
        invoice.status == InvoiceStatus::Staged,
        EscrowError::InvoiceNotStaged
    );
    let clock = Clock::get()?;
    require!(
        clock.unix_timestamp >= invoice.release_at,
        EscrowError::InvoiceWindowNotElapsed
    );

    let amount_net = invoice.amount_net;
    let commission = invoice.commission;
    let invoice_index = invoice.invoice_index;
    let ref_id = invoice.ref_id;

    let period_key = ctx.accounts.hourly_period.key();
    let vault_bump = ctx.accounts.hourly_period.vault_bump;
    let vault_seeds = &[Escrow::VAULT_SEED, period_key.as_ref(), &[vault_bump]];
    let signer_seeds = &[&vault_seeds[..]];

    if amount_net > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.key(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employee.to_account_info(),
                },
                signer_seeds,
            ),
            amount_net,
        )?;
    }
    if commission > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.key(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.fee_recipient.to_account_info(),
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

    emit!(HourlyInvoiceFinalized {
        hire_id: period.hire_id,
        period_index: period.period_index,
        invoice_index,
        ref_id,
        recipient: period.employee,
        amount_net,
        commission,
        commission_recipient: period.fee_recipient,
        forced: false,
        is_native: true,
        token_mint: period.token_mint,
    });

    Ok(())
}

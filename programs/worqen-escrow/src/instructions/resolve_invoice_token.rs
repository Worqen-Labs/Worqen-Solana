use crate::errors::EscrowError;
use crate::events::HourlyInvoiceResolved;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct ResolveInvoiceToken<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = !hourly_period.is_native @ EscrowError::NotTokenEscrow,
        constraint = hourly_period.platform_authority == platform_authority.key() @ EscrowError::Unauthorized,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        mut,
        close = rent_payer,
        constraint = invoice.period == hourly_period.key() @ EscrowError::InvoicePeriodMismatch,
    )]
    pub invoice: Box<Account<'info, HourlyInvoice>>,

    #[account(constraint = token_mint.key() == hourly_period.token_mint @ EscrowError::InvalidTokenMint)]
    pub token_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == hourly_period.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against hourly_period.employer
    #[account(constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized)]
    pub employer: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = platform_authority,
        associated_token::mint = token_mint,
        associated_token::authority = employer,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against hourly_period.employee
    #[account(constraint = employee.key() == hourly_period.employee @ EscrowError::Unauthorized)]
    pub employee: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = platform_authority,
        associated_token::mint = token_mint,
        associated_token::authority = employee,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against hourly_period.fee_recipient
    #[account(constraint = fee_recipient.key() == hourly_period.fee_recipient @ EscrowError::InvalidFeeRecipient)]
    pub fee_recipient: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = platform_authority,
        associated_token::mint = token_mint,
        associated_token::authority = fee_recipient,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against invoice.rent_payer; receives invoice rent
    #[account(
        mut,
        constraint = rent_payer.key() == invoice.rent_payer @ EscrowError::Unauthorized,
    )]
    pub rent_payer: UncheckedAccount<'info>,

    #[account(mut)]
    pub platform_authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<ResolveInvoiceToken>, employee_share: u64) -> Result<()> {
    let invoice = &ctx.accounts.invoice;
    require!(
        invoice.status == InvoiceStatus::Disputed,
        EscrowError::InvoiceNotDisputed
    );
    require!(
        employee_share <= invoice.amount_net,
        EscrowError::EmployeeShareExceedsInvoice
    );

    let amount_net = invoice.amount_net;
    let commission = invoice.commission;
    let invoice_index = invoice.invoice_index;
    let ref_id = invoice.ref_id;

    let commission_on_share = Escrow::calculate_commission(
        employee_share,
        ctx.accounts.hourly_period.commission_rate_bps,
    );
    let treasury_leg = commission_on_share.min(commission);
    let employer_worker_share = amount_net
        .checked_sub(employee_share)
        .ok_or(EscrowError::InvalidAmount)?;
    let employer_leg = employer_worker_share
        .checked_add(
            commission
                .checked_sub(treasury_leg)
                .ok_or(EscrowError::InvalidAmount)?,
        )
        .ok_or(EscrowError::InvalidAmount)?;

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

    if employee_share > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employee_token_account.to_account_info(),
                    authority: ctx.accounts.hourly_period.to_account_info(),
                },
                signer_seeds,
            ),
            employee_share,
        )?;
    }
    if treasury_leg > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.platform_token_account.to_account_info(),
                    authority: ctx.accounts.hourly_period.to_account_info(),
                },
                signer_seeds,
            ),
            treasury_leg,
        )?;
    }
    if employer_leg > 0 {
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
            employer_leg,
        )?;
    }

    let period = &mut ctx.accounts.hourly_period;
    period
        .register_settled_invoice(amount_net, commission, employee_share)
        .ok_or(EscrowError::InvalidAmount)?;

    emit!(HourlyInvoiceResolved {
        hire_id: period.hire_id,
        period_index: period.period_index,
        invoice_index,
        ref_id,
        employee_share,
        employer_share: employer_worker_share,
        commission_to_treasury: treasury_leg,
        commission_refunded: commission - treasury_leg,
        forced: false,
        is_native: false,
        token_mint: period.token_mint,
    });

    Ok(())
}

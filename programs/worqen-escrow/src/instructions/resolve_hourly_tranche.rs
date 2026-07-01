use crate::errors::EscrowError;
use crate::events::HourlyTrancheResolved;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct ResolveHourlyTranche<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        constraint = hourly_period.platform_authority == platform_authority.key() @ EscrowError::Unauthorized,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

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
        mut,
        constraint = platform_token_account.owner == hourly_period.fee_recipient @ EscrowError::Unauthorized,
        constraint = platform_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub platform_authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<ResolveHourlyTranche>, index: u8, employee_share: u64) -> Result<()> {
    let idx = index as usize;
    let period = &mut ctx.accounts.hourly_period;
    require!(
        idx < period.tranche_count as usize,
        EscrowError::InvalidTrancheIndex
    );
    let t = period.tranches[idx];
    require!(
        t.status == TrancheStatus::Disputed,
        EscrowError::TrancheNotDisputed
    );
    require!(
        employee_share <= t.amount,
        EscrowError::HourlyEmployeeShareExceedsTranche
    );

    let commission_on_share = Escrow::calculate_commission(employee_share, period.commission_rate_bps);
    let treasury_leg = commission_on_share.min(t.commission);
    let employer_worker_share = t
        .amount
        .checked_sub(employee_share)
        .ok_or(EscrowError::InvalidAmount)?;
    let employer_leg = employer_worker_share
        .checked_add(
            t.commission
                .checked_sub(treasury_leg)
                .ok_or(EscrowError::InvalidAmount)?,
        )
        .ok_or(EscrowError::InvalidAmount)?;

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

    if employee_share > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employee_token_account.to_account_info(),
                    authority: period.to_account_info(),
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
                    authority: period.to_account_info(),
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
                    authority: period.to_account_info(),
                },
                signer_seeds,
            ),
            employer_leg,
        )?;
    }

    period.tranches[idx].status = TrancheStatus::Resolved;
    period.released_net = period
        .released_net
        .checked_add(employee_share)
        .ok_or(EscrowError::InvalidAmount)?;

    emit!(HourlyTrancheResolved {
        hire_id: period.hire_id,
        period_index: period.period_index,
        tranche_index: index,
        employee_share,
        employer_share: employer_worker_share,
        commission_to_treasury: treasury_leg,
        commission_refunded: t.commission - treasury_leg,
        forced: false,
        token_mint: period.token_mint,
    });

    Ok(())
}

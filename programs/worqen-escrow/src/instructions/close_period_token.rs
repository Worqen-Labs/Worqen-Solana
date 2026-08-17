use crate::errors::EscrowError;
use crate::events::HourlyPeriodClosed;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount, Transfer};

#[derive(Accounts)]
pub struct ClosePeriodToken<'info> {
    #[account(
        mut,
        seeds = [HourlyPeriod::HOURLY_SEED, hourly_period.hire_id.as_ref(), &hourly_period.period_index.to_le_bytes()],
        bump = hourly_period.bump,
        close = rent_payer,
        constraint = !hourly_period.is_native @ EscrowError::NotTokenEscrow,
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == hourly_period.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against hourly_period.employer; receives the vault ATA rent
    #[account(
        mut,
        constraint = employer.key() == hourly_period.employer @ EscrowError::Unauthorized,
    )]
    pub employer: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = employer_token_account.owner == hourly_period.employer @ EscrowError::Unauthorized,
        constraint = employer_token_account.mint == hourly_period.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: matched against hourly_period.rent_payer; receives period rent
    #[account(
        mut,
        constraint = rent_payer.key() == hourly_period.rent_payer @ EscrowError::Unauthorized,
    )]
    pub rent_payer: UncheckedAccount<'info>,

    pub caller: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<ClosePeriodToken>) -> Result<()> {
    let period = &ctx.accounts.hourly_period;
    require!(
        period.status == HourlyStatus::Settled,
        EscrowError::PeriodNotSettled
    );
    require!(
        period.live_invoices == 0,
        EscrowError::LiveInvoicesOutstanding
    );

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

    let swept = ctx.accounts.vault_token_account.amount;
    if swept > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employer_token_account.to_account_info(),
                    authority: ctx.accounts.hourly_period.to_account_info(),
                },
                signer_seeds,
            ),
            swept,
        )?;
    }

    token::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        CloseAccount {
            account: ctx.accounts.vault_token_account.to_account_info(),
            destination: ctx.accounts.employer.to_account_info(),
            authority: ctx.accounts.hourly_period.to_account_info(),
        },
        signer_seeds,
    ))?;

    emit!(HourlyPeriodClosed {
        hire_id,
        period_index: ctx.accounts.hourly_period.period_index,
        swept,
        is_native: false,
        token_mint: ctx.accounts.hourly_period.token_mint,
    });

    Ok(())
}

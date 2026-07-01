use crate::errors::EscrowError;
use crate::events::HourlyPeriodOpened;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Mint, Token, TokenAccount};

#[derive(Accounts)]
#[instruction(
    hire_id: [u8; 32],
    period_index: u32,
    weekly_cap_net: u64,
    commission_rate_bps: u16,
    review_window_secs: i64
)]
pub struct OpenPeriod<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        init,
        payer = payer,
        space = HourlyPeriod::SPACE,
        seeds = [HourlyPeriod::HOURLY_SEED, hire_id.as_ref(), &period_index.to_le_bytes()],
        bump
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    /// CHECK: stored only
    pub employer: UncheckedAccount<'info>,

    /// CHECK: stored only
    pub employee: UncheckedAccount<'info>,

    /// CHECK: stored only
    pub platform_authority: UncheckedAccount<'info>,

    /// CHECK: matched against config.fee_recipient
    #[account(constraint = fee_recipient.key() == config.fee_recipient @ EscrowError::InvalidFeeRecipient)]
    pub fee_recipient: UncheckedAccount<'info>,

    pub token_mint: Box<Account<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = token_mint,
        associated_token::authority = hourly_period,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = token_mint,
        associated_token::authority = employee,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = token_mint,
        associated_token::authority = fee_recipient,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[allow(clippy::too_many_arguments)]
pub fn handler(
    ctx: Context<OpenPeriod>,
    hire_id: [u8; 32],
    period_index: u32,
    weekly_cap_net: u64,
    commission_rate_bps: u16,
    review_window_secs: i64,
) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);

    let token_mint_key = ctx.accounts.token_mint.key();
    require!(
        ctx.accounts.config.is_mint_allowed(&token_mint_key, false),
        EscrowError::MintNotAllowed
    );
    require!(weekly_cap_net > 0, EscrowError::InvalidAmount);
    require!(
        commission_rate_bps <= Escrow::MAX_COMMISSION_RATE_BPS,
        EscrowError::InvalidCommissionRate
    );

    let employer_key = ctx.accounts.employer.key();
    let employee_key = ctx.accounts.employee.key();
    let platform_key = ctx.accounts.platform_authority.key();
    require!(employer_key != employee_key, EscrowError::EmployeeIsEmployer);
    require!(
        platform_key != employer_key && platform_key != employee_key,
        EscrowError::PlatformAuthorityConflict
    );
    require!(
        review_window_secs > 0 && review_window_secs <= HourlyPeriod::MAX_REVIEW_WINDOW_SECS,
        EscrowError::InvalidAmount
    );

    let clock = Clock::get()?;
    let fee_recipient_key = ctx.accounts.config.fee_recipient;
    let period = &mut ctx.accounts.hourly_period;
    period.version = HOURLY_PERIOD_VERSION;
    period.hire_id = hire_id;
    period.period_index = period_index;
    period.employer = employer_key;
    period.employee = employee_key;
    period.platform_authority = platform_key;
    period.fee_recipient = fee_recipient_key;
    period.token_mint = token_mint_key;
    period.bump = ctx.bumps.hourly_period;
    period.vault_bump = 0;
    period.weekly_cap_net = weekly_cap_net;
    period.commission_rate_bps = commission_rate_bps;
    period.funded_amount = 0;
    period.released_net = 0;
    period.total_staged_net = 0;
    period.tranches = [Tranche::default(); 7];
    period.tranche_count = 0;
    period.review_window_secs = review_window_secs;
    period.created_at = clock.unix_timestamp;
    period.funded_at = 0;
    period.period_end_at = clock.unix_timestamp + HourlyPeriod::DEFAULT_REVIEW_WINDOW_SECS;
    period.completed_at = 0;
    period.status = HourlyStatus::Open;
    period.reserved = [0u8; 64];

    emit!(HourlyPeriodOpened {
        hire_id,
        period_index,
        employer: employer_key,
        employee: employee_key,
        platform_authority: platform_key,
        fee_recipient: fee_recipient_key,
        token_mint: token_mint_key,
        weekly_cap_net,
        commission_rate_bps,
        review_window_secs,
    });

    Ok(())
}

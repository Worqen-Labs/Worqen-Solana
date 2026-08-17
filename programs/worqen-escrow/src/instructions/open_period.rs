use crate::errors::EscrowError;
use crate::events::HourlyPeriodOpened;
use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
#[instruction(
    hire_id: [u8; 32],
    period_index: u32,
)]
pub struct OpenPeriod<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        init,
        payer = authorizer,
        space = HourlyPeriod::SPACE,
        seeds = [HourlyPeriod::HOURLY_SEED, hire_id.as_ref(), &period_index.to_le_bytes()],
        bump
    )]
    pub hourly_period: Box<Account<'info, HourlyPeriod>>,

    /// CHECK: stored only; authorizer is constrained against it
    pub employer: UncheckedAccount<'info>,

    /// CHECK: stored only
    pub employee: UncheckedAccount<'info>,

    /// CHECK: pinned to config.platform_authority so no caller can name a
    /// hostile arbitrator; must be set on Config before any period can open.
    #[account(
        constraint = platform_authority.key() == config.platform_authority
            && config.platform_authority != Pubkey::default() @ EscrowError::InvalidPlatformAuthority,
    )]
    pub platform_authority: UncheckedAccount<'info>,

    /// CHECK: matched against config.fee_recipient
    #[account(constraint = fee_recipient.key() == config.fee_recipient @ EscrowError::InvalidFeeRecipient)]
    pub fee_recipient: UncheckedAccount<'info>,

    /// CHECK: validated against is_native and the config allowlist in the handler
    pub token_mint: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = authorizer.key() == employer.key()
            || authorizer.key() == platform_authority.key() @ EscrowError::Unauthorized,
    )]
    pub authorizer: Signer<'info>,

    /// Platform co-signature: without it nobody can squat a period PDA and
    /// block a hire-week. Redundant with `authorizer` when the platform opens.
    #[account(
        constraint = platform_signer.key() == config.platform_authority
            && config.platform_authority != Pubkey::default() @ EscrowError::InvalidPlatformAuthority,
    )]
    pub platform_signer: Signer<'info>,

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
    period_start_at: i64,
    period_duration_secs: i64,
    is_native: bool,
) -> Result<()> {
    require!(!ctx.accounts.config.paused, EscrowError::ProgramPaused);

    let token_mint_key = ctx.accounts.token_mint.key();
    if is_native {
        require!(
            token_mint_key == anchor_lang::system_program::ID,
            EscrowError::IsNativeMintMismatch
        );
    } else {
        require!(
            token_mint_key != anchor_lang::system_program::ID,
            EscrowError::IsNativeMintMismatch
        );
    }
    require!(
        ctx.accounts
            .config
            .is_mint_allowed(&token_mint_key, is_native),
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
    require!(
        employer_key != employee_key,
        EscrowError::EmployeeIsEmployer
    );
    require!(
        platform_key != employer_key && platform_key != employee_key,
        EscrowError::PlatformAuthorityConflict
    );
    require!(
        review_window_secs > 0 && review_window_secs <= HourlyPeriod::MAX_REVIEW_WINDOW_SECS,
        EscrowError::InvalidAmount
    );

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    require!(
        (HourlyPeriod::MIN_PERIOD_DURATION_SECS..=HourlyPeriod::MAX_PERIOD_DURATION_SECS)
            .contains(&period_duration_secs),
        EscrowError::InvalidPeriodWindow
    );
    require!(period_start_at > 0, EscrowError::InvalidPeriodWindow);
    let period_end_at = period_start_at
        .checked_add(period_duration_secs)
        .ok_or(EscrowError::InvalidPeriodWindow)?;
    require!(period_end_at > now, EscrowError::PeriodEnded);
    require!(
        period_start_at <= now + HourlyPeriod::MAX_PERIOD_DURATION_SECS,
        EscrowError::InvalidPeriodWindow
    );

    let (_, vault_bump) = Pubkey::find_program_address(
        &[
            Escrow::VAULT_SEED,
            ctx.accounts.hourly_period.key().as_ref(),
        ],
        ctx.program_id,
    );

    let fee_recipient_key = ctx.accounts.config.fee_recipient;
    let rent_payer_key = ctx.accounts.authorizer.key();
    let period = &mut ctx.accounts.hourly_period;
    period.version = HOURLY_PERIOD_VERSION;
    period.hire_id = hire_id;
    period.period_index = period_index;
    period.employer = employer_key;
    period.employee = employee_key;
    period.platform_authority = platform_key;
    period.fee_recipient = fee_recipient_key;
    period.token_mint = token_mint_key;
    period.is_native = is_native;
    period.bump = ctx.bumps.hourly_period;
    period.vault_bump = vault_bump;
    period.rent_payer = rent_payer_key;
    period.weekly_cap_net = weekly_cap_net;
    period.commission_rate_bps = commission_rate_bps;
    period.funded_gross = 0;
    period.total_staged_net = 0;
    period.released_net = 0;
    period.refunded_amount = 0;
    period.outstanding_net = 0;
    period.outstanding_commission = 0;
    period.vault_rent_reserve = 0;
    period.invoice_count = 0;
    period.live_invoices = 0;
    period.review_window_secs = review_window_secs;
    period.period_start_at = period_start_at;
    period.period_end_at = period_end_at;
    period.created_at = now;
    period.funded_at = 0;
    period.settled_at = 0;
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
        is_native,
        weekly_cap_net,
        commission_rate_bps,
        review_window_secs,
        period_start_at,
        period_end_at,
        rent_payer: rent_payer_key,
    });

    Ok(())
}

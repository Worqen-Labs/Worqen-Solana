use anchor_lang::prelude::*;

pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;

#[cfg(not(feature = "no-entrypoint"))]
use solana_security_txt::security_txt;

#[cfg(not(feature = "no-entrypoint"))]
security_txt! {
    name: "Worqen Escrow",
    project_url: "https://worqen.com",
    contacts: "email:security@worqen.com",
    policy: "https://github.com/Worqen-Labs/Worqen-Escrow/blob/main/SECURITY.md",
    preferred_languages: "en",
    source_code: "https://github.com/Worqen-Labs/Worqen-Escrow",
    source_release: "v1.1.0",
    auditors: "Pending external audit"
}

// Program ID is per-cluster. Mainnet builds (`--features mainnet`, used only by
// release.yml's verifiable build) get their own id; every other build — devnet,
// localnet, LiteSVM tests, CI — stays on the devnet id. Keep both in sync with
// Anchor.toml [programs.*] and the backend/frontend ESCROW_PROGRAM_ID env.
#[cfg(feature = "mainnet")]
declare_id!("HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs");
#[cfg(not(feature = "mainnet"))]
declare_id!("6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ");

/// Trustless payment escrow for the Worqen job marketplace (native SOL and an
/// allowlisted set of SPL tokens). Employers lock funds when hiring; employees
/// are paid only after confirmation, with platform-mediated dispute resolution
/// and a deadline-based force-resolve as a platform-failure safety net.
///
/// Key invariants: commission is fee-on-top (employer pays `amount + commission`,
/// employee receives the full `amount`, commission goes to a per-escrow
/// `fee_recipient`); pause blocks only new money, never releases/disputes/closes;
/// token destinations are constrained on `mint`/`owner` and SOL paths drain the
/// actual vault balance (dust-DoS safe). Account schema: `state::Escrow`.
#[program]
pub mod worqen_escrow {
    use super::*;

    /// Initialize the singleton global Config (admin signs + pays rent).
    pub fn init_config(
        ctx: Context<InitConfig>,
        fee_recipient: Pubkey,
        default_commission_bps: u16,
        allowed_mints: Vec<Pubkey>,
    ) -> Result<()> {
        instructions::config::init_config(ctx, fee_recipient, default_commission_bps, allowed_mints)
    }

    /// Update Config fields (current authority signs). Any `None` is left
    /// unchanged. Setting `new_pending_authority` starts a two-step handoff.
    pub fn update_config(
        ctx: Context<UpdateConfig>,
        new_fee_recipient: Option<Pubkey>,
        new_default_commission_bps: Option<u16>,
        new_paused: Option<bool>,
        new_pending_authority: Option<Pubkey>,
    ) -> Result<()> {
        instructions::config::update_config(
            ctx,
            new_fee_recipient,
            new_default_commission_bps,
            new_paused,
            new_pending_authority,
        )
    }

    /// Accept a pending authority handoff (the pending authority signs).
    pub fn accept_authority(ctx: Context<AcceptAuthority>) -> Result<()> {
        instructions::config::accept_authority(ctx)
    }

    /// Add an SPL mint to the allowlist (authority signs).
    pub fn add_allowed_mint(ctx: Context<UpdateAllowlist>, mint: Pubkey) -> Result<()> {
        instructions::config::add_allowed_mint(ctx, mint)
    }

    /// Remove an SPL mint from the allowlist (authority signs).
    pub fn remove_allowed_mint(ctx: Context<UpdateAllowlist>, mint: Pubkey) -> Result<()> {
        instructions::config::remove_allowed_mint(ctx, mint)
    }

    /// Create a new escrow account for a hire or milestone.
    #[allow(clippy::too_many_arguments)]
    pub fn create_escrow(
        ctx: Context<CreateEscrow>,
        escrow_id: [u8; 32],
        escrow_group_id: [u8; 32],
        sequence_in_group: u8,
        total_in_group: u8,
        amount: u64,
        is_native: bool,
        commission_rate_bps: u16,
        auto_release_at: i64,
        escrow_kind: u8,
        terms_hash: [u8; 32],
    ) -> Result<()> {
        instructions::create_escrow::handler(
            ctx,
            escrow_id,
            escrow_group_id,
            sequence_in_group,
            total_in_group,
            amount,
            is_native,
            commission_rate_bps,
            auto_release_at,
            escrow_kind,
            terms_hash,
        )
    }

    /// Deposit native SOL into the escrow vault (employer signs).
    pub fn deposit_sol(ctx: Context<DepositSol>) -> Result<()> {
        instructions::deposit_sol::handler(ctx)
    }

    /// Deposit SPL tokens into the escrow vault (employer signs).
    pub fn deposit_token(ctx: Context<DepositToken>) -> Result<()> {
        instructions::deposit_token::handler(ctx)
    }

    /// Either party confirms work completion.
    pub fn confirm_completion(ctx: Context<ConfirmCompletion>) -> Result<()> {
        instructions::confirm_completion::handler(ctx)
    }

    /// Release the remaining SOL to the employee (+ commission to treasury).
    pub fn release_sol(ctx: Context<ReleaseSol>, ref_id: [u8; 32]) -> Result<()> {
        instructions::release_sol::handler(ctx, ref_id)
    }

    /// Release the remaining SPL tokens to the employee (+ commission).
    pub fn release_token(ctx: Context<ReleaseToken>, ref_id: [u8; 32]) -> Result<()> {
        instructions::release_token::handler(ctx, ref_id)
    }

    /// Release `amount` SOL to employee now, keeping the rest escrowed.
    pub fn release_partial_sol(
        ctx: Context<ReleasePartialSol>,
        amount: u64,
        ref_id: [u8; 32],
    ) -> Result<()> {
        instructions::release_partial_sol::handler(ctx, amount, ref_id)
    }

    /// Release `amount` SPL tokens to employee now, keeping the rest.
    pub fn release_partial_token(
        ctx: Context<ReleasePartialToken>,
        amount: u64,
        ref_id: [u8; 32],
    ) -> Result<()> {
        instructions::release_partial_token::handler(ctx, amount, ref_id)
    }

    /// Raise a dispute, freezing funds. `dispute_deadline` is mandatory and
    /// must be within [now + 3 days, now + 90 days].
    pub fn raise_dispute(
        ctx: Context<RaiseDispute>,
        reason: Vec<u8>,
        dispute_deadline: i64,
    ) -> Result<()> {
        instructions::raise_dispute::handler(ctx, reason, dispute_deadline)
    }

    /// Platform resolves a dispute by splitting the remaining worker amount.
    pub fn resolve_dispute_sol(ctx: Context<ResolveDisputeSol>, employee_share: u64) -> Result<()> {
        instructions::resolve_dispute_sol::handler(ctx, employee_share)
    }

    /// Platform resolves a token dispute.
    pub fn resolve_dispute_token(
        ctx: Context<ResolveDisputeToken>,
        employee_share: u64,
    ) -> Result<()> {
        instructions::resolve_dispute_token::handler(ctx, employee_share)
    }

    /// Cancel a SOL escrow. Employer may cancel only in `Created`; platform
    /// may cancel in `Created` or `Funded`. Full refund to employer.
    pub fn cancel_escrow_sol(ctx: Context<CancelEscrowSol>, reason: Vec<u8>) -> Result<()> {
        instructions::cancel_escrow_sol::handler(ctx, reason)
    }

    /// Cancel a token escrow. Same rules as `cancel_escrow_sol`.
    pub fn cancel_escrow_token(ctx: Context<CancelEscrowToken>, reason: Vec<u8>) -> Result<()> {
        instructions::cancel_escrow_token::handler(ctx, reason)
    }

    /// Permissionless force-resolve of a `Disputed` escrow after
    /// `dispute_deadline`. Pays the worker their remaining amount.
    pub fn trigger_auto_release_sol(ctx: Context<TriggerAutoReleaseSol>) -> Result<()> {
        instructions::trigger_auto_release_sol::handler(ctx)
    }

    /// Token variant of `trigger_auto_release_sol`.
    pub fn trigger_auto_release_token(ctx: Context<TriggerAutoReleaseToken>) -> Result<()> {
        instructions::trigger_auto_release_token::handler(ctx)
    }

    /// Rotate the escrow's platform_authority (current authority signs).
    /// Blocked while `Disputed`; the new authority must differ from the
    /// current one and from employer/employee.
    pub fn update_platform_authority(ctx: Context<UpdatePlatformAuthority>) -> Result<()> {
        instructions::update_platform_authority::handler(ctx)
    }

    /// Close a terminal SOL escrow and refund rent + vault dust to employer.
    pub fn close_escrow_sol(ctx: Context<CloseEscrowSol>) -> Result<()> {
        instructions::close_escrow_sol::handler(ctx)
    }

    /// Close a terminal token escrow, sweep residual tokens, refund all rent.
    pub fn close_escrow_token(ctx: Context<CloseEscrowToken>) -> Result<()> {
        instructions::close_escrow_token::handler(ctx)
    }

    /// Close a never-funded (Cancelled-from-Created) SOL escrow to reclaim
    /// its account rent. No vault is involved.
    pub fn close_unfunded_escrow_sol(ctx: Context<CloseUnfundedEscrowSol>) -> Result<()> {
        instructions::close_unfunded_escrow_sol::handler(ctx)
    }

    /// Token variant: close a never-funded escrow (no vault ATA was created).
    pub fn close_unfunded_escrow_token(ctx: Context<CloseUnfundedEscrowToken>) -> Result<()> {
        instructions::close_unfunded_escrow_token::handler(ctx)
    }

    /// Direct one-shot SOL payment with platform commission (fee-on-top).
    /// No escrow, no lock — an atomic split. For trusted hires, tips, and
    /// approved-invoice settlement. Subject to the mint allowlist + pause.
    pub fn pay_with_commission_sol(
        ctx: Context<PayWithCommissionSol>,
        hire_id: [u8; 32],
        amount: u64,
        commission_bps: u16,
    ) -> Result<()> {
        instructions::pay_with_commission_sol::handler(ctx, hire_id, amount, commission_bps)
    }

    /// Token variant of `pay_with_commission_sol`.
    pub fn pay_with_commission_token(
        ctx: Context<PayWithCommissionToken>,
        hire_id: [u8; 32],
        amount: u64,
        commission_bps: u16,
    ) -> Result<()> {
        instructions::pay_with_commission_token::handler(ctx, hire_id, amount, commission_bps)
    }

    /// Top up a Funded SOL escrow (retainer/hourly): raises amount +
    /// commission and moves the delta into the vault. Employer signs.
    pub fn deposit_more_sol(ctx: Context<DepositMoreSol>, additional_amount: u64) -> Result<()> {
        instructions::deposit_more_sol::handler(ctx, additional_amount)
    }

    /// Token variant of `deposit_more_sol`.
    pub fn deposit_more_token(
        ctx: Context<DepositMoreToken>,
        additional_amount: u64,
    ) -> Result<()> {
        instructions::deposit_more_token::handler(ctx, additional_amount)
    }

    /// Direct pay (fee-on-top) split across many recipients in one atomic tx
    /// (teams / referral fees). Recipient SOL accounts are passed via
    /// remaining_accounts; `amounts[i]` is recipient i's net; one commission on
    /// the total goes to the treasury. Subject to the pause switch.
    pub fn batch_pay_with_commission_sol<'info>(
        ctx: Context<'_, '_, 'info, 'info, BatchPayWithCommissionSol<'info>>,
        hire_id: [u8; 32],
        amounts: Vec<u64>,
        commission_bps: u16,
    ) -> Result<()> {
        instructions::batch_pay_with_commission_sol::handler(ctx, hire_id, amounts, commission_bps)
    }

    /// Token variant: recipient ATAs passed via remaining_accounts.
    pub fn batch_pay_with_commission_token<'info>(
        ctx: Context<'_, '_, 'info, 'info, BatchPayWithCommissionToken<'info>>,
        hire_id: [u8; 32],
        amounts: Vec<u64>,
        commission_bps: u16,
    ) -> Result<()> {
        instructions::batch_pay_with_commission_token::handler(
            ctx,
            hire_id,
            amounts,
            commission_bps,
        )
    }

    /// Amicable settle of a non-terminal SOL escrow WITHOUT a dispute. Both
    /// employer AND employee sign; `employee_share` (<= remaining worker
    /// amount) goes to the employee, the rest (incl. commission) refunds to
    /// the employer.
    pub fn mutual_cancel_sol(ctx: Context<MutualCancelSol>, employee_share: u64) -> Result<()> {
        instructions::mutual_cancel_sol::handler(ctx, employee_share)
    }

    /// Token variant of `mutual_cancel_sol`.
    pub fn mutual_cancel_token(ctx: Context<MutualCancelToken>, employee_share: u64) -> Result<()> {
        instructions::mutual_cancel_token::handler(ctx, employee_share)
    }

    pub fn open_period(
        ctx: Context<OpenPeriod>,
        hire_id: [u8; 32],
        period_index: u32,
        weekly_cap_net: u64,
        commission_rate_bps: u16,
        review_window_secs: i64,
    ) -> Result<()> {
        instructions::open_period::handler(
            ctx,
            hire_id,
            period_index,
            weekly_cap_net,
            commission_rate_bps,
            review_window_secs,
        )
    }

    pub fn fund_period(ctx: Context<FundPeriod>) -> Result<()> {
        instructions::fund_period::handler(ctx)
    }

    pub fn pull_fund_period(ctx: Context<PullFundPeriod>) -> Result<()> {
        instructions::pull_fund_period::handler(ctx)
    }

    pub fn raise_weekly_cap(ctx: Context<RaiseWeeklyCap>, new_weekly_cap_net: u64) -> Result<()> {
        instructions::raise_weekly_cap::handler(ctx, new_weekly_cap_net)
    }

    pub fn stage_tranche(ctx: Context<StageTranche>, amount: u64) -> Result<()> {
        instructions::stage_tranche::handler(ctx, amount)
    }

    pub fn finalize_tranche(ctx: Context<FinalizeTranche>, index: u8) -> Result<()> {
        instructions::finalize_tranche::handler(ctx, index)
    }

    pub fn raise_hourly_dispute(
        ctx: Context<RaiseHourlyDispute>,
        index: u8,
        dispute_deadline: i64,
        reason: Vec<u8>,
    ) -> Result<()> {
        instructions::raise_hourly_dispute::handler(ctx, index, dispute_deadline, reason)
    }

    pub fn resolve_hourly_tranche(
        ctx: Context<ResolveHourlyTranche>,
        index: u8,
        employee_share: u64,
    ) -> Result<()> {
        instructions::resolve_hourly_tranche::handler(ctx, index, employee_share)
    }

    pub fn trigger_hourly_auto_release(
        ctx: Context<TriggerHourlyAutoRelease>,
        index: u8,
    ) -> Result<()> {
        instructions::trigger_hourly_auto_release::handler(ctx, index)
    }

    pub fn refund_remainder(ctx: Context<RefundRemainder>) -> Result<()> {
        instructions::refund_remainder::handler(ctx)
    }

    pub fn close_period(ctx: Context<ClosePeriod>) -> Result<()> {
        instructions::close_period::handler(ctx)
    }
}

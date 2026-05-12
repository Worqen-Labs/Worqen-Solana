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
    policy: "https://github.com/worqen-labs/worqen-escrow/blob/main/SECURITY.md",
    preferred_languages: "en",
    source_code: "https://github.com/worqen-labs/worqen-escrow",
    source_release: "v1.0.0",
    auditors: "Pending external audit"
}

declare_id!("GDCBqN8AVU5i2xXdeTNwBmCCsd9Y8rfiH1JDKA8UjDYh");

/// Worqen Escrow Program
///
/// Provides trustless payment escrow for the Worqen job marketplace.
/// Supports native SOL and any SPL token. Employers lock funds when hiring;
/// employees are paid only after confirmation, with platform-mediated
/// dispute resolution and a deadline-based force-resolve as a safety net
/// for platform failure.
///
/// **Account schema:** see `state::Escrow` (`ESCROW_ACCOUNT_VERSION = 2`).
/// v2 semantics:
///   - Employer can no longer cancel a `Funded` escrow.
///   - Worker can self-release once both parties have confirmed.
///   - `dispute_deadline` is mandatory and bounded to 90 days.
///   - `auto_release_at` no longer triggers releases from
///     `Funded`/`PendingRelease`. Force-resolve only fires from `Disputed`.
///   - All token destinations are constrained on `mint` and `owner`.
///   - SOL release / resolve / auto-release drain the actual vault balance
///     to defend against dust DoS.
///   - `close_escrow_*` reclaims rent on terminal escrows.
#[program]
pub mod worqen_escrow {
    use super::*;

    /// Create a new escrow account for a hire or milestone.
    ///
    /// See `instructions::create_escrow` for full argument docs.
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

    /// Release the remaining SOL to the employee (+ commission to platform).
    pub fn release_sol(ctx: Context<ReleaseSol>) -> Result<()> {
        instructions::release_sol::handler(ctx)
    }

    /// Release the remaining SPL tokens to the employee (+ commission).
    pub fn release_token(ctx: Context<ReleaseToken>) -> Result<()> {
        instructions::release_token::handler(ctx)
    }

    /// Release `amount` SOL to employee now, keeping the rest escrowed.
    /// Proportional commission is paid to platform.
    pub fn release_partial_sol(ctx: Context<ReleasePartialSol>, amount: u64) -> Result<()> {
        instructions::release_partial_sol::handler(ctx, amount)
    }

    /// Release `amount` SPL tokens to employee now, keeping the rest.
    pub fn release_partial_token(
        ctx: Context<ReleasePartialToken>,
        amount: u64,
    ) -> Result<()> {
        instructions::release_partial_token::handler(ctx, amount)
    }

    /// Raise a dispute, freezing funds. Optionally set a deadline.
    pub fn raise_dispute(
        ctx: Context<RaiseDispute>,
        reason: Vec<u8>,
        dispute_deadline: i64,
    ) -> Result<()> {
        instructions::raise_dispute::handler(ctx, reason, dispute_deadline)
    }

    /// Platform resolves a dispute by splitting the remaining worker amount.
    pub fn resolve_dispute_sol(
        ctx: Context<ResolveDisputeSol>,
        employee_share: u64,
    ) -> Result<()> {
        instructions::resolve_dispute_sol::handler(ctx, employee_share)
    }

    /// Platform resolves a token dispute.
    pub fn resolve_dispute_token(
        ctx: Context<ResolveDisputeToken>,
        employee_share: u64,
    ) -> Result<()> {
        instructions::resolve_dispute_token::handler(ctx, employee_share)
    }

    /// Cancel a SOL escrow (employer or platform), full refund to employer.
    pub fn cancel_escrow_sol(
        ctx: Context<CancelEscrowSol>,
        reason: Vec<u8>,
    ) -> Result<()> {
        instructions::cancel_escrow_sol::handler(ctx, reason)
    }

    /// Cancel a token escrow, full refund to employer.
    pub fn cancel_escrow_token(
        ctx: Context<CancelEscrowToken>,
        reason: Vec<u8>,
    ) -> Result<()> {
        instructions::cancel_escrow_token::handler(ctx, reason)
    }

    /// Anyone can trigger this after `auto_release_at` (Funded/PendingRelease)
    /// or after `dispute_deadline` (Disputed). Prevents stuck funds.
    pub fn trigger_auto_release_sol(ctx: Context<TriggerAutoReleaseSol>) -> Result<()> {
        instructions::trigger_auto_release_sol::handler(ctx)
    }

    /// Token variant of trigger_auto_release_sol.
    pub fn trigger_auto_release_token(ctx: Context<TriggerAutoReleaseToken>) -> Result<()> {
        instructions::trigger_auto_release_token::handler(ctx)
    }

    /// Rotate the escrow's platform_authority (current authority signs).
    /// Blocked while status == Disputed (v2).
    pub fn update_platform_authority(ctx: Context<UpdatePlatformAuthority>) -> Result<()> {
        instructions::update_platform_authority::handler(ctx)
    }

    /// Close a terminal SOL escrow and refund storage rent (~0.005 SOL)
    /// plus any vault dust to the employer. Allowed in Released / Resolved /
    /// Cancelled. Signed by employer or platform_authority.
    pub fn close_escrow_sol(ctx: Context<CloseEscrowSol>) -> Result<()> {
        instructions::close_escrow_sol::handler(ctx)
    }

    /// Close a terminal token escrow: sweeps any residual tokens to the
    /// employer's ATA, closes the vault token account (~0.002 SOL rent),
    /// and closes the escrow account (~0.005 SOL rent). All rent goes to
    /// employer. Signed by employer or platform_authority.
    pub fn close_escrow_token(ctx: Context<CloseEscrowToken>) -> Result<()> {
        instructions::close_escrow_token::handler(ctx)
    }

    /// Direct one-shot SOL payment with platform commission. No escrow,
    /// no lock, no state — just an atomic split transfer. Use for trusted
    /// hires, tips, recurring invoice settlement, or any "pay now, track
    /// off-chain" flow where the platform still needs its cut.
    pub fn pay_with_commission_sol(
        ctx: Context<PayWithCommissionSol>,
        hire_id: [u8; 32],
        amount: u64,
        commission_bps: u16,
    ) -> Result<()> {
        instructions::pay_with_commission_sol::handler(ctx, hire_id, amount, commission_bps)
    }

    /// Token variant of `pay_with_commission_sol`. Source/destination ATAs
    /// must exist — create them idempotently in a preceding instruction.
    pub fn pay_with_commission_token(
        ctx: Context<PayWithCommissionToken>,
        hire_id: [u8; 32],
        amount: u64,
        commission_bps: u16,
    ) -> Result<()> {
        instructions::pay_with_commission_token::handler(ctx, hire_id, amount, commission_bps)
    }
}

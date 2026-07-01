use anchor_lang::prelude::*;

/// Custom error codes for the Worqen Escrow program
#[error_code]
pub enum EscrowError {
    #[msg("Invalid escrow status for this operation")]
    InvalidStatus,

    #[msg("Not authorized to perform this action")]
    Unauthorized,

    #[msg("Operation requires native SOL escrow")]
    NotNativeEscrow,

    #[msg("Operation requires SPL token escrow")]
    NotTokenEscrow,

    #[msg("Party has already confirmed completion")]
    AlreadyConfirmed,

    #[msg("Release requires employer confirmation or platform authority")]
    ReleaseNotAuthorized,

    #[msg("Invalid amount specified")]
    InvalidAmount,

    #[msg("Dispute reason exceeds maximum length")]
    DisputeReasonTooLong,

    #[msg("Invalid token mint for this escrow")]
    InvalidTokenMint,

    #[msg("Insufficient funds in vault")]
    InsufficientFunds,

    #[msg("Employee share exceeds remaining worker amount")]
    InvalidEmployeeShare,

    #[msg("Commission rate exceeds maximum allowed (10%)")]
    InvalidCommissionRate,

    #[msg("Employee and employer must be different addresses")]
    EmployeeIsEmployer,

    #[msg("Platform authority must differ from employer and employee")]
    PlatformAuthorityConflict,

    #[msg("Cancellation reason exceeds maximum length")]
    CancellationReasonTooLong,

    #[msg("Auto-release deadline has not been reached")]
    AutoReleaseNotReached,

    #[msg("Auto-release is not configured for this escrow")]
    AutoReleaseNotConfigured,

    #[msg("Dispute deadline has not been reached")]
    DisputeDeadlineNotReached,

    #[msg("Partial release exceeds remaining worker amount")]
    PartialReleaseTooLarge,

    #[msg("sequence_in_group must be in [1, total_in_group] when grouped")]
    InvalidGroupSequence,

    #[msg("New platform authority cannot equal employer or employee")]
    InvalidNewPlatformAuthority,

    #[msg("auto_release_at must be in the future")]
    InvalidAutoReleaseTime,

    #[msg("dispute_deadline must be in the future")]
    InvalidDisputeDeadline,

    #[msg("Self-payment is not allowed")]
    SelfPaymentNotAllowed,

    #[msg("Dispute is locked once either party has confirmed completion")]
    DisputeLockedAfterConfirm,

    /// A zero deadline would disable the platform-failure safety net.
    #[msg("dispute_deadline must be greater than 0")]
    DisputeDeadlineRequired,

    #[msg("dispute_deadline exceeds the maximum allowed window")]
    DisputeDeadlineTooLong,

    /// Funded escrows must go through dispute resolution, not unilateral cancel.
    #[msg("Employer cannot cancel after the escrow has been funded; raise a dispute instead")]
    EmployerCancelAfterFundedDisallowed,

    /// Blocked during a dispute so a compromised authority can't escalate mid-dispute.
    #[msg("Cannot rotate platform_authority while escrow is in Disputed state")]
    AuthorityRotationDuringDispute,

    #[msg("Escrow is not in a terminal status; cannot close")]
    EscrowNotTerminal,

    #[msg("auto_release_at exceeds the maximum allowed window")]
    AutoReleaseTooFar,

    /// `is_native = true` requires SystemProgram::ID and vice versa.
    #[msg("is_native and token_mint must be consistent")]
    IsNativeMintMismatch,

    /// Solana rejects an account left below rent-exempt minimum but above zero.
    #[msg("Partial release would leave vault below rent-exempt minimum; release in full or adjust amount")]
    PartialReleaseLeavesDust,

    #[msg("Token vault is non-empty; transfer remaining tokens before closing")]
    VaultNotEmpty,

    /// Too short a window would let a party force-resolve before the platform can mediate.
    #[msg("dispute_deadline is sooner than the minimum allowed window")]
    DisputeWindowTooShort,

    #[msg("Token mint is not allowed by platform config")]
    MintNotAllowed,

    /// When paused, new escrows/deposits/direct payments are blocked; releases, disputes and closes are not.
    #[msg("Program is paused")]
    ProgramPaused,

    #[msg("fee_recipient account does not match the escrow")]
    InvalidFeeRecipient,

    #[msg("No pending authority to accept")]
    NoPendingAuthority,

    #[msg("Signer is not the pending authority")]
    PendingAuthorityMismatch,

    #[msg("Escrow was funded; use close_escrow_* instead")]
    EscrowWasFunded,

    #[msg("Mint allowlist is full or the mint is already present")]
    MintAllowlistFull,

    #[msg("Too many recipients in batch payment")]
    TooManyRecipients,

    #[msg("Recipient count does not match amounts length")]
    RecipientCountMismatch,

    #[msg("Batch payment must have at least one recipient")]
    EmptyBatch,

    #[msg("Top-up requires the escrow to be Funded")]
    TopUpNotFunded,

    #[msg("Staged amount would exceed the weekly cap")]
    WeeklyCapExceeded,

    #[msg("Weekly tranche limit (7) reached")]
    TrancheLimitReached,

    #[msg("Tranche is not in Frozen status")]
    TrancheNotFrozen,

    #[msg("Tranche is not in Disputed status")]
    TrancheNotDisputed,

    #[msg("Tranche review window has not elapsed")]
    TrancheWindowNotElapsed,

    #[msg("Cannot dispute after the tranche review window has opened")]
    DisputeWindowClosed,

    #[msg("Tranche index out of bounds")]
    InvalidTrancheIndex,

    #[msg("Vault balance insufficient to back this earmark")]
    VaultUnderfunded,

    #[msg("employee_share exceeds the tranche amount")]
    HourlyEmployeeShareExceedsTranche,

    #[msg("Weekly cap can only be raised, never lowered")]
    CapCannotDecrease,

    #[msg("Weekly cap cannot drop below already-staged total")]
    CapBelowStaged,

    #[msg("Period vault already funded to the current cap_gross")]
    PeriodFullyFunded,

    #[msg("Period has live earmarks; cannot close")]
    HourlyPeriodNotTerminal,
}

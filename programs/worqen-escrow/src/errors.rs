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

    #[msg("Reserved error code")]
    Reserved6015,

    #[msg("Auto-release is not configured for this escrow")]
    AutoReleaseNotConfigured,

    #[msg("Dispute deadline has not been reached")]
    DisputeDeadlineNotReached,

    #[msg("Reserved error code")]
    Reserved6018,

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

    #[msg("Reserved error code")]
    Reserved6032,

    #[msg("Reserved error code")]
    Reserved6033,

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

    #[msg("Reserved error code")]
    Reserved6045,

    #[msg("Staged amount would exceed the weekly cap")]
    WeeklyCapExceeded,

    #[msg("Invoice is not in Staged status")]
    InvoiceNotStaged,

    #[msg("Invoice is not in Disputed status")]
    InvoiceNotDisputed,

    #[msg("Invoice review window has not elapsed")]
    InvoiceWindowNotElapsed,

    #[msg("Cannot dispute after the invoice release time")]
    DisputeWindowClosed,

    #[msg("Vault balance insufficient to back this earmark")]
    VaultUnderfunded,

    #[msg("employee_share exceeds the invoice amount")]
    EmployeeShareExceedsInvoice,

    #[msg("Weekly cap can only be raised, never lowered")]
    CapCannotDecrease,

    #[msg("Weekly cap cannot drop below already-staged total")]
    CapBelowStaged,

    #[msg("Period vault already funded to the current cap_gross")]
    PeriodFullyFunded,

    #[msg("Period window has already ended")]
    PeriodEnded,

    #[msg("Period window has not started yet")]
    PeriodNotStarted,

    #[msg("Period window has not ended yet")]
    PeriodNotEnded,

    #[msg("Period is already settled")]
    PeriodAlreadySettled,

    #[msg("Period must be settled before closing")]
    PeriodNotSettled,

    #[msg("Period still has live invoices")]
    LiveInvoicesOutstanding,

    #[msg("Invalid period start/duration window")]
    InvalidPeriodWindow,

    #[msg("Invoice index overflow")]
    InvoiceIndexOverflow,

    #[msg("Invoice does not belong to this period")]
    InvoicePeriodMismatch,

    #[msg("Period platform_authority is unset or does not match Config")]
    InvalidPlatformAuthority,

    #[msg("Funding amount exceeds the caller-supplied maximum")]
    FundExceedsMax,

    #[msg("escrow_kind is not a known kind")]
    InvalidEscrowKind,
}

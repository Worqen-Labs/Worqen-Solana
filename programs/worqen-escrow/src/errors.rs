use anchor_lang::prelude::*;

/// Custom error codes for the Worqen Escrow program
#[error_code]
pub enum EscrowError {
    /// Error code 6000: Invalid escrow status for this operation
    #[msg("Invalid escrow status for this operation")]
    InvalidStatus,

    /// Error code 6001: Not authorized to perform this action
    #[msg("Not authorized to perform this action")]
    Unauthorized,

    /// Error code 6002: Operation requires native SOL escrow
    #[msg("Operation requires native SOL escrow")]
    NotNativeEscrow,

    /// Error code 6003: Operation requires SPL token escrow
    #[msg("Operation requires SPL token escrow")]
    NotTokenEscrow,

    /// Error code 6004: Party has already confirmed
    #[msg("Party has already confirmed completion")]
    AlreadyConfirmed,

    /// Error code 6005: Release requires employer confirmation or platform authority
    #[msg("Release requires employer confirmation or platform authority")]
    ReleaseNotAuthorized,

    /// Error code 6006: Invalid amount specified
    #[msg("Invalid amount specified")]
    InvalidAmount,

    /// Error code 6007: Dispute reason too long
    #[msg("Dispute reason exceeds maximum length")]
    DisputeReasonTooLong,

    /// Error code 6008: Invalid token mint
    #[msg("Invalid token mint for this escrow")]
    InvalidTokenMint,

    /// Error code 6009: Insufficient funds in vault
    #[msg("Insufficient funds in vault")]
    InsufficientFunds,

    /// Error code 6010: Employee share exceeds remaining vault amount
    #[msg("Employee share exceeds remaining worker amount")]
    InvalidEmployeeShare,

    /// Error code 6011: Commission rate exceeds maximum allowed
    #[msg("Commission rate exceeds maximum allowed (10%)")]
    InvalidCommissionRate,

    /// Error code 6012: Employee and employer must be different addresses
    #[msg("Employee and employer must be different addresses")]
    EmployeeIsEmployer,

    /// Error code 6013: Platform authority must differ from employer/employee
    #[msg("Platform authority must differ from employer and employee")]
    PlatformAuthorityConflict,

    /// Error code 6014: Cancellation reason exceeds maximum length
    #[msg("Cancellation reason exceeds maximum length")]
    CancellationReasonTooLong,

    /// Error code 6015: Auto-release deadline has not been reached
    #[msg("Auto-release deadline has not been reached")]
    AutoReleaseNotReached,

    /// Error code 6016: Auto-release is not configured for this escrow
    #[msg("Auto-release is not configured for this escrow")]
    AutoReleaseNotConfigured,

    /// Error code 6017: Dispute deadline has not been reached
    #[msg("Dispute deadline has not been reached")]
    DisputeDeadlineNotReached,

    /// Error code 6018: Partial release would exceed remaining amount
    #[msg("Partial release exceeds remaining worker amount")]
    PartialReleaseTooLarge,

    /// Error code 6019: Group sequence numbers are inconsistent
    #[msg("sequence_in_group must be in [1, total_in_group] when grouped")]
    InvalidGroupSequence,

    /// Error code 6020: New platform authority cannot equal employer/employee
    #[msg("New platform authority cannot equal employer or employee")]
    InvalidNewPlatformAuthority,

    /// Error code 6021: Invalid auto-release configuration (must be future timestamp)
    #[msg("auto_release_at must be in the future")]
    InvalidAutoReleaseTime,

    /// Error code 6022: Invalid dispute deadline (must be future timestamp)
    #[msg("dispute_deadline must be in the future")]
    InvalidDisputeDeadline,

    /// Error code 6023: Payer and recipient must differ (direct-pay path)
    #[msg("Self-payment is not allowed")]
    SelfPaymentNotAllowed,

    /// Error code 6024: Worker cannot dispute after either party confirmed
    #[msg("Dispute is locked once either party has confirmed completion")]
    DisputeLockedAfterConfirm,

    /// Error code 6025: dispute_deadline = 0 disables the platform-failure
    /// safety net; require a real value.
    #[msg("dispute_deadline must be greater than 0")]
    DisputeDeadlineRequired,

    /// Error code 6026: dispute_deadline beyond MAX_DISPUTE_DEADLINE_DURATION
    #[msg("dispute_deadline exceeds the maximum allowed window")]
    DisputeDeadlineTooLong,

    /// Error code 6027: Employer cannot unilaterally cancel a Funded escrow.
    /// They must raise a dispute and have the platform resolve it.
    #[msg("Employer cannot cancel after the escrow has been funded; raise a dispute instead")]
    EmployerCancelAfterFundedDisallowed,

    /// Error code 6028: Authority rotation is blocked while the escrow is
    /// in `Disputed` so a compromised authority can't escalate mid-dispute.
    #[msg("Cannot rotate platform_authority while escrow is in Disputed state")]
    AuthorityRotationDuringDispute,

    /// Error code 6029: `close_escrow_*` requires a terminal status
    /// (Released / Resolved / Cancelled).
    #[msg("Escrow is not in a terminal status; cannot close")]
    EscrowNotTerminal,

    /// Error code 6030: auto_release_at exceeds MAX_AUTO_RELEASE_DURATION
    #[msg("auto_release_at exceeds the maximum allowed window")]
    AutoReleaseTooFar,

    /// Error code 6031: is_native flag and token_mint disagree
    /// (`is_native = true` requires SystemProgram::ID and vice versa).
    #[msg("is_native and token_mint must be consistent")]
    IsNativeMintMismatch,

    /// Error code 6032: Partial release would leave the SOL vault below
    /// rent-exempt minimum but above 0, which Solana rejects.
    #[msg("Partial release would leave vault below rent-exempt minimum; release in full or adjust amount")]
    PartialReleaseLeavesDust,

    /// Error code 6033: Vault still holds tokens; sweep before close
    #[msg("Token vault is non-empty; transfer remaining tokens before closing")]
    VaultNotEmpty,
}

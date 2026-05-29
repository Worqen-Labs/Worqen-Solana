use anchor_lang::prelude::*;

/// Emitted when a new escrow is created
#[event]
pub struct EscrowCreated {
    pub escrow_id: [u8; 32],
    pub escrow_group_id: [u8; 32],
    pub sequence_in_group: u8,
    pub total_in_group: u8,
    pub employer: Pubkey,
    pub employee: Pubkey,
    pub platform_authority: Pubkey,
    /// Treasury that will receive commission (snapshotted from Config).
    pub fee_recipient: Pubkey,
    pub amount: u64,
    pub commission_amount: u64,
    pub commission_rate_bps: u16,
    pub is_native: bool,
    pub token_mint: Pubkey,
    pub auto_release_at: i64,
    /// Product-flow tag (see `state::escrow_kind`).
    pub escrow_kind: u8,
    /// Optional terms/invoice hash (zero = none).
    pub terms_hash: [u8; 32],
}

/// Emitted when funds are deposited into escrow
#[event]
pub struct EscrowFunded {
    pub escrow_id: [u8; 32],
    pub amount: u64,
    pub commission_amount: u64,
    pub total_deposited: u64,
    pub is_native: bool,
    pub token_mint: Pubkey,
}

/// Emitted when a party confirms work completion
#[event]
pub struct CompletionConfirmed {
    pub escrow_id: [u8; 32],
    pub confirmer: Pubkey,
    pub employer_confirmed: bool,
    pub employee_confirmed: bool,
}

/// Emitted when funds (full or partial) are released from escrow
#[event]
pub struct EscrowReleased {
    pub escrow_id: [u8; 32],
    pub recipient: Pubkey,
    pub amount: u64,
    pub commission_amount: u64,
    /// Treasury that received the commission.
    pub commission_recipient: Pubkey,
    pub is_native: bool,
    pub token_mint: Pubkey,
    pub initiator: Pubkey,
    pub is_partial: bool,
    pub remaining_worker_amount: u64,
    /// Off-chain reference (e.g. invoice / worklog-batch id) so a single hire
    /// with many partial draw-downs can be reconciled. Zero = none.
    pub ref_id: [u8; 32],
}

/// Emitted when a dispute is raised
#[event]
pub struct DisputeRaised {
    pub escrow_id: [u8; 32],
    pub raised_by: Pubkey,
    pub raised_at: i64,
    pub dispute_deadline: i64,
}

/// Emitted when a dispute is resolved
#[event]
pub struct DisputeResolved {
    pub escrow_id: [u8; 32],
    pub resolver: Pubkey,
    pub employee_share: u64,
    pub employer_share: u64,
    pub commission_refunded: u64,
    pub is_native: bool,
    pub token_mint: Pubkey,
    pub forced: bool,
}

/// Emitted when an escrow is cancelled
#[event]
pub struct EscrowCancelled {
    pub escrow_id: [u8; 32],
    pub cancelled_by: Pubkey,
    pub refunded_to: Pubkey,
    /// Actual worker amount refunded (remaining, after any partial releases).
    pub amount_refunded: u64,
    /// Actual commission refunded (remaining, after any partial releases).
    pub commission_refunded: u64,
    pub is_native: bool,
    pub token_mint: Pubkey,
}

/// Emitted when the platform authority for an escrow is rotated
#[event]
pub struct PlatformAuthorityRotated {
    pub escrow_id: [u8; 32],
    pub old_authority: Pubkey,
    pub new_authority: Pubkey,
}

/// Emitted by `pay_with_commission_{sol,token}` (non-escrow direct pay). No
/// on-chain state persists for direct payments, so indexers rely entirely on
/// this event. Fee-on-top: `total = worker_amount + commission_amount`.
#[event]
pub struct DirectPaymentMade {
    /// Worqen hire / invoice reference (off-chain id). Doubles as ref_id.
    pub hire_id: [u8; 32],
    pub payer: Pubkey,
    pub recipient: Pubkey,
    /// Treasury that received the commission.
    pub fee_recipient: Pubkey,
    /// Total moved out of payer's wallet (worker_amount + commission_amount).
    pub total: u64,
    /// Amount received by recipient (in full).
    pub worker_amount: u64,
    /// Amount received by fee_recipient.
    pub commission_amount: u64,
    pub commission_bps: u16,
    pub is_native: bool,
    /// SPL mint if `is_native == false`, `Pubkey::default()` otherwise.
    pub token_mint: Pubkey,
    /// Unix timestamp from `Clock::get()`.
    pub paid_at: i64,
}

/// Emitted when the global Config is created or updated.
#[event]
pub struct ConfigUpdated {
    pub authority: Pubkey,
    pub pending_authority: Pubkey,
    pub fee_recipient: Pubkey,
    pub default_commission_bps: u16,
    pub paused: bool,
}

/// Emitted when a mint is added to / removed from the allowlist.
#[event]
pub struct MintAllowlistChanged {
    pub mint: Pubkey,
    pub added: bool,
}

/// Emitted when an existing funded escrow is topped up (retainer/hourly).
#[event]
pub struct EscrowToppedUp {
    pub escrow_id: [u8; 32],
    pub additional_amount: u64,
    pub additional_commission: u64,
    pub new_amount: u64,
    pub new_commission_amount: u64,
    pub total_added: u64,
    pub is_native: bool,
    pub token_mint: Pubkey,
}

/// Emitted by `batch_pay_with_commission_{sol,token}` — one atomic direct
/// payment split across many recipients (teams/referrals). Per-recipient
/// amounts are in the instruction args; this carries the totals.
#[event]
pub struct BatchPaymentMade {
    pub hire_id: [u8; 32],
    pub payer: Pubkey,
    pub fee_recipient: Pubkey,
    pub recipient_count: u8,
    pub total_worker_amount: u64,
    pub commission_amount: u64,
    pub commission_bps: u16,
    pub is_native: bool,
    pub token_mint: Pubkey,
    pub paid_at: i64,
}

/// Emitted when employer + employee amicably settle a funded escrow without a
/// dispute (`mutual_cancel_{sol,token}`).
#[event]
pub struct EscrowSettled {
    pub escrow_id: [u8; 32],
    pub employee_share: u64,
    pub employer_share: u64,
    pub is_native: bool,
    pub token_mint: Pubkey,
}

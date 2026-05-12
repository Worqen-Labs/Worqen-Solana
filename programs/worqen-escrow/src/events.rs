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
    pub amount: u64,
    pub commission_amount: u64,
    pub commission_rate_bps: u16,
    pub is_native: bool,
    pub token_mint: Pubkey,
    pub auto_release_at: i64,
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
    pub commission_recipient: Pubkey,
    pub is_native: bool,
    pub token_mint: Pubkey,
    pub initiator: Pubkey,
    pub is_partial: bool,
    pub remaining_worker_amount: u64,
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
    pub amount_refunded: u64,
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

/// Emitted by `pay_with_commission_{sol,token}` — the non-escrow direct-pay
/// path. No state is persisted on-chain for direct payments, so indexers
/// rely entirely on this event to attribute amounts to Worqen hires.
#[event]
pub struct DirectPaymentMade {
    /// Worqen hire id (application-owned). Typically SHA256(hire_id_uuid).
    pub hire_id: [u8; 32],
    pub payer: Pubkey,
    pub recipient: Pubkey,
    pub platform_authority: Pubkey,
    /// Total moved out of payer's wallet (worker_amount + commission_amount).
    pub amount: u64,
    /// Amount received by recipient.
    pub worker_amount: u64,
    /// Amount received by platform_authority.
    pub commission_amount: u64,
    pub commission_bps: u16,
    pub is_native: bool,
    /// SPL mint if `is_native == false`, `Pubkey::default()` otherwise.
    pub token_mint: Pubkey,
    /// Unix timestamp from `Clock::get()`.
    pub paid_at: i64,
}

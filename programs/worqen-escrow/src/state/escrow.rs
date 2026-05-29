use anchor_lang::prelude::*;

/// Escrow status enum representing the lifecycle of an escrow
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EscrowStatus {
    /// Escrow created, awaiting deposit
    #[default]
    Created = 0,
    /// Funds deposited and locked
    Funded = 1,
    /// Work done, awaiting release confirmation
    PendingRelease = 2,
    /// Funds released to employee
    Released = 3,
    /// Dispute raised, funds frozen
    Disputed = 4,
    /// Dispute resolved, funds distributed
    Resolved = 5,
    /// Escrow cancelled, funds refunded
    Cancelled = 6,
}

/// Escrow "kind" tag — lets the same account schema serve different product
/// flows and lets the indexer/UI classify an escrow without off-chain joins.
/// Stored as a `u8` (not a closed enum) so new kinds can be introduced in a
/// later program upgrade without an account-schema migration.
pub mod escrow_kind {
    /// Fixed-price or per-milestone deliverable escrow (default).
    pub const MILESTONE: u8 = 0;
    /// Hourly / capped-prepaid block drawn down with `release_partial_*`.
    pub const HOURLY: u8 = 1;
    /// Retainer / ongoing engagement.
    pub const RETAINER: u8 = 2;
    /// Anything else; classify off-chain.
    pub const OTHER: u8 = 255;
}

/// Account schema version. Bump when adding/removing/reordering fields so
/// off-chain readers can detect mismatches deterministically.
pub const ESCROW_ACCOUNT_VERSION: u8 = 1;

/// Main escrow account structure.
///
/// Layout-changing fields MUST be added at the end and the version bumped.
/// `reserved` exists so a future upgrade can carve new fields onto accounts
/// that are already open without a realloc.
#[account]
pub struct Escrow {
    /// Account schema version
    pub version: u8,

    /// Unique identifier (random 32 bytes from the backend) - 32 bytes.
    /// Used as the escrow PDA seed.
    pub escrow_id: [u8; 32],

    /// Links related milestone escrows for one hire (off-chain SHA256 of
    /// hire_id). Zero-bytes means "ungrouped".
    pub escrow_group_id: [u8; 32],

    /// Sequence number within the group (1-indexed). 0 if ungrouped.
    pub sequence_in_group: u8,

    /// Total milestones in the group. 0 if ungrouped.
    pub total_in_group: u8,

    /// Employer's wallet address - 32 bytes
    pub employer: Pubkey,

    /// Employee's wallet address - 32 bytes
    pub employee: Pubkey,

    /// Platform authority (can resolve disputes, trigger auto-release,
    /// rotate keys). This is the SIGNING/ops key only — it does NOT receive
    /// commission (see `fee_recipient`). 32 bytes.
    pub platform_authority: Pubkey,

    /// Worker payment amount in lamports/token units - 8 bytes.
    /// This is the amount the employee receives in full (fee is on top).
    pub amount: u64,

    /// Platform commission amount in lamports/token units - 8 bytes.
    /// Commission = amount * commission_rate_bps / 10000 (fee on top of
    /// `amount`; the employer deposits `amount + commission_amount`).
    pub commission_amount: u64,

    /// Commission rate in basis points (500 = 5% standard, 150 = Prime,
    /// 200 = tip). - 2 bytes
    pub commission_rate_bps: u16,

    /// Cumulative amount released to employee via partial releases.
    /// Used by `release_partial_*` and full release / dispute math.
    pub released_to_employee: u64,

    /// Token mint (SystemProgram ID for SOL) - 32 bytes
    pub token_mint: Pubkey,

    /// True = SOL, False = SPL Token - 1 byte
    pub is_native: bool,

    /// Current escrow status - 1 byte
    pub status: EscrowStatus,

    /// Employer confirmed completion - 1 byte
    pub employer_confirmed: bool,

    /// Employee confirmed completion - 1 byte
    pub employee_confirmed: bool,

    /// Unix timestamp of creation - 8 bytes
    pub created_at: i64,

    /// Unix timestamp of funding - 8 bytes
    pub funded_at: i64,

    /// Unix timestamp of completion (release/resolve/cancel) - 8 bytes
    pub completed_at: i64,

    /// Unix timestamp after which anyone may trigger auto-release to the
    /// employee. 0 = disabled. Reserved for a future instruction; not read in v1.
    pub auto_release_at: i64,

    /// Who initiated release - 32 bytes
    pub release_initiator: Pubkey,

    /// UTF-8 encoded dispute reason - 256 bytes
    pub dispute_reason: [u8; 256],

    /// Who raised the dispute - 32 bytes (Pubkey::default if no dispute)
    pub dispute_raised_by: Pubkey,

    /// Unix timestamp the dispute was raised - 8 bytes (0 if no dispute)
    pub dispute_raised_at: i64,

    /// Unix timestamp after which the dispute can be force-resolved by
    /// anyone via `trigger_auto_release_*`. Bounded to
    /// [now + MIN_DISPUTE_DEADLINE_DURATION, now + MAX_DISPUTE_DEADLINE_DURATION].
    pub dispute_deadline: i64,

    /// Who resolved the dispute - 32 bytes (Pubkey::default if unresolved)
    pub dispute_resolved_by: Pubkey,

    /// Unix timestamp the dispute was resolved - 8 bytes (0 if unresolved)
    pub dispute_resolved_at: i64,

    /// Amount sent to employee in dispute resolution. 0 if not resolved.
    pub employee_share_resolved: u64,

    /// Amount sent to employer in dispute resolution (worker share only,
    /// excludes commission refund). 0 if not resolved.
    pub employer_share_resolved: u64,

    /// UTF-8 encoded cancellation reason - 128 bytes
    pub cancellation_reason: [u8; 128],

    /// Who cancelled the escrow - 32 bytes (Pubkey::default if not cancelled)
    pub cancelled_by: Pubkey,

    /// PDA bump seed - 1 byte
    pub bump: u8,

    /// Vault PDA bump seed - 1 byte
    pub vault_bump: u8,

    // ---- v1 additions (kept at the end of the layout) ----
    /// Product-flow tag (see `escrow_kind`). - 1 byte
    pub escrow_kind: u8,

    /// Commission destination (platform treasury). Snapshotted from
    /// `Config.fee_recipient` at create so a later Config change never
    /// re-routes an in-flight escrow. - 32 bytes
    pub fee_recipient: Pubkey,

    /// Optional tamper-evident hash of the agreed terms / approved invoice
    /// for dispute evidence. Zero = none. - 32 bytes
    pub terms_hash: [u8; 32],

    /// Reserved padding for forward-compatible field additions without a
    /// realloc on already-open accounts. - 64 bytes
    pub reserved: [u8; 64],
}

impl Escrow {
    /// Default / standard commission rate: 5% = 500 basis points.
    pub const DEFAULT_COMMISSION_RATE_BPS: u16 = 500;

    /// Prime-subscription employer commission rate: 1.5% = 150 bps.
    /// Informational — the actual bps is passed per call by the backend.
    pub const PRIME_COMMISSION_RATE_BPS: u16 = 150;

    /// Tip commission rate: 2% = 200 bps. Informational.
    pub const TIP_COMMISSION_RATE_BPS: u16 = 200;

    /// Maximum commission rate: 10% = 1000 basis points (hard cap).
    pub const MAX_COMMISSION_RATE_BPS: u16 = 1000;

    /// Maximum dispute reason length in bytes
    pub const MAX_DISPUTE_REASON_LEN: usize = 256;

    /// Maximum cancellation reason length in bytes
    pub const MAX_CANCELLATION_REASON_LEN: usize = 128;

    /// Minimum dispute window from raise time, in seconds. 3 days.
    /// Guarantees the platform always has time to mediate before anyone can
    /// permissionlessly force-resolve — closes the instant-self-payout hole.
    pub const MIN_DISPUTE_DEADLINE_DURATION: i64 = 3 * 24 * 60 * 60;

    /// Maximum dispute deadline duration from raise time, in seconds.
    /// 90 days. Bounds platform-failure exposure.
    pub const MAX_DISPUTE_DEADLINE_DURATION: i64 = 90 * 24 * 60 * 60;

    /// Maximum auto_release_at duration from create time, in seconds. 1 year.
    pub const MAX_AUTO_RELEASE_DURATION: i64 = 365 * 24 * 60 * 60;

    /// Account discriminator (8) + sum of every field above. Recompute when
    /// fields change. Off-chain readers should use this constant — never
    /// hardcode a number.
    pub const SPACE: usize = 8       // discriminator
        + 1                          // version
        + 32                         // escrow_id
        + 32                         // escrow_group_id
        + 1                          // sequence_in_group
        + 1                          // total_in_group
        + 32                         // employer
        + 32                         // employee
        + 32                         // platform_authority
        + 8                          // amount
        + 8                          // commission_amount
        + 2                          // commission_rate_bps
        + 8                          // released_to_employee
        + 32                         // token_mint
        + 1                          // is_native
        + 1                          // status
        + 1                          // employer_confirmed
        + 1                          // employee_confirmed
        + 8                          // created_at
        + 8                          // funded_at
        + 8                          // completed_at
        + 8                          // auto_release_at
        + 32                         // release_initiator
        + 256                        // dispute_reason
        + 32                         // dispute_raised_by
        + 8                          // dispute_raised_at
        + 8                          // dispute_deadline
        + 32                         // dispute_resolved_by
        + 8                          // dispute_resolved_at
        + 8                          // employee_share_resolved
        + 8                          // employer_share_resolved
        + 128                        // cancellation_reason
        + 32                         // cancelled_by
        + 1                          // bump
        + 1                          // vault_bump
        + 1                          // escrow_kind
        + 32                         // fee_recipient
        + 32                         // terms_hash
        + 64; // reserved

    /// Seed prefix for escrow PDA
    pub const ESCROW_SEED: &'static [u8] = b"escrow";

    /// Seed prefix for vault PDA
    pub const VAULT_SEED: &'static [u8] = b"vault";

    /// Calculate commission from amount and rate (floor; u128 intermediate)
    pub fn calculate_commission(amount: u64, rate_bps: u16) -> u64 {
        ((amount as u128) * (rate_bps as u128) / 10000) as u64
    }

    /// Get total deposit required (worker amount + commission)
    pub fn total_deposit(&self) -> Result<u64> {
        self.amount
            .checked_add(self.commission_amount)
            .ok_or(crate::errors::EscrowError::InvalidAmount.into())
    }

    /// Worker amount remaining for release/dispute after partial releases
    pub fn remaining_worker_amount(&self) -> u64 {
        self.amount.saturating_sub(self.released_to_employee)
    }

    /// Commission remaining to be paid to platform after partial releases
    pub fn remaining_commission(&self) -> u64 {
        let already_paid =
            Escrow::calculate_commission(self.released_to_employee, self.commission_rate_bps);
        self.commission_amount.saturating_sub(already_paid)
    }

    /// True if status is terminal (no further state transitions). Used by
    /// `close_escrow_*` to gate rent recovery.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            EscrowStatus::Released | EscrowStatus::Resolved | EscrowStatus::Cancelled
        )
    }
}

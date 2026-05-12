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

/// Account schema version. Bump when adding/removing/reordering fields so
/// off-chain readers can detect mismatches deterministically.
///
/// **v2** — same schema as v1, but several semantics changed: employer can no
/// longer cancel a Funded escrow, `auto_release_at` is no longer used to
/// trigger releases from `Funded`/`PendingRelease`, `dispute_deadline` must
/// be set and bounded, worker can self-release once both parties confirmed,
/// and a new `close_escrow_*` instruction reclaims rent on terminal escrows.
pub const ESCROW_ACCOUNT_VERSION: u8 = 2;

/// Main escrow account structure (v1).
///
/// Layout-changing fields MUST be added at the end and the version bumped.
#[account]
pub struct Escrow {
    /// Account schema version
    pub version: u8,

    /// Unique identifier (SHA256 of hire_id) - 32 bytes
    pub escrow_id: [u8; 32],

    /// Group identifier linking related milestone escrows for the same
    /// hire. Off-chain SHA256 of hire_id (so all milestones of one hire
    /// share the same escrow_group_id even though each has a unique
    /// escrow_id). Zero-bytes means "ungrouped".
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
    /// rotate keys). 32 bytes.
    pub platform_authority: Pubkey,

    /// Worker payment amount in lamports/token units - 8 bytes
    pub amount: u64,

    /// Platform commission amount in lamports/token units - 8 bytes
    /// Commission = amount * commission_rate_bps / 10000
    pub commission_amount: u64,

    /// Commission rate in basis points (150 = 1.5%) - 2 bytes
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

    /// Unix timestamp after which anyone can trigger auto-release to
    /// employee. 0 = auto-release disabled. Configured at create time.
    pub auto_release_at: i64,

    /// Who initiated release - 32 bytes
    pub release_initiator: Pubkey,

    /// UTF-8 encoded dispute reason - 256 bytes
    pub dispute_reason: [u8; 256],

    /// Who raised the dispute - 32 bytes (Pubkey::default if no dispute)
    pub dispute_raised_by: Pubkey,

    /// Unix timestamp the dispute was raised - 8 bytes (0 if no dispute)
    pub dispute_raised_at: i64,

    /// Unix timestamp after which the dispute can be force-resolved by the
    /// platform_authority via `trigger_auto_release_*`. 0 = no deadline.
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
}

impl Escrow {
    /// Default commission rate: 0% = 0 basis points (no commission by default)
    pub const DEFAULT_COMMISSION_RATE_BPS: u16 = 0;

    /// Maximum commission rate: 10% = 1000 basis points
    pub const MAX_COMMISSION_RATE_BPS: u16 = 1000;

    /// Maximum dispute reason length in bytes
    pub const MAX_DISPUTE_REASON_LEN: usize = 256;

    /// Maximum cancellation reason length in bytes
    pub const MAX_CANCELLATION_REASON_LEN: usize = 128;

    /// Maximum dispute deadline duration from raise time, in seconds.
    /// 90 days. Bounds platform-failure exposure.
    pub const MAX_DISPUTE_DEADLINE_DURATION: i64 = 90 * 24 * 60 * 60;

    /// Maximum auto_release_at duration from create time, in seconds.
    /// 1 year. Bounds escrow lifetime so funds aren't locked indefinitely
    /// by a far-future deadline.
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
        + 1; // vault_bump

    /// Seed prefix for escrow PDA
    pub const ESCROW_SEED: &'static [u8] = b"escrow";

    /// Seed prefix for vault PDA
    pub const VAULT_SEED: &'static [u8] = b"vault";

    /// Calculate commission from amount and rate
    pub fn calculate_commission(amount: u64, rate_bps: u16) -> u64 {
        ((amount as u128) * (rate_bps as u128) / 10000) as u64
    }

    /// Get total deposit required (worker amount + commission)
    pub fn total_deposit(&self) -> u64 {
        self.amount.checked_add(self.commission_amount).unwrap_or(u64::MAX)
    }

    /// Worker amount remaining for release/dispute after partial releases
    pub fn remaining_worker_amount(&self) -> u64 {
        self.amount.saturating_sub(self.released_to_employee)
    }

    /// Commission remaining to be paid to platform after partial releases
    pub fn remaining_commission(&self) -> u64 {
        let already_paid = Escrow::calculate_commission(
            self.released_to_employee,
            self.commission_rate_bps,
        );
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

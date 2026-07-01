use anchor_lang::prelude::*;

pub const HOURLY_PERIOD_VERSION: u8 = 1;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HourlyStatus {
    #[default]
    Open = 0,
    Funded = 1,
    Active = 2,
    Settling = 3,
    Closed = 4,
    Refunded = 5,
    Cancelled = 6,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TrancheStatus {
    #[default]
    Empty = 0,
    Frozen = 1,
    Disputed = 2,
    Finalized = 3,
    Resolved = 4,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Tranche {
    pub amount: u64,
    pub commission: u64,
    pub staged_at: i64,
    pub release_at: i64,
    pub dispute_deadline: i64,
    pub status: TrancheStatus,
}

#[account]
pub struct HourlyPeriod {
    pub version: u8,
    pub hire_id: [u8; 32],
    pub period_index: u32,
    pub employer: Pubkey,
    pub employee: Pubkey,
    pub platform_authority: Pubkey,
    pub fee_recipient: Pubkey,
    pub token_mint: Pubkey,
    pub bump: u8,
    pub vault_bump: u8,
    pub weekly_cap_net: u64,
    pub commission_rate_bps: u16,
    pub funded_amount: u64,
    pub released_net: u64,
    pub total_staged_net: u64,
    pub tranches: [Tranche; 7],
    pub tranche_count: u8,
    pub review_window_secs: i64,
    pub created_at: i64,
    pub funded_at: i64,
    pub period_end_at: i64,
    pub completed_at: i64,
    pub status: HourlyStatus,
    pub reserved: [u8; 64],
}

impl HourlyPeriod {
    pub const SPACE: usize = 8
        + 1
        + 32
        + 4
        + 32
        + 32
        + 32
        + 32
        + 32
        + 1
        + 1
        + 8
        + 2
        + 8
        + 8
        + 8
        + (41 * 7)
        + 1
        + 8
        + 8
        + 8
        + 8
        + 8
        + 1
        + 64;

    pub const HOURLY_SEED: &'static [u8] = b"hourly";
    pub const DELEGATE_AUTH_SEED: &'static [u8] = b"delegate_auth";
    pub const MAX_TRANCHES: usize = 7;
    pub const DEFAULT_REVIEW_WINDOW_SECS: i64 = 7 * 24 * 60 * 60;
    pub const MAX_REVIEW_WINDOW_SECS: i64 = 30 * 24 * 60 * 60;

    pub fn live_liabilities(&self) -> Option<u64> {
        let mut total: u64 = 0;
        for t in self.tranches.iter() {
            if matches!(t.status, TrancheStatus::Frozen | TrancheStatus::Disputed) {
                total = total.checked_add(t.amount)?.checked_add(t.commission)?;
            }
        }
        Some(total)
    }

    pub fn has_live_tranche(&self) -> bool {
        self.tranches
            .iter()
            .any(|t| matches!(t.status, TrancheStatus::Frozen | TrancheStatus::Disputed))
    }
}

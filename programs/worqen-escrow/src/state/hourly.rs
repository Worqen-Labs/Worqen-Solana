use anchor_lang::prelude::*;

pub const HOURLY_PERIOD_VERSION: u8 = 2;
pub const HOURLY_INVOICE_VERSION: u8 = 1;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[borsh(use_discriminant = true)]
pub enum HourlyStatus {
    #[default]
    Open = 0,
    Funded = 1,
    Active = 2,
    Settled = 3,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[borsh(use_discriminant = true)]
pub enum InvoiceStatus {
    #[default]
    Staged = 0,
    Disputed = 1,
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
    pub is_native: bool,
    pub bump: u8,
    pub vault_bump: u8,
    pub rent_payer: Pubkey,
    pub weekly_cap_net: u64,
    pub commission_rate_bps: u16,
    pub funded_gross: u64,
    pub total_staged_net: u64,
    pub released_net: u64,
    pub refunded_amount: u64,
    pub outstanding_net: u64,
    pub outstanding_commission: u64,
    pub vault_rent_reserve: u64,
    pub invoice_count: u16,
    pub live_invoices: u16,
    pub review_window_secs: i64,
    pub period_start_at: i64,
    pub period_end_at: i64,
    pub created_at: i64,
    pub funded_at: i64,
    pub settled_at: i64,
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
        + 1
        + 32
        + 8
        + 2
        + 8
        + 8
        + 8
        + 8
        + 8
        + 8
        + 8
        + 2
        + 2
        + 8
        + 8
        + 8
        + 8
        + 8
        + 8
        + 1
        + 64;

    pub const HOURLY_SEED: &'static [u8] = b"hourly";
    pub const MAX_REVIEW_WINDOW_SECS: i64 = 30 * 24 * 60 * 60;
    pub const MIN_PERIOD_DURATION_SECS: i64 = 24 * 60 * 60;
    pub const MAX_PERIOD_DURATION_SECS: i64 = 30 * 24 * 60 * 60;

    pub fn outstanding_total(&self) -> Option<u64> {
        self.outstanding_net
            .checked_add(self.outstanding_commission)
    }

    pub fn cap_gross(&self) -> Option<u64> {
        let commission = crate::state::Escrow::calculate_commission(
            self.weekly_cap_net,
            self.commission_rate_bps,
        );
        self.weekly_cap_net.checked_add(commission)
    }

    pub fn marginal_commission(&self, amount: u64) -> Option<u64> {
        let new_total = self.total_staged_net.checked_add(amount)?;
        let cum_before = crate::state::Escrow::calculate_commission(
            self.total_staged_net,
            self.commission_rate_bps,
        );
        let cum_after =
            crate::state::Escrow::calculate_commission(new_total, self.commission_rate_bps);
        Some(cum_after.saturating_sub(cum_before))
    }

    pub fn register_staged(&mut self, amount: u64, commission: u64) -> Option<()> {
        self.total_staged_net = self.total_staged_net.checked_add(amount)?;
        self.outstanding_net = self.outstanding_net.checked_add(amount)?;
        self.outstanding_commission = self.outstanding_commission.checked_add(commission)?;
        self.invoice_count = self.invoice_count.checked_add(1)?;
        self.live_invoices = self.live_invoices.checked_add(1)?;
        Some(())
    }

    pub fn register_settled_invoice(
        &mut self,
        amount: u64,
        commission: u64,
        released_to_worker: u64,
    ) -> Option<()> {
        self.outstanding_net = self.outstanding_net.checked_sub(amount)?;
        self.outstanding_commission = self.outstanding_commission.checked_sub(commission)?;
        self.live_invoices = self.live_invoices.checked_sub(1)?;
        self.released_net = self.released_net.checked_add(released_to_worker)?;
        Some(())
    }
}

#[account]
pub struct HourlyInvoice {
    pub version: u8,
    pub period: Pubkey,
    pub invoice_index: u16,
    pub ref_id: [u8; 32],
    pub amount_net: u64,
    pub commission: u64,
    pub staged_at: i64,
    pub release_at: i64,
    pub dispute_deadline: i64,
    pub disputed_by: Pubkey,
    pub status: InvoiceStatus,
    pub rent_payer: Pubkey,
    pub bump: u8,
    pub reserved: [u8; 32],
}

impl HourlyInvoice {
    pub const SPACE: usize = 8 + 1 + 32 + 2 + 32 + 8 + 8 + 8 + 8 + 8 + 32 + 1 + 32 + 1 + 32;

    pub const INVOICE_SEED: &'static [u8] = b"hourly_inv";
}

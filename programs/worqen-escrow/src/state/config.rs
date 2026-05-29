use anchor_lang::prelude::*;

/// Seed for the singleton global config PDA.
pub const CONFIG_SEED: &[u8] = b"config";

/// Maximum number of SPL mints in the allowlist. Native SOL is always
/// allowed and is NOT stored here.
pub const MAX_ALLOWED_MINTS: usize = 30;

/// Current config schema version.
pub const CONFIG_VERSION: u8 = 1;

/// Singleton program configuration at PDA `[CONFIG_SEED]`: mint allowlist,
/// pause kill-switch, commission treasury, and default commission rate.
///
/// Pausing only blocks *new* money entering the system (create/deposit/
/// direct-pay); it can never block release, dispute, auto-release or close,
/// so a pause can never strand user funds.
#[account]
pub struct Config {
    /// Schema version.
    pub version: u8,

    /// Admin authority (a multisig on mainnet). May update config and
    /// propose an authority handoff.
    pub authority: Pubkey,

    /// Pending admin during a two-step authority handoff. `Pubkey::default()`
    /// when no handoff is in progress.
    pub pending_authority: Pubkey,

    /// Treasury wallet that receives platform commission. Distinct from any
    /// escrow's `platform_authority` signing key. Snapshotted onto each
    /// escrow at create time.
    pub fee_recipient: Pubkey,

    /// Default commission rate in bps (5% = 500). Informational default; the
    /// backend passes the effective tier (500 / 150 Prime / 200 tip) per call.
    pub default_commission_bps: u16,

    /// Global pause switch. When true, create_escrow / deposit_* /
    /// pay_with_commission_* are blocked. Releases/disputes/closes are not.
    pub paused: bool,

    /// Allowlist of SPL token mints permitted for escrow and direct-pay.
    /// Native SOL is always allowed and is not listed here.
    pub allowed_mints: Vec<Pubkey>,

    /// PDA bump.
    pub bump: u8,

    /// Reserved padding for forward-compatible additions.
    pub reserved: [u8; 64],
}

impl Config {
    pub const SPACE: usize = 8 // discriminator
        + 1                    // version
        + 32                   // authority
        + 32                   // pending_authority
        + 32                   // fee_recipient
        + 2                    // default_commission_bps
        + 1                    // paused
        + 4 + (MAX_ALLOWED_MINTS * 32) // allowed_mints (len prefix + max elems)
        + 1                    // bump
        + 64; // reserved

    /// True if a given mint is permitted. Native SOL is always allowed.
    pub fn is_mint_allowed(&self, mint: &Pubkey, is_native: bool) -> bool {
        if is_native {
            return true;
        }
        self.allowed_mints.iter().any(|m| m == mint)
    }
}

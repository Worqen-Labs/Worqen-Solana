use crate::errors::EscrowError;
use crate::events::{ConfigUpdated, MintAllowlistChanged};
use crate::state::{Config, Escrow, CONFIG_SEED, CONFIG_VERSION, MAX_ALLOWED_MINTS};
use anchor_lang::prelude::*;

/// Create the singleton global Config PDA. The signer becomes the admin
/// authority and pays the account rent.
#[derive(Accounts)]
pub struct InitConfig<'info> {
    #[account(
        init,
        payer = authority,
        space = Config::SPACE,
        seeds = [CONFIG_SEED],
        bump
    )]
    pub config: Account<'info, Config>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn init_config(
    ctx: Context<InitConfig>,
    fee_recipient: Pubkey,
    default_commission_bps: u16,
    allowed_mints: Vec<Pubkey>,
) -> Result<()> {
    require!(
        default_commission_bps <= Escrow::MAX_COMMISSION_RATE_BPS,
        EscrowError::InvalidCommissionRate
    );
    require!(
        allowed_mints.len() <= MAX_ALLOWED_MINTS,
        EscrowError::MintAllowlistFull
    );
    require!(
        fee_recipient != Pubkey::default(),
        EscrowError::InvalidFeeRecipient
    );

    let config = &mut ctx.accounts.config;
    config.version = CONFIG_VERSION;
    config.authority = ctx.accounts.authority.key();
    config.pending_authority = Pubkey::default();
    config.fee_recipient = fee_recipient;
    config.default_commission_bps = default_commission_bps;
    config.paused = false;
    config.allowed_mints = allowed_mints;
    config.bump = ctx.bumps.config;
    config.reserved = [0u8; 64];

    emit!(ConfigUpdated {
        authority: config.authority,
        pending_authority: config.pending_authority,
        fee_recipient: config.fee_recipient,
        default_commission_bps: config.default_commission_bps,
        paused: config.paused,
    });
    Ok(())
}

/// Update mutable Config fields. Any `None` argument is left unchanged.
/// Setting `new_pending_authority` begins a two-step authority handoff
/// (completed by `accept_authority`).
#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ EscrowError::Unauthorized,
    )]
    pub config: Account<'info, Config>,

    pub authority: Signer<'info>,
}

pub fn update_config(
    ctx: Context<UpdateConfig>,
    new_fee_recipient: Option<Pubkey>,
    new_default_commission_bps: Option<u16>,
    new_paused: Option<bool>,
    new_pending_authority: Option<Pubkey>,
) -> Result<()> {
    let config = &mut ctx.accounts.config;

    if let Some(fr) = new_fee_recipient {
        require!(fr != Pubkey::default(), EscrowError::InvalidFeeRecipient);
        config.fee_recipient = fr;
    }
    if let Some(bps) = new_default_commission_bps {
        require!(
            bps <= Escrow::MAX_COMMISSION_RATE_BPS,
            EscrowError::InvalidCommissionRate
        );
        config.default_commission_bps = bps;
    }
    if let Some(p) = new_paused {
        config.paused = p;
    }
    if let Some(pa) = new_pending_authority {
        config.pending_authority = pa;
    }

    emit!(ConfigUpdated {
        authority: config.authority,
        pending_authority: config.pending_authority,
        fee_recipient: config.fee_recipient,
        default_commission_bps: config.default_commission_bps,
        paused: config.paused,
    });
    Ok(())
}

/// Complete a two-step authority handoff. The pending authority signs.
#[derive(Accounts)]
pub struct AcceptAuthority<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
    )]
    pub config: Account<'info, Config>,

    pub pending_authority: Signer<'info>,
}

pub fn accept_authority(ctx: Context<AcceptAuthority>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    require!(
        config.pending_authority != Pubkey::default(),
        EscrowError::NoPendingAuthority
    );
    require!(
        ctx.accounts.pending_authority.key() == config.pending_authority,
        EscrowError::PendingAuthorityMismatch
    );
    config.authority = config.pending_authority;
    config.pending_authority = Pubkey::default();

    emit!(ConfigUpdated {
        authority: config.authority,
        pending_authority: config.pending_authority,
        fee_recipient: config.fee_recipient,
        default_commission_bps: config.default_commission_bps,
        paused: config.paused,
    });
    Ok(())
}

/// Add or remove a mint from the allowlist (authority signs).
#[derive(Accounts)]
pub struct UpdateAllowlist<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = authority @ EscrowError::Unauthorized,
    )]
    pub config: Account<'info, Config>,

    pub authority: Signer<'info>,
}

pub fn add_allowed_mint(ctx: Context<UpdateAllowlist>, mint: Pubkey) -> Result<()> {
    require!(mint != Pubkey::default(), EscrowError::MintNotAllowed);
    let config = &mut ctx.accounts.config;
    require!(
        !config.allowed_mints.iter().any(|m| m == &mint),
        EscrowError::MintAllowlistFull
    );
    require!(
        config.allowed_mints.len() < MAX_ALLOWED_MINTS,
        EscrowError::MintAllowlistFull
    );
    config.allowed_mints.push(mint);
    emit!(MintAllowlistChanged { mint, added: true });
    Ok(())
}

pub fn remove_allowed_mint(ctx: Context<UpdateAllowlist>, mint: Pubkey) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let before = config.allowed_mints.len();
    config.allowed_mints.retain(|m| m != &mint);
    require!(
        config.allowed_mints.len() < before,
        EscrowError::MintNotAllowed
    );
    emit!(MintAllowlistChanged { mint, added: false });
    Ok(())
}

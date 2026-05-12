use crate::errors::EscrowError;
use crate::events::PlatformAuthorityRotated;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;

/// Rotate the escrow's `platform_authority` key.
///
/// Signed by the *current* platform_authority. Intended for key rotation
/// after suspected compromise or planned ops changes. Can only be called
/// while the escrow is in an active state (not Released / Resolved /
/// Cancelled) — once finalized the authority no longer matters.
///
/// **v2 change:** rotation is also blocked while the escrow is `Disputed`.
/// A compromised authority that rotates mid-dispute could resolve in their
/// own (or a colluder's) favor; locking rotation during dispute closes
/// that window.
///
/// The new authority must differ from employer and employee to preserve the
/// three-party model.
#[derive(Accounts)]
pub struct UpdatePlatformAuthority<'info> {
    #[account(
        mut,
        constraint = escrow.status != EscrowStatus::Released
            && escrow.status != EscrowStatus::Resolved
            && escrow.status != EscrowStatus::Cancelled
            @ EscrowError::InvalidStatus,
        constraint = escrow.status != EscrowStatus::Disputed @ EscrowError::AuthorityRotationDuringDispute,
        constraint = escrow.platform_authority == current_platform_authority.key() @ EscrowError::Unauthorized,
    )]
    pub escrow: Account<'info, Escrow>,

    pub current_platform_authority: Signer<'info>,

    /// CHECK: we only store this pubkey
    pub new_platform_authority: UncheckedAccount<'info>,
}

pub fn handler(ctx: Context<UpdatePlatformAuthority>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let new_auth = ctx.accounts.new_platform_authority.key();

    require!(
        new_auth != escrow.employer && new_auth != escrow.employee,
        EscrowError::InvalidNewPlatformAuthority
    );

    let old_auth = escrow.platform_authority;
    escrow.platform_authority = new_auth;

    emit!(PlatformAuthorityRotated {
        escrow_id: escrow.escrow_id,
        old_authority: old_auth,
        new_authority: new_auth,
    });

    msg!(
        "Platform authority rotated: {:?} -> {:?}",
        old_auth,
        new_auth
    );

    Ok(())
}

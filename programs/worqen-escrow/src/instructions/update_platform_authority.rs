use crate::errors::EscrowError;
use crate::events::PlatformAuthorityRotated;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;

/// Rotate the escrow's `platform_authority`, signed by the current authority.
///
/// Allowed only while the escrow is active. Blocked during `Disputed` so a
/// compromised authority cannot rotate mid-dispute and resolve in its own
/// favor. The new authority must differ from employer and employee.
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
    require!(
        new_auth != escrow.platform_authority,
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

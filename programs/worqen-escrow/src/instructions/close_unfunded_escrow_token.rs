use crate::errors::EscrowError;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;

/// Close a cancelled, never-funded token escrow, reclaiming its rent to the
/// employer. No vault ATA exists yet (created lazily on first `deposit_token`),
/// so only the escrow account is closed. Gated on `Cancelled && funded_at == 0`.
#[derive(Accounts)]
pub struct CloseUnfundedEscrowToken<'info> {
    #[account(
        mut,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
        constraint = escrow.status == EscrowStatus::Cancelled @ EscrowError::EscrowNotTerminal,
        constraint = escrow.funded_at == 0 @ EscrowError::EscrowWasFunded,
        close = employer,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    /// Receives the reclaimed account rent.
    /// CHECK: matched against escrow.employer
    #[account(
        mut,
        constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized,
    )]
    pub employer: UncheckedAccount<'info>,

    /// Employer or platform_authority may trigger the close.
    #[account(
        constraint = signer.key() == escrow.employer || signer.key() == escrow.platform_authority @ EscrowError::Unauthorized,
    )]
    pub signer: Signer<'info>,
}

pub fn handler(ctx: Context<CloseUnfundedEscrowToken>) -> Result<()> {
    msg!(
        "Unfunded token escrow {:?} closed; rent refunded to employer",
        ctx.accounts.escrow.escrow_id
    );
    Ok(())
}

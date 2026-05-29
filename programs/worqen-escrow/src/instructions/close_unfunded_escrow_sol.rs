use crate::errors::EscrowError;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;

/// Close a cancelled, never-funded SOL escrow and reclaim its rent to the
/// employer. Gated on `status == Cancelled && funded_at == 0` so it can never
/// touch a live or funded escrow (use `close_escrow_sol` for those).
#[derive(Accounts)]
pub struct CloseUnfundedEscrowSol<'info> {
    #[account(
        mut,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
        constraint = escrow.status == EscrowStatus::Cancelled @ EscrowError::EscrowNotTerminal,
        constraint = escrow.funded_at == 0 @ EscrowError::EscrowWasFunded,
        close = employer,
    )]
    pub escrow: Account<'info, Escrow>,

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

pub fn handler(ctx: Context<CloseUnfundedEscrowSol>) -> Result<()> {
    msg!(
        "Unfunded SOL escrow {:?} closed; rent refunded to employer",
        ctx.accounts.escrow.escrow_id
    );
    Ok(())
}

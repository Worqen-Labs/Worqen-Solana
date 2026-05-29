use crate::errors::EscrowError;
use crate::state::Escrow;
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Close a terminal SOL escrow, sweeping any vault dust and refunding rent to the employer.
///
/// Either employer or platform_authority may sign, so a cleanup worker can
/// reclaim rent on terminal escrows without requiring employer action.
#[derive(Accounts)]
pub struct CloseEscrowSol<'info> {
    #[account(
        mut,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
        constraint = escrow.is_terminal() @ EscrowError::EscrowNotTerminal,
        close = employer,
    )]
    pub escrow: Account<'info, Escrow>,

    /// SOL vault PDA — drained of any dust before close.
    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: PDA vault
    pub escrow_vault: UncheckedAccount<'info>,

    /// Receives both vault dust and the escrow account's rent.
    /// CHECK: matched against escrow.employer
    #[account(
        mut,
        constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized,
    )]
    pub employer: UncheckedAccount<'info>,

    /// Authorizes the close. Employer or platform_authority.
    #[account(
        constraint = signer.key() == escrow.employer || signer.key() == escrow.platform_authority @ EscrowError::Unauthorized,
    )]
    pub signer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<CloseEscrowSol>) -> Result<()> {
    let vault_balance = ctx.accounts.escrow_vault.lamports();
    if vault_balance > 0 {
        let escrow_key = ctx.accounts.escrow.key();
        let vault_seeds = &[
            Escrow::VAULT_SEED,
            escrow_key.as_ref(),
            &[ctx.accounts.escrow.vault_bump],
        ];
        let signer_seeds = &[&vault_seeds[..]];

        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.employer.to_account_info(),
                },
                signer_seeds,
            ),
            vault_balance,
        )?;
    }

    msg!(
        "Escrow {:?} closed; vault dust ({} lamports) and rent refunded to employer",
        ctx.accounts.escrow.escrow_id,
        vault_balance
    );

    Ok(())
}

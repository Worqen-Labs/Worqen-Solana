use crate::errors::EscrowError;
use crate::state::Escrow;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount, Transfer};

/// Close a terminal token escrow and refund all rent to the employer.
///
/// **Allowed status:** `Released` / `Resolved` / `Cancelled`.
///
/// This:
/// 1. Sweeps any residual tokens from the vault token account into the
///    employer's token account (covers stray transfers post-release).
/// 2. Closes the vault token account, refunding ~0.002 SOL of ATA rent
///    to the employer.
/// 3. Closes the escrow account, refunding ~0.005 SOL of account rent
///    to the employer.
///
/// Either employer or platform_authority may sign.
///
/// **Limitation:** if the escrow was cancelled in `Created` state (vault
/// was never deposited into and so the vault token account doesn't exist),
/// this instruction can't be called. Such escrows leak account rent. Token
/// escrows that were never funded are an unusual flow; if it matters for
/// your use case, add a separate `close_unfunded_escrow_token` instruction.
#[derive(Accounts)]
pub struct CloseEscrowToken<'info> {
    #[account(
        mut,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
        constraint = escrow.is_terminal() @ EscrowError::EscrowNotTerminal,
        close = employer,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == escrow.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// Receives any residual tokens swept from the vault.
    #[account(
        mut,
        constraint = employer_token_account.owner == escrow.employer @ EscrowError::Unauthorized,
        constraint = employer_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// Receives the rent refund (vault ATA rent + escrow account rent).
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

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<CloseEscrowToken>) -> Result<()> {
    let escrow_id = ctx.accounts.escrow.escrow_id;
    let bump = ctx.accounts.escrow.bump;
    let escrow_seeds = &[Escrow::ESCROW_SEED, escrow_id.as_ref(), &[bump]];
    let signer_seeds = &[&escrow_seeds[..]];

    // Sweep any residual tokens out of the vault before closing it.
    let vault_balance = ctx.accounts.vault_token_account.amount;
    if vault_balance > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employer_token_account.to_account_info(),
                    authority: ctx.accounts.escrow.to_account_info(),
                },
                signer_seeds,
            ),
            vault_balance,
        )?;
    }

    // Close the vault token account; rent goes to employer.
    token::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        CloseAccount {
            account: ctx.accounts.vault_token_account.to_account_info(),
            destination: ctx.accounts.employer.to_account_info(),
            authority: ctx.accounts.escrow.to_account_info(),
        },
        signer_seeds,
    ))?;

    msg!(
        "Token escrow {:?} closed; {} tokens swept, all rent refunded to employer",
        escrow_id,
        vault_balance
    );

    Ok(())
}

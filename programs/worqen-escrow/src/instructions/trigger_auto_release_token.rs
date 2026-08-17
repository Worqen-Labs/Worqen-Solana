use crate::errors::EscrowError;
use crate::events::DisputeResolved;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

/// Accounts for triggering auto-release of a token escrow. Every destination
/// must already exist — the caller creates them.
#[derive(Accounts)]
pub struct TriggerAutoReleaseToken<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Disputed @ EscrowError::InvalidStatus,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == escrow.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = employee_token_account.owner == escrow.employee @ EscrowError::Unauthorized,
        constraint = employee_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = employer_token_account.owner == escrow.employer @ EscrowError::Unauthorized,
        constraint = employer_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// Treasury token account — receives the commission. Constrained on owner +
    /// mint so the platform fee cannot be redirected.
    #[account(
        mut,
        constraint = platform_token_account.owner == escrow.fee_recipient @ EscrowError::Unauthorized,
        constraint = platform_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    pub caller: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

/// After the dispute deadline, anyone can release the full remaining worker
/// amount to the employee. The platform keeps its commission (routed to the
/// treasury); only any dust is swept to the employer.
pub fn handler(ctx: Context<TriggerAutoReleaseToken>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;

    require!(
        escrow.dispute_deadline != 0,
        EscrowError::AutoReleaseNotConfigured
    );
    require!(
        clock.unix_timestamp >= escrow.dispute_deadline,
        EscrowError::DisputeDeadlineNotReached
    );

    let remaining_worker = escrow.remaining_worker_amount();
    let remaining_commission = escrow.remaining_commission();

    let escrow_id = escrow.escrow_id;
    let bump = escrow.bump;
    let escrow_seeds = &[Escrow::ESCROW_SEED, escrow_id.as_ref(), &[bump]];
    let signer_seeds = &[&escrow_seeds[..]];

    if remaining_worker > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employee_token_account.to_account_info(),
                    authority: escrow.to_account_info(),
                },
                signer_seeds,
            ),
            remaining_worker,
        )?;
    }

    // Platform keeps its commission: route remaining commission to the treasury
    // token account before sweeping any dust to the employer.
    ctx.accounts.vault_token_account.reload()?;
    let commission_to_treasury = remaining_commission.min(ctx.accounts.vault_token_account.amount);
    if commission_to_treasury > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.platform_token_account.to_account_info(),
                    authority: escrow.to_account_info(),
                },
                signer_seeds,
            ),
            commission_to_treasury,
        )?;
    }

    // Sweep any remaining dust to the employer.
    ctx.accounts.vault_token_account.reload()?;
    let dust_to_employer = ctx.accounts.vault_token_account.amount;
    if dust_to_employer > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employer_token_account.to_account_info(),
                    authority: escrow.to_account_info(),
                },
                signer_seeds,
            ),
            dust_to_employer,
        )?;
    }

    escrow.released_to_employee = escrow.amount;
    escrow.status = EscrowStatus::Resolved;
    escrow.completed_at = clock.unix_timestamp;
    escrow.dispute_resolved_by = ctx.accounts.caller.key();
    escrow.dispute_resolved_at = clock.unix_timestamp;
    escrow.employee_share_resolved = remaining_worker;
    escrow.employer_share_resolved = 0;

    emit!(DisputeResolved {
        escrow_id: escrow.escrow_id,
        resolver: ctx.accounts.caller.key(),
        employee_share: remaining_worker,
        employer_share: 0,
        // Platform keeps its commission on auto-release; nothing is refunded.
        commission_refunded: 0,
        is_native: false,
        token_mint: escrow.token_mint,
        forced: true,
    });

    Ok(())
}

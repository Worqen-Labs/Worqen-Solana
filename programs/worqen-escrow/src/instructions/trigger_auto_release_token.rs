use crate::errors::EscrowError;
use crate::events::DisputeResolved;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Token variant of `trigger_auto_release_sol`: Disputed-state only.
///
/// ATAs are `init_if_needed` so parties with no token account can still
/// receive funds; the caller pays the rent.
#[derive(Accounts)]
pub struct TriggerAutoReleaseToken<'info> {
    #[account(
        mut,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
        constraint = escrow.status == EscrowStatus::Disputed @ EscrowError::InvalidStatus,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        constraint = token_mint.key() == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub token_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = vault_token_account.owner == escrow.key() @ EscrowError::Unauthorized,
        constraint = vault_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: verified against escrow.employee
    #[account(constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized)]
    pub employee: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = caller,
        associated_token::mint = token_mint,
        associated_token::authority = employee,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: verified against escrow.employer
    #[account(constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized)]
    pub employer: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = caller,
        associated_token::mint = token_mint,
        associated_token::authority = employer,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// Anyone can call. Pays gas + ATA rent if the parties' ATAs need init.
    #[account(mut)]
    pub caller: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<TriggerAutoReleaseToken>) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;
    let caller_key = ctx.accounts.caller.key();

    require!(
        escrow.dispute_deadline != 0,
        EscrowError::AutoReleaseNotConfigured
    );
    require!(
        clock.unix_timestamp >= escrow.dispute_deadline,
        EscrowError::DisputeDeadlineNotReached
    );

    let escrow_id = escrow.escrow_id;
    let bump = escrow.bump;
    let escrow_seeds = &[Escrow::ESCROW_SEED, escrow_id.as_ref(), &[bump]];
    let signer_seeds = &[&escrow_seeds[..]];

    let remaining_worker = escrow.remaining_worker_amount();
    let remaining_commission = escrow.remaining_commission();

    // Pay worker their full remaining amount.
    if remaining_worker > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
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

    // Drain remainder (commission + dust) to employer.
    ctx.accounts.vault_token_account.reload()?;
    let total_to_employer = ctx.accounts.vault_token_account.amount;
    if total_to_employer > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.employer_token_account.to_account_info(),
                    authority: escrow.to_account_info(),
                },
                signer_seeds,
            ),
            total_to_employer,
        )?;
    }

    escrow.released_to_employee = escrow.released_to_employee.saturating_add(remaining_worker);
    escrow.status = EscrowStatus::Resolved;
    escrow.completed_at = clock.unix_timestamp;
    escrow.dispute_resolved_by = caller_key;
    escrow.dispute_resolved_at = clock.unix_timestamp;
    escrow.employee_share_resolved = remaining_worker;
    escrow.employer_share_resolved = 0;

    emit!(DisputeResolved {
        escrow_id: escrow.escrow_id,
        resolver: caller_key,
        employee_share: remaining_worker,
        employer_share: 0,
        commission_refunded: remaining_commission,
        is_native: false,
        token_mint: escrow.token_mint,
        forced: true,
    });

    msg!(
        "Dispute force-resolved by {:?} after deadline ({} to worker, {} to employer)",
        caller_key,
        remaining_worker,
        total_to_employer
    );

    Ok(())
}

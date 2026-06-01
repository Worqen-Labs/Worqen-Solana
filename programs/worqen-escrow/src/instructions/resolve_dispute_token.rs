use crate::errors::EscrowError;
use crate::events::DisputeResolved;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Accounts for resolving a token dispute. Token accounts are constrained on
/// mint and owner to prevent fund redirection; party ATAs use `init_if_needed`
/// (platform pays, refundable via `close_escrow_token`) so 0-balance parties
/// can still receive their shares.
#[derive(Accounts)]
pub struct ResolveDisputeToken<'info> {
    /// The escrow account
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Disputed @ EscrowError::InvalidStatus,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
        constraint = escrow.platform_authority == platform_authority.key() @ EscrowError::Unauthorized,
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

    /// CHECK: Verified against escrow.employer
    #[account(constraint = employer.key() == escrow.employer @ EscrowError::Unauthorized)]
    pub employer: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = platform_authority,
        associated_token::mint = token_mint,
        associated_token::authority = employer,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Verified against escrow.employee
    #[account(constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized)]
    pub employee: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = platform_authority,
        associated_token::mint = token_mint,
        associated_token::authority = employee,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Verified against escrow.fee_recipient
    #[account(constraint = fee_recipient.key() == escrow.fee_recipient @ EscrowError::InvalidFeeRecipient)]
    pub fee_recipient: UncheckedAccount<'info>,

    /// Treasury token account — receives the commission. Constrained on owner +
    /// mint so the platform fee cannot be redirected. The platform keeps its fee
    /// on dispute resolution; it is no longer refunded to the employer.
    #[account(
        mut,
        constraint = platform_token_account.owner == escrow.fee_recipient @ EscrowError::Unauthorized,
        constraint = platform_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    /// The platform authority — pays for ATA creation if needed.
    #[account(mut)]
    pub platform_authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Platform resolves a dispute by splitting the remaining worker amount and
/// KEEPING the commission. The full remaining commission is routed to the
/// treasury (`platform_token_account`); only the employer's share of the worker
/// amount (plus any dust) is refunded — the fee is never returned.
/// Vault is drained to actual balance (sweeps any dust to employer).
pub fn handler(ctx: Context<ResolveDisputeToken>, employee_share: u64) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let clock = Clock::get()?;

    let remaining_worker = escrow.remaining_worker_amount();
    let remaining_commission = escrow.remaining_commission();

    require!(
        employee_share <= remaining_worker,
        EscrowError::InvalidEmployeeShare
    );

    let employer_share = remaining_worker - employee_share;

    let escrow_id = escrow.escrow_id;
    let bump = escrow.bump;
    let escrow_seeds = &[Escrow::ESCROW_SEED, escrow_id.as_ref(), &[bump]];
    let signer_seeds = &[&escrow_seeds[..]];

    // Pay worker their share first.
    if employee_share > 0 {
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
            employee_share,
        )?;
    }

    // Platform keeps its commission on a dispute: route the full remaining
    // commission to the treasury token account before refunding the employer.
    ctx.accounts.vault_token_account.reload()?;
    let commission_to_treasury = remaining_commission.min(ctx.accounts.vault_token_account.amount);
    if commission_to_treasury > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
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

    // Drain remaining vault (employer_share + dust) to employer's token account.
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

    escrow.released_to_employee = escrow.released_to_employee.saturating_add(employee_share);
    escrow.status = EscrowStatus::Resolved;
    escrow.completed_at = clock.unix_timestamp;
    escrow.dispute_resolved_by = ctx.accounts.platform_authority.key();
    escrow.dispute_resolved_at = clock.unix_timestamp;
    escrow.employee_share_resolved = employee_share;
    escrow.employer_share_resolved = employer_share;

    emit!(DisputeResolved {
        escrow_id: escrow.escrow_id,
        resolver: ctx.accounts.platform_authority.key(),
        employee_share,
        employer_share,
        // Platform now keeps its commission on disputes; nothing is refunded.
        commission_refunded: 0,
        is_native: false,
        token_mint: escrow.token_mint,
        forced: false,
    });

    msg!(
        "Dispute resolved: {} tokens to employee, {} tokens to employer, {} commission to treasury",
        employee_share,
        total_to_employer,
        commission_to_treasury
    );

    Ok(())
}

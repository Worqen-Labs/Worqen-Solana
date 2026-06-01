use crate::errors::EscrowError;
use crate::events::EscrowSettled;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

/// Accounts for an amicable settle of a token escrow.
///
/// Both parties sign (no platform authority). Token accounts are constrained
/// on `mint`/`owner` to prevent fund redirection; payee ATAs use
/// `init_if_needed` so a 0-balance party can still receive its share.
#[derive(Accounts)]
pub struct MutualCancelToken<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Funded
            || escrow.status == EscrowStatus::PendingRelease @ EscrowError::InvalidStatus,
        constraint = !escrow.is_native @ EscrowError::NotTokenEscrow,
        constraint = escrow.employer == employer.key() @ EscrowError::Unauthorized,
        constraint = escrow.employee == employee.key() @ EscrowError::Unauthorized,
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

    /// The employer — co-signs the settlement and pays for ATA creation.
    #[account(mut)]
    pub employer: Signer<'info>,

    #[account(
        init_if_needed,
        payer = employer,
        associated_token::mint = token_mint,
        associated_token::authority = employer,
    )]
    pub employer_token_account: Box<Account<'info, TokenAccount>>,

    /// The employee — co-signs the settlement.
    pub employee: Signer<'info>,

    #[account(
        init_if_needed,
        payer = employer,
        associated_token::mint = token_mint,
        associated_token::authority = employee,
    )]
    pub employee_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Verified against escrow.fee_recipient
    #[account(constraint = fee_recipient.key() == escrow.fee_recipient @ EscrowError::InvalidFeeRecipient)]
    pub fee_recipient: UncheckedAccount<'info>,

    /// Treasury token account — receives the commission. Constrained on owner +
    /// mint. The platform keeps its fee on a mutual cancellation.
    #[account(
        mut,
        constraint = platform_token_account.owner == escrow.fee_recipient @ EscrowError::Unauthorized,
        constraint = platform_token_account.mint == escrow.token_mint @ EscrowError::InvalidTokenMint,
    )]
    pub platform_token_account: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Employer + employee amicably settle a funded token escrow without a
/// dispute. `employee_share` (<= remaining worker amount) goes to the
/// employee; the platform keeps its commission (routed to the treasury); the
/// remainder — the employer's portion plus any dust — drains back to the
/// employer.
pub fn handler(ctx: Context<MutualCancelToken>, employee_share: u64) -> Result<()> {
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

    // Platform keeps its commission: route remaining commission to the treasury
    // token account before refunding the employer.
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

    // Drain remaining vault (employer_share + dust) back to the employer.
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

    escrow.released_to_employee = escrow
        .released_to_employee
        .checked_add(employee_share)
        .ok_or(EscrowError::InvalidAmount)?;
    escrow.status = EscrowStatus::Resolved;
    escrow.completed_at = clock.unix_timestamp;
    escrow.employee_share_resolved = employee_share;
    escrow.employer_share_resolved = employer_share;

    emit!(EscrowSettled {
        escrow_id: escrow.escrow_id,
        employee_share,
        employer_share,
        is_native: false,
        token_mint: escrow.token_mint,
    });

    msg!(
        "Mutual settle: {} tokens to employee, {} tokens to employer, {} commission to treasury",
        employee_share,
        total_to_employer,
        commission_to_treasury
    );

    Ok(())
}

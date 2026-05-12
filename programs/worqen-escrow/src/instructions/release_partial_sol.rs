use crate::errors::EscrowError;
use crate::events::EscrowReleased;
use crate::state::{Escrow, EscrowStatus};
use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

/// Accounts required for a partial SOL release.
///
/// Partial releases are only permitted in `Funded` status (before any
/// confirmation or dispute). Once a party confirms or disputes, the caller
/// must use the regular `release_sol` / `resolve_dispute_sol` flows.
#[derive(Accounts)]
pub struct ReleasePartialSol<'info> {
    #[account(
        mut,
        constraint = escrow.status == EscrowStatus::Funded @ EscrowError::InvalidStatus,
        constraint = escrow.is_native @ EscrowError::NotNativeEscrow,
    )]
    pub escrow: Account<'info, Escrow>,

    #[account(
        mut,
        seeds = [Escrow::VAULT_SEED, escrow.key().as_ref()],
        bump = escrow.vault_bump,
    )]
    /// CHECK: PDA vault
    pub escrow_vault: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = employee.key() == escrow.employee @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.employee
    pub employee: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = platform_authority.key() == escrow.platform_authority @ EscrowError::Unauthorized,
    )]
    /// CHECK: Verified against escrow.platform_authority
    pub platform_authority: UncheckedAccount<'info>,

    /// Authority: employer (no confirmation required for partials, because
    /// employer is volunteering to pay out mid-work) OR platform_authority.
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// Release `amount` to employee now, plus a proportional commission to the
/// platform. The remainder stays in escrow, status stays `Funded` unless this
/// slice exhausts the worker amount (then it flips to `Released` and the
/// vault is fully drained, including any dust).
///
/// Commission for the slice is computed as the **delta** between cumulative-
/// due before and after the partial — never as a per-slice independent
/// floor — so the sum of partial commissions equals the single-release
/// commission for the same total amount.
///
/// For non-final partials, the post-slice vault balance must be either 0
/// or above the rent-exempt minimum (Solana's transfer rule). Otherwise the
/// instruction errors with `PartialReleaseLeavesDust`. The full release
/// path doesn't have this restriction because it drains the vault entirely.
pub fn handler(ctx: Context<ReleasePartialSol>, amount: u64) -> Result<()> {
    let escrow = &mut ctx.accounts.escrow;
    let authority_key = ctx.accounts.authority.key();

    let is_employer = authority_key == escrow.employer;
    let is_platform = authority_key == escrow.platform_authority;
    require!(is_employer || is_platform, EscrowError::ReleaseNotAuthorized);

    require!(amount > 0, EscrowError::InvalidAmount);

    let remaining = escrow.remaining_worker_amount();
    require!(amount <= remaining, EscrowError::PartialReleaseTooLarge);

    // Cumulative-delta commission math: the slice commission is whatever
    // the cumulative commission grows by, not a per-slice floor. This makes
    // sum-of-slices == single-release commission for the same total.
    let bps = escrow.commission_rate_bps;
    let cumulative_before = Escrow::calculate_commission(escrow.released_to_employee, bps);
    let new_released = escrow
        .released_to_employee
        .checked_add(amount)
        .ok_or(EscrowError::InvalidAmount)?;
    let cumulative_after = Escrow::calculate_commission(new_released, bps);
    let commission_slice = cumulative_after.saturating_sub(cumulative_before);

    let new_remaining = remaining
        .checked_sub(amount)
        .ok_or(EscrowError::PartialReleaseTooLarge)?;
    let is_final = new_remaining == 0;

    let escrow_key = escrow.key();
    let vault_seeds = &[
        Escrow::VAULT_SEED,
        escrow_key.as_ref(),
        &[escrow.vault_bump],
    ];
    let signer_seeds = &[&vault_seeds[..]];

    // Pre-flight rent-exempt check for non-final partials. If this slice
    // would leave the vault at a non-zero amount below rent-exempt minimum,
    // Solana's System Program will reject the second transfer. Catch it
    // here with a clear error rather than a cryptic CPI failure.
    if !is_final {
        let rent_exempt_min = Rent::get()?.minimum_balance(0);
        let vault_balance = ctx.accounts.escrow_vault.lamports();
        let vault_after = vault_balance
            .checked_sub(amount.saturating_add(commission_slice))
            .ok_or(EscrowError::InsufficientFunds)?;
        require!(
            vault_after == 0 || vault_after >= rent_exempt_min,
            EscrowError::PartialReleaseLeavesDust
        );
    }

    // Worker leg
    transfer(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.escrow_vault.to_account_info(),
                to: ctx.accounts.employee.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
    )?;

    // Commission/leftover leg.
    // - Final slice: drain the entire remaining vault to platform (covers
    //   commission + any dust deposit). Vault ends at exactly 0.
    // - Non-final slice: pay only the recorded commission_slice; vault
    //   stays funded for future slices.
    let platform_payment = if is_final {
        ctx.accounts.escrow_vault.lamports()
    } else {
        commission_slice
    };
    if platform_payment > 0 {
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_vault.to_account_info(),
                    to: ctx.accounts.platform_authority.to_account_info(),
                },
                signer_seeds,
            ),
            platform_payment,
        )?;
    }

    escrow.released_to_employee = new_released;

    if is_final {
        let clock = Clock::get()?;
        escrow.status = EscrowStatus::Released;
        escrow.completed_at = clock.unix_timestamp;
        escrow.release_initiator = authority_key;
    }

    emit!(EscrowReleased {
        escrow_id: escrow.escrow_id,
        recipient: escrow.employee,
        amount,
        commission_amount: platform_payment,
        commission_recipient: escrow.platform_authority,
        is_native: true,
        token_mint: escrow.token_mint,
        initiator: authority_key,
        is_partial: !is_final,
        remaining_worker_amount: new_remaining,
    });

    msg!(
        "Partial release: {} lamports to employee ({} remaining), {} lamports to platform",
        amount,
        new_remaining,
        platform_payment
    );

    Ok(())
}

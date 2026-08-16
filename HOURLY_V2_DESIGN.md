# Hourly v2 — on-chain design

Replaces the v1 GWS engine (`HourlyPeriod` with a fixed `[Tranche; 7]` array) with a
per-invoice-PDA model. Product spec agreed 2026-08-08 (`../HOURLY_REDESIGN_QUESTIONS.md`).
Pre-mainnet, dev data droppable: no migration path for v1 accounts; devnet is redeployed.

## Product rules encoded

- Employer sets a weekly **hours** limit; backend derives the money cap (`hours x rate`) and
  passes `weekly_cap_net` in token base units. Fee-on-top: vault target is
  `cap_gross = cap_net + commission(cap_net)`.
- Weeks are hire-anchored 168h windows computed by the backend; the program only enforces the
  window it is given: staging allowed strictly inside `[period_start_at, period_end_at)`.
- Unlimited invoices per week (bounded only by cap and u16 index).
- Each invoice releases independently `review_window_secs` (default 7d) after staging unless
  disputed first; dispute window == review window.
- Disputes: either party or platform raises; platform resolves with a split; permissionless
  auto-release to the worker after `dispute_deadline` (long-stop, 3–90d) if the platform is dead.
- At `period_end_at` anyone may settle: refund `vault - outstanding` to the employer. Disputed
  and still-frozen invoices keep their money until finalized/resolved.
- Native SOL and allowlisted SPL mints both supported.
- No delegate/pull funding. Employer signs every fund (custodial key server-side or browser wallet).

## Accounts

### HourlyPeriod v2 (PDA `["hourly", hire_id, period_index_le_u32]`)

| field | type | notes |
|---|---|---|
| version | u8 | = 2 |
| hire_id | [u8;32] | backend hire UUID hash |
| period_index | u32 | backend week number (hire-anchored), pure seed |
| employer / employee / platform_authority / fee_recipient | Pubkey | fee_recipient snapshotted from Config at open |
| token_mint | Pubkey | System Program ID when native |
| is_native | bool | |
| bump / vault_bump | u8 | vault_bump only for native vault PDA `["vault", period_key]`; 0 for token (vault = ATA of period) |
| rent_payer | Pubkey | open_period payer; receives period rent at close |
| weekly_cap_net | u64 | monotonic via raise_weekly_cap |
| commission_rate_bps | u16 | snapshot at open, <= 1000 |
| funded_gross | u64 | cumulative in; fund tops up to cap_gross |
| total_staged_net | u64 | lifetime staged, <= weekly_cap_net |
| released_net | u64 | paid to worker (finalize + resolve shares) |
| refunded_amount | u64 | settle refund |
| outstanding_net / outstanding_commission | u64 | running sums over live (Staged/Disputed) invoices — replaces v1 array scan |
| invoice_count | u16 | append-only index cursor |
| live_invoices | u16 | open invoice accounts; close_period requires 0 |
| review_window_secs | i64 | (0, 30d] |
| period_start_at / period_end_at | i64 | end = start + duration, duration in [1d, 30d], end > now at open |
| created_at / funded_at / settled_at | i64 | |
| status | HourlyStatus | Open -> Funded -> Active -> Settled |

### HourlyInvoice (PDA `["hourly_inv", period_key, invoice_index_le_u16]`)

version u8 (=1), period Pubkey, invoice_index u16, ref_id [u8;32] (backend invoice UUID hash),
amount_net u64, commission u64 (marginal, cumulative-rounded), staged_at i64, release_at i64,
dispute_deadline i64 (0 = none), disputed_by Pubkey, status (Staged | Disputed),
rent_payer Pubkey (stage signer = platform), bump u8, reserved [u8;32].

Invoice accounts are closed at finalize/approve/resolve/auto-release (rent back to rent_payer);
terminal history lives in events + backend.

## Instructions (19, replacing v1's 10; `pull_fund_period` and the global
`delegate_auth` PDA are removed)

| ix | signer | gates |
|---|---|---|
| open_period | employer OR platform (payer, stored as rent_payer) | config not paused; mint allowlisted (is_native rule as create_escrow); cap_net > 0; bps <= 1000; parties distinct; window bounds; review window bounds |
| fund_period_sol / _token | employer | status Open/Funded/Active; now < period_end_at; transfers `cap_gross - funded_gross`; token variant inits vault ATA (init_if_needed) |
| raise_weekly_cap | employer OR platform | status Open/Funded/Active; now < period_end_at; new >= old; new >= total_staged_net; moves no money (fund the delta next) |
| stage_invoice_sol / _token | platform (pays invoice rent) | not paused; status Funded/Active; period_start <= now < period_end; cap check; solvency: outstanding + new amount+commission <= actual vault balance; init invoice PDA at invoice_count |
| raise_invoice_dispute | employer OR employee OR platform | invoice Staged; now < release_at; deadline in [now+3d, now+90d]; reason <= 256 bytes (event only) |
| finalize_invoice_sol / _token | anyone | invoice Staged; now >= release_at; pays net -> employee, commission -> treasury; closes invoice |
| approve_invoice_sol / _token | employer OR platform | not paused; invoice Staged; NO time gate (instant release, skips remaining review window); pays net -> employee, commission -> treasury; closes invoice; emits HourlyInvoiceApproved |
| resolve_invoice_sol / _token | platform | invoice Disputed; employee_share <= amount_net; pro-rata commission to treasury (capped at invoice.commission), excess commission + employer share -> employer; closes invoice |
| auto_release_invoice_sol / _token | anyone | invoice Disputed; now >= dispute_deadline != 0; full amount -> employee, commission -> treasury; closes invoice |
| settle_period_sol / _token | anyone | now >= period_end_at; status != Settled; refund `vault - outstanding` -> employer; status = Settled |
| close_period_sol / _token | anyone | status Settled; live_invoices == 0; sweep vault dust -> employer; close vault (token: CloseAccount; native: drain); close period -> rent to rent_payer |

Counter updates: stage `+outstanding/+live/+staged/+count`; finalize/approve/resolve/auto-release
`-outstanding/-live`, `+released_net` (by worker-paid amount). All math checked.

## Native SOL handling

Vault = 0-data PDA `["vault", period_key]` (fixed-escrow precedent). Fund via system
transfer; payouts via system transfer with vault signer seeds; solvency reads
`vault.lamports()`. close_period_sol drains any residue to the employer so the vault ends at 0.

## What deliberately stays off-chain

Hire-anchored week math, hours/rate arithmetic, who may invoice after termination, the
7d amicable-negotiation clock and moderation escalation (on-chain sees only the 3–90d
long-stop), notifications, and invoice metadata beyond `ref_id`.

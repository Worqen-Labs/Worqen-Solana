---
name: worqen-solana
description: >-
  Build or modify the Worqen on-chain escrow program (solana/programs/worqen-escrow).
  Use when adding or changing Anchor instructions, account state, PDAs/seeds, errors or
  events, writing LiteSVM tests, regenerating the codama client, running the config/pause
  ops scripts, or deploying to devnet/mainnet. Triggers on tasks touching Anchor 1.1,
  Rust escrow logic, hourly-v2 periods/invoices, commission math, program ids, or the
  backend byte-offset decoders and frontend generated client that must stay in sync.
---

# Worqen Escrow Program Playbook

Stack: **Anchor 1.1.2 / Rust, program `worqen-escrow` — v1.5.0, Solana CLI (Agave) 4.0.0,
Bun + LiteSVM for tests, codama for the frontend client.** One program, three settlement
engines (fixed-price/milestone escrow, direct pay, hourly v2) plus a singleton `Config`.
**47 instructions, 4 accounts, 24 events, 68 errors** (5 of them retired as `ReservedXXXX`).
Two artifacts from one source: `target/deploy/worqen_escrow.so` (905,720 B, default arch —
what LiteSVM loads and the IDL comes from) and `target/deploy-v3/worqen_escrow.so`
(751,720 B, SBPFv3 — **what every cluster deploy uploads**).

> **Devnet runs v1.5.0 — deployed 2026-08-17, so deployed == source.** The upgrade landed in
> place at slot 484755229 (tx `64q6nFoc…`); the on-chain bytes are sha256
> `73ea91040ce236e8486fdcd509540f8e3507cf524547d576a307653bc9050c65`, byte-identical to
> `target/deploy-v3/worqen_escrow.so`. The on-chain IDL moved to the **Program Metadata
> Program** account `D5EDchbfDVyCfgF1SmVTXutyDAiSU4R5ZYWyH3urwXZC` (Anchor 1.x); the legacy
> `2Y1y1m4c…` account was closed first with `anchor-0.32.1` and its rent recovered. If
> `anchor idl fetch` returns a truncated stream, re-run `make idl-devnet` — publication fought
> public-RPC 429 throttling. `devnet-deployment.json` stays the reference for what is actually
> out there. **Caveat that bites**: the deployed staging services (backend :8001,
> dev.worqen.com) still run pre-campaign code, so their deposit builders (no `Config` account),
> `open_period` (no platform co-signer) **and every token-payout builder** (v1.4.0-era account
> lists, now 46 slots too long) fail against devnet until the `dev` branches are pushed and
> redeployed — `create_escrow` keeps working from old backends.

Read `solana/CLAUDE.md` first — it owns the program-id/keypair table, the two-artifact split
and the deploy-key facts. `HOURLY_V2_DESIGN.md` is the accurate hourly-v2 spec; `SECURITY.md`
and `README.md` were both brought current with v1.5.0 on 2026-08-17 (47 instructions, the
15-instruction pause gate, the caller-supplied-ATA contract, the completed devnet deploy) and
are trustworthy again.

## What v1.5.0 changed

Same 47 instructions, same account **structs** and discriminators — stored state did not
migrate. What moved is the account *lists* of ten `_token` instructions, plus the toolchain.

| Change | Detail |
|---|---|
| **ATA-constraint diet** | The 18 non-vault `associated_token::mint` / `associated_token::authority` constraints became plain `token::mint` + `token::authority` (`owner` + `mint`) checks, which made the `token_mint` / derivation-only `employee` / `employer` / `fee_recipient` / `associated_token_program` / `system_program` slots dead. **46 account slots removed** across 10 instructions: `release_token` 11→6, `mutual_cancel_token` 12→8, `resolve_dispute_token` 13→7, `trigger_auto_release_token` 13→7, `finalize_invoice_token` 13→8, `auto_release_invoice_token` 13→8, `approve_invoice_token` 14→9, `resolve_invoice_token` 15→9, `settle_period_token` 9→8, `close_period_token` 10→7. The three **vault** ATA constraints (`deposit_token`, `fund_period_token`, `settle_period_token`) are untouched — there the canonical derivation *is* the security binding |
| **Destination ATAs are the caller's job** | `init_if_needed` is gone from every payout destination. A missing destination now fails **3012 `AccountNotInitialized`** instead of self-healing. Backend prepends `ensure_token_account_ixs` / `HourlyContext.ensure_token_accounts`, frontend prepends `buildEnsureTokenAccountInstructions`, and `scripts/bootstrap-config.ts` creates the treasury ATA per allowlisted mint (done on devnet). The program also no longer requires the *canonical* ATA address — canonicality is now an off-chain concern |
| **Anchor 0.32.1 → 1.1.2** | Adds a built-in duplicate-mutable-account guard: passing the same mutable account in two slots of one instruction now returns **2040 `ConstraintDuplicateMutableAccount`**. The legacy `__idl_*` handlers are gone (on-chain IDL → Program Metadata Program). JS tooling moved to `@anchor-lang/core`. Test suite grew to **103** |
| **SBPFv3 deploy artifact** | `make build-deploy` (`cargo build-sbf --arch v3 --tools-version v1.55`) emits `target/deploy-v3/worqen_escrow.so`; `make deploy-devnet` uploads that one. **Deploys require an Agave ≥ 4.0.0 client** — a 3.x client fails `ELF error: invalid file header`. LiteSVM 1.1.0 cannot load v3, so tests run the default-arch build of identical source |
| **Size / CU** | 999,496 B (v1.4.0) → 905,720 default-arch → **751,720 B deployable**. Mainnet `extend` cost 2.0845 → **0.3600 SOL** (51,725 B still above the 700,040 B allocation — the goal was not fully met, but the cost is down 82.7%). Token-instruction CU is **−14.5 % … −40 %** vs v1.4.0 and now fully deterministic (the ATA bump-search tail is gone); `deposit_sol` is **+5.6 %** from the Anchor 1.x duplicate guard, kept deliberately as a safety invariant |

## What v1.4.0 changed

Every item here is breaking IDL surface: the codama regen and backend builder review were
done with the deploy, and any client still on the old surface breaks against devnet.

| Change | Detail |
|---|---|
| **5 instructions removed** | `release_partial_sol/_token`, `deposit_more_sol/_token`, `close_unfunded_escrow_sol`. All were callerless. `close_unfunded_escrow_token` is **kept** — the backend's nightly rent-close task builds it for terminal never-funded SPL escrows. 52 → 47 |
| **`create_escrow` pins the arbitrator** | `platform_authority.key() == config.platform_authority && config.platform_authority != Pubkey::default()`, exactly like `open_period`. Violations return **6065 `InvalidPlatformAuthority`** |
| **`create_escrow` validates `escrow_kind`** | `escrow_kind::is_known(...)`; an unknown tag returns the new **6067 `InvalidEscrowKind`** instead of being stored verbatim |
| **Deposits are pause-gated** | `deposit_sol` and `deposit_token` now carry the `Config` account and the `!paused` check. Pause matrix **13 → 15 instructions** |
| **`open_period` needs a co-signer** | The platform authority signs alongside the employer authorizer, closing the period-PDA squat |
| **5 error codes retired** | 6015 (was `AutoReleaseNotReached`), 6018 (`PartialReleaseTooLarge`), 6032 (`PartialReleaseLeavesDust`), 6033 (`VaultNotEmpty`), 6045 (`TopUpNotFunded`) are now `Reserved6015`…`Reserved6045` and are never returned. Anchor numbers positionally, so codes are **held, never reused** |

## The golden path for a program change

1. **Edit the program.** One file per instruction in
   `programs/worqen-escrow/src/instructions/`, each exposing a `handler` fn plus its
   `#[derive(Accounts)]` context. A new instruction needs three edits: the file, a
   `pub mod` line in `instructions/mod.rs`, and a thin `#[program]` wrapper in `lib.rs`
   that calls `instructions::<file>::handler(...)`.
2. **Format and lint** — no hook does this for `.rs` (the workspace Stop hook only runs
   prettier on this repo's TS):
   ```bash
   make lint        # cargo fmt --all + cargo clippy --all-targets -- -D warnings
   ```
   CI runs `cargo fmt --all --check`, the same clippy line, and `bun run lint`
   (prettier over `tests/ scripts/ migrations/`) on every push/PR to `master`.
3. **Build** — `make build` (`anchor build`). Regenerates `target/idl/worqen_escrow.json`
   and `target/deploy/worqen_escrow.so`; both are inputs to everything downstream.
4. **Test** — `make test` (= `anchor build && bun test`). LiteSVM, in-process, no
   validator, no devnet. See [Tests](#tests).
5. **Regenerate the frontend client** — `bun run generate:client`
   (`scripts/generate-client.ts`) renders the `@solana/kit` client from the freshly built
   IDL into `frontend/apps/dashboard/lib/solana-wallet/generated/` with
   `deleteFolderBeforeRendering: true`. Commit the regenerated folder **in the frontend
   repo**; hand-edits there are destroyed on the next run.
6. **Re-check the backend decoders and builders** — they read neither the IDL nor the
   generated client:
   - `backend/app/services/solana.py:152-162` — hardcoded `Escrow` byte offsets.
   - `backend/app/services/hourly/onchain.py:11-40` — hand-written `HourlyPeriod` layout.
   - `backend/app/services/hourly/instructions.py` — re-implements all 19 hourly
     instructions with `_anchor_discriminator(name)` and duplicated seeds.
7. **Update the backend enums if a status enum changed** —
   `backend/app/core/enums/escrow.py` (`EscrowStatus`, `EscrowKind`, `HourlyPeriodStatus`)
   mirrors the on-chain integers 1:1. See [Cross-repo sync](#cross-repo-sync-checklist).

Anything that changes the IDL (accounts, args, errors, events) is a cross-repo change —
steps 5–7 are not optional.

## Source layout

```
programs/worqen-escrow/src/
  lib.rs                 declare_id (cfg-split) + security_txt (source_release: v1.5.0)
                         + 47 #[program] entrypoints
  errors.rs              EscrowError — 68 variants, codes 6000..6067 (5 Reserved)
  events.rs              24 #[event] structs
  state/escrow.rs        EscrowStatus, escrow_kind consts, Escrow account, commission math
  state/hourly.rs        HourlyStatus, InvoiceStatus, HourlyPeriod, HourlyInvoice
  state/config.rs        Config, CONFIG_SEED, MAX_ALLOWED_MINTS = 30
  instructions/*.rs      one file per instruction (config.rs holds all 5 config ones)
tests/escrow.test.ts     46 tests — config, fixed-price SOL + token, direct pay, disputes,
                         authority rotation, and the authorization negatives
tests/hourly.test.ts     57 tests — hourly v2 token + native SOL, pause, rent, isolation
                         (103 total)
scripts/                 bootstrap-config.ts, set-platform-authority.ts, pause.ts,
                         generate-client.ts, verify.sh
.github/workflows/       ci.yml, deploy.yml (manual devnet), release.yml (tag → mainnet buffer)
```

## Accounts, PDAs, rent

| Account | PDA seeds | Size (incl. 8-B discriminator) | Rent payer → refund |
|---|---|---|---|
| `Config` | `["config"]` | 1137 | `init_config` signer; never closed |
| `Escrow` | `["escrow", escrow_id[32]]` | 948 | `employer` → `close = employer` on `close_escrow_*` / `close_unfunded_escrow_token` |
| escrow SOL vault | `["vault", escrow_key]`, 0-data | lamports only | funded by `deposit_sol`; drained to 0 on release/resolve/cancel/close |
| escrow token vault | ATA(mint, authority = **escrow PDA**) | 165 | `employer` on first `deposit_token` → `close_escrow_token` |
| `HourlyPeriod` | `["hourly", hire_id[32], period_index u32 LE]` | 423 | `open_period` authorizer, stored as `rent_payer` → `close_period_*` |
| hourly SOL vault | `["vault", period_key]`, 0-data | lamports only | employer seeds `vault_rent_reserve = Rent::minimum_balance(0)` on first `fund_period_sol`; swept to employer by `close_period_sol` |
| hourly token vault | ATA(mint, authority = **period PDA**) | 165 | `employer` on `fund_period_token`, or the caller if `settle_period_token` creates it → closed to employer |
| `HourlyInvoice` | `["hourly_inv", period_key, invoice_count u16 LE]` | 213 | staging `platform_authority`, stored as `rent_payer` → refunded when the invoice closes |

Seed constants: `Escrow::ESCROW_SEED = b"escrow"`, `Escrow::VAULT_SEED = b"vault"` (both
vault kinds), `HourlyPeriod::HOURLY_SEED = b"hourly"`,
`HourlyInvoice::INVOICE_SEED = b"hourly_inv"`, `CONFIG_SEED = b"config"`.

Version bytes are per-account, not global: `ESCROW_ACCOUNT_VERSION = 1`,
`HOURLY_PERIOD_VERSION = 2`, `HOURLY_INVOICE_VERSION = 1`, `CONFIG_VERSION = 1`
(`devnet-deployment.json`'s single `account_schema_version: 2` means the hourly schema).
Every account ends in a `reserved` tail (Escrow 64 B, Period 64 B, Invoice 32 B,
Config 32 B) — **new fields must be carved out of `reserved`, appended after the last real
field, never inserted mid-struct.** `Config.platform_authority` was carved this way and
reads as `Pubkey::default()` on Configs created before it existed.

## Instruction quick reference

### Config (5) — `instructions/config.rs`

| Instruction | Signer | Effect |
|---|---|---|
| `init_config(fee_recipient, default_bps, allowed_mints)` | anyone (becomes `authority`, pays rent) | creates the singleton; `paused=false`, `platform_authority = default()` |
| `update_config(fee_recipient?, default_bps?, paused?, pending_authority?, platform_authority?)` | `Config.authority` | each `None` untouched; `bps ≤ 1000`; `fee_recipient ≠ default()` |
| `accept_authority` | the `pending_authority` | completes the two-step handoff |
| `add_allowed_mint` / `remove_allowed_mint` | `Config.authority` | mint allowlist, ≤ 30 entries |

Four distinct roles: **upgrade authority** (bytecode), **Config authority** (pause,
treasury, default bps, allowlist, handoff), **platform authority** (per-escrow/per-period
hot key that releases and resolves), **fee_recipient** (treasury, never signs).
`fee_recipient` is snapshotted onto each `Escrow`/`HourlyPeriod` at create/open, so
rotating `Config.fee_recipient` affects only future objects.
`Config.platform_authority` must be set (`scripts/set-platform-authority.ts`) before any
`open_period` can succeed.

### Fixed-price / milestone escrow (19)

| Instruction | Signer(s) | Gates |
|---|---|---|
| `create_escrow(escrow_id, group_id, seq, total, amount, is_native, bps, auto_release_at, escrow_kind, terms_hash)` | `employer` | not paused; `amount>0`; `bps ≤ 1000`; **`escrow_kind` known (6067)**; **`platform_authority == config.platform_authority ≠ default` (6065)**; employer ≠ employee; platform ∉ {employer, employee}; `is_native` ⇔ mint == SystemProgram; mint allowlisted; group seq in `[1,total]` or both 0; `auto_release_at` future and ≤ 1 y |
| `deposit_sol` / `deposit_token` | `employer` | **not paused (new in v1.4.0 — carries the `Config` account)**; status `Created`; moves `amount + commission`; token variant `init_if_needed`s the vault ATA |
| `confirm_completion` | employer **or** employee | `Funded`/`PendingRelease`; the first confirm flips `Funded → PendingRelease`; each party once |
| `release_sol` / `release_token(ref_id)` | employer (needs `employer_confirmed`) **or** `platform_authority` **or** employee (needs both confirms) | `Funded`/`PendingRelease`; worker gets `remaining_worker_amount()`, vault then drained to `fee_recipient` → `Released` |
| `raise_dispute(reason, dispute_deadline)` | employer or employee; in `PendingRelease` **employer only** (`DisputeLockedAfterConfirm`) | deadline mandatory, in `[now+3 d, now+90 d]`; reason ≤ 256 B → `Disputed` |
| `resolve_dispute_sol` / `_token(employee_share)` | `platform_authority` | `Disputed`; `share ≤ remaining_worker`; **full remaining commission → treasury**, rest + dust → employer → `Resolved` |
| `trigger_auto_release_sol` / `_token` | **anyone** | `Disputed`; `now ≥ dispute_deadline ≠ 0`; full remaining worker amount → employee, commission → treasury → `Resolved` |
| `cancel_escrow_sol` / `_token(reason)` | `Created`: employer or platform. `Funded`: **platform only** (`EmployerCancelAfterFundedDisallowed`) | commission → treasury (capped at the live vault), rest → employer → `Cancelled` |
| `mutual_cancel_sol` / `_token(employee_share)` | **employer AND employee both sign** | `Funded`/`PendingRelease`; commission → treasury → `Resolved` |
| `update_platform_authority` | current `platform_authority` | blocked while `Disputed` and in every terminal state; new ≠ employer/employee/current |
| `close_escrow_sol` / `_token` | employer or platform | `is_terminal()`; sweeps vault residue, `close = employer` |
| `close_unfunded_escrow_token` | employer or platform | `Cancelled && funded_at == 0`; rent → employer. The `_sol` twin was removed in v1.4.0 — a never-funded SOL escrow closes through `close_escrow_sol` |

One live trap remains in this engine: `escrow.auto_release_at` is validated, stored and
emitted but **never read by any handler** — the only forced payout is
`trigger_auto_release_*`, keyed off `dispute_deadline`. v1.4.0 retired the matching error as
`Reserved6015` rather than wiring it up, so the gap is now explicit (`docs/RISK-REGISTER.md`
R-20). `escrow_kind` (`0 MILESTONE, 1 HOURLY, 2 RETAINER, 255 OTHER`) is now **validated** —
an unknown tag returns 6067.

**Before touching a `_token` payout — the v1.5.0 contract.** `release_token`, the four
invoice payout instructions, `resolve_dispute_token`, `trigger_auto_release_token`,
`mutual_cancel_token`, `settle_period_token` and `close_period_token` **no longer create the
destination token account and no longer require the canonical ATA** — each destination slot
is validated as `token::authority == <employee | employer | fee_recipient snapshotted on the
escrow/period>` and `token::mint == <escrow/period mint>`, nothing more. A missing
destination fails **3012 `AccountNotInitialized`**, so the caller must prepend an idempotent
`CreateAssociatedTokenAccountIdempotent` (backend `ensure_token_account_ixs` /
`HourlyContext.ensure_token_accounts`; frontend `buildEnsureTokenAccountInstructions`;
treasury ATAs seeded per mint by `scripts/bootstrap-config.ts`). "A worker with an empty
wallet can still be paid" is still true, but now because the *caller* pays that ~0.00204 SOL
explicitly rather than the constraint doing it invisibly (R-71). Add a new payout path
without the prepend and the first payout on a fresh mint reverts. Only the three **vault**
ATAs (`deposit_token`, `fund_period_token`, `settle_period_token`) still self-create.

### Direct pay, no escrow (4)

`pay_with_commission_sol` / `_token(hire_id, amount, bps)` and
`batch_pay_with_commission_sol` / `_token(hire_id, amounts[], bps)`, all signed by `payer`.
Pause-gated; `fee_recipient` must equal `Config.fee_recipient`; no self-pay; token variants
check the mint allowlist and require the recipient/treasury ATAs to exist. Batch recipients
arrive via `remaining_accounts` positionally aligned with `amounts`,
`MAX_BATCH_RECIPIENTS = 16`, one commission on the total. Nothing is stored on-chain —
`DirectPaymentMade` / `BatchPaymentMade` events are the only record.

### Hourly v2 (19)

| Instruction | Signer | Gates |
|---|---|---|
| `open_period(hire_id, period_index, weekly_cap_net, bps, review_window_secs, period_start_at, period_duration_secs, is_native)` | employer **or** `Config.platform_authority` as authorizer, **plus the platform authority as a required co-signer** (v1.4.0 — this is what closes the period-PDA squat); the payer becomes `rent_payer` | not paused; named platform authority **pinned** to `config.platform_authority ≠ default`; `fee_recipient == config.fee_recipient`; mint allowlisted / native-consistent; cap > 0; bps ≤ 1000; parties distinct; review window ∈ (0, 30 d]; duration ∈ [1 d, 30 d]; `end > now`; `start ≤ now + 30 d` |
| `fund_period_sol` / `_token(max_fund_amount)` | `employer` | not paused; Open/Funded/Active; `now < period_end_at`; transfers exactly `cap_gross − funded_gross`, rejected with `FundExceedsMax` above the bound (anti-front-run of `raise_weekly_cap`); the SOL variant also transfers a one-time `vault_rent_reserve` |
| `raise_weekly_cap(new_cap_net)` | employer **or** period platform | not paused; Open/Funded/Active; `now < end`; monotonic (`CapCannotDecrease`) and `≥ total_staged_net`; moves no money |
| `stage_invoice_sol` / `_token(amount_net, ref_id)` | period `platform_authority` (pays invoice rent) | not paused; Funded/Active; `start ≤ now < end`; `total_staged + amount ≤ weekly_cap_net`; solvency `outstanding + amount + marginal_commission ≤ vault − rent_reserve`; PDA index = `invoice_count`; the first stage flips Funded → Active |
| `raise_invoice_dispute(dispute_deadline, reason)` | employer, employee **or** platform | invoice `Staged`; `now < release_at`; deadline ∈ `[now+3 d, now+90 d]`; reason ≤ 256 B (event only, not stored) |
| `finalize_invoice_sol` / `_token` | **anyone** | `Staged`; `now ≥ release_at` (= `staged_at + review_window_secs`); net → employee, commission → treasury; closes the invoice |
| `approve_invoice_sol` / `_token` | period `employer` **or** period `platform_authority` | not paused; `Staged`; **no time gate**; identical money motion to finalize; emits `HourlyInvoiceApproved` |
| `resolve_invoice_sol` / `_token(employee_share)` | period `platform_authority` | `Disputed`; `share ≤ amount_net`; treasury takes `min(commission(share), invoice.commission)`, employer gets the unpaid net **plus the un-earned commission** |
| `auto_release_invoice_sol` / `_token` | **anyone** | `Disputed`; `now ≥ dispute_deadline ≠ 0`; full net → employee, commission → treasury |
| `settle_period_sol` / `_token` | **anyone** | `now ≥ period_end_at`; status ≠ Settled; refunds `vault − outstanding_total − vault_rent_reserve` (SOL) / `vault − outstanding_total` (token) → employer → `Settled` |
| `close_period_sol` / `_token` | **anyone**, but the `rent_payer` account must be the one stored at `open_period` (`Unauthorized` otherwise) | `Settled` and `live_invoices == 0`; sweeps vault residue → employer (token: transfer + `CloseAccount`), closes the period and refunds its rent to the stored `rent_payer` |

## State machines

`EscrowStatus` (`state/escrow.rs`): `Created=0, Funded=1, PendingRelease=2, Released=3,
Disputed=4, Resolved=5, Cancelled=6`; `is_terminal()` = {Released, Resolved, Cancelled}.

```
Created --deposit_*--------------------------> Funded --confirm_completion--> PendingRelease
Created --cancel (employer|platform)---------> Cancelled --close_escrow_*/close_unfunded_escrow_token--> (closed)
Funded --cancel (platform only)--------------> Cancelled
Funded|PendingRelease --release_*------------> Released
Funded|PendingRelease --raise_dispute--------> Disputed
Funded|PendingRelease --mutual_cancel_* (both sign)--> Resolved
Disputed --resolve_dispute_* (platform)------> Resolved
Disputed --trigger_auto_release_* (anyone, after deadline)--> Resolved
Released|Resolved|Cancelled --close_escrow_*--> (account closed, rent → employer)
```

`HourlyStatus`: `Open=0 → Funded=1 → Active=2 → Settled=3`; there is **no on-chain
`Closed`** — `close_period_*` deletes the account and the backend adds
`HourlyPeriodStatus.CLOSED` off-chain. `InvoiceStatus`: `Staged=0 | Disputed=1`; invoices
have **no terminal status** — finalize/approve/resolve/auto-release close the account, so
terminal history exists only in events and the backend DB.

### Pause matrix (what `Config.paused` actually blocks)

`paused` gates exactly **15 instructions** (14 `require!` sites — the two `stage_invoice_*`
share `stage_common`) — every instruction that takes the `Config` account:
`create_escrow`, **`deposit_sol`**, **`deposit_token`**, `pay_with_commission_sol/_token`,
`batch_pay_with_commission_sol/_token`, `open_period`, `fund_period_sol/_token`,
`raise_weekly_cap`, `stage_invoice_sol/_token` (via `stage_common`),
`approve_invoice_sol/_token`.

The deposit pair joined the gate in v1.4.0, so **a freeze now stops every inflow**, including
into an escrow that already exists. `deposit_more_sol/_token` no longer exist.
This is live on devnet since the 2026-08-17 upgrade — which also means a deposit built without
the `Config` account (any pre-campaign backend, including the still-deployed :8001 staging one)
fails outright.

Everything on the payout side is ungated by design — release, confirm, dispute, resolve,
auto-release, cancel, mutual-cancel, close, finalize, settle, close_period,
`update_platform_authority` — so a pause can never strand funds already in escrow. The one
deliberate exception is `approve_invoice_*`: it is the only payout callable at will with no
time lock, and the same invoice still settles through the pause-free `finalize_invoice_*`
once `release_at` passes. That invariant is explicitly tested in `tests/hourly.test.ts`;
keep it true.

## Money conventions

- `Escrow::calculate_commission(amount, bps) = amount * bps / 10000`, u128 intermediate,
  floor (`state/escrow.rs`). `MAX_COMMISSION_RATE_BPS = 1000` is the hard cap.
- **Fee-on-top everywhere**: the employer deposits `amount + commission`; the worker
  receives the full `amount`. Commission goes to the snapshotted `fee_recipient`, never to
  the signing `platform_authority`.
- The tier constants in `state/escrow.rs` (`DEFAULT 500`, `PRIME 150`, `TIP 200`) are
  **informational only** — the effective bps is passed per call by the backend from
  `Plan.commission_bps` (`backend/app/services/subscription_billing.py:55`
  `effective_commission_bps`). The on-chain Prime constant is stale; the product Prime rate
  is 300 bps. Never treat these constants as pricing truth.
- Hourly invoices use **cumulative-delta** commission
  (`HourlyPeriod::marginal_commission`), so the sum of per-invoice commissions equals the
  single-shot commission for the same total. (`release_partial_*`, which used the same
  mechanism for fixed-price slices, was removed in v1.4.0.)
- SOL payouts always drain the **actual vault balance**, never the recorded amount
  (dust-DoS defence). The hourly SOL vault instead holds a `vault_rent_reserve` so a payout
  can never take it below rent-exempt.
- **Non-happy-path commission policy differs by engine**: fixed-price keeps the full
  remaining commission on resolve / auto-release / cancel / mutual-cancel; hourly
  `resolve_invoice_*` pro-rates commission to the paid share and refunds the remainder to
  the employer. Do not "unify" one to the other without a product decision.
- **The program has no notion of decimals — every amount is base units.** Decimals live
  off-chain (`backend/app/core/enums/payment.py`,
  `frontend/apps/dashboard/lib/solana-wallet/config.ts`): SOL 9, USDC/USDT/EURC 6. Native
  SOL is always allowed (mint pinned to `SystemProgram::ID`); SPL mints must be on
  `Config.allowed_mints` (≤ 30). USDT has no devnet mint and is aliased to USDC.

## Program ids

`declare_id!` is cfg-split in `lib.rs`: `--features mainnet` →
`HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs`; **every other build — devnet, localnet, CI,
LiteSVM tests — is `FinhtLJ4PVBwVi8tGwWoCzN3vDpcMofBZFXFzmntGxEh`.** `Anchor.toml` matches
for all three clusters, and the tests read `IDL.address`, so nothing needs a manual id.
There is no localnet/devnet id split. Addresses, keypair files and roles: the table in
`solana/CLAUDE.md`.

Two dead ids still lurk and must never be reintroduced: `6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ`
(v1, upgrade key lost, no hourly-v2 instructions) and `GDCBqN8AVU5i2xXdeTNwBmCCsd9Y8rfiH1JDKA8UjDYh`.
Both were purged from the off-chain defaults on 2026-08-17 — `scripts/verify.sh`, backend
config, the frontend fallback and all three `.env.example` files now resolve to `Finht…`,
and the backend's mainnet boot guard rejects the dead ids outright. They survive only in the
dashboard legal pages (R-13). Always pass the real id explicitly.

## Tests

Both suites are **LiteSVM, in-process**: `beforeAll` calls
`svm.addProgramFromFile(PROGRAM_ID, "target/deploy/worqen_escrow.so")`, so `anchor build`
must run first (`make test` does both). The `new Connection("http://localhost:8899")` in
each file is a dummy — Anchor is used only to *build* instructions (`.instruction()`),
never `.rpc()`. Nothing touches localnet or devnet, and clock warping makes every time gate
testable.

```bash
make test                          # anchor build && bun test (both files)
bun test tests/hourly.test.ts      # one file
bun test -t "approve_invoice"      # by test-name filter
bun run lint:fix                   # prettier over tests/, scripts/, migrations/
```

Each file carries its own copy of the harness — extend the one you are in, they do not share
a module. Both define `buildTx` (calls `svm.expireBlockhash()` first so two identical
instructions do not collide as `AlreadyProcessed`), `send`, `expectFail(ixs, signers, code)`
asserting an Anchor error name appears in the logs, `fundedKeypair`, `tokenBalance`,
account decoders over `program.coder.accounts.decode`, and the clock helpers `now()` +
`warpBy(seconds)`; `tests/hourly.test.ts` adds `warpTo(ts)`, PDA helpers
(`periodPda`/`solVaultPda`/`invoicePda`) and the `newCtx` period fixture,
`tests/escrow.test.ts` adds `pdas(escrowId)` and `ensureAta`. The SVM clock is pinned to
`BASE_TS = 1_900_000_000` in `beforeAll` — LiteSVM boots at `unixTimestamp = 0`, which
would make every `[now+3 d, now+90 d]` bound unsatisfiable.

When adding an instruction, add at least one happy-path test **and** its negative cases
(wrong signer, wrong status, boundary value) using `expectFail` with the exact
`EscrowError` variant name.

**The fixed-price authorization gap is closed — keep it closed.** `tests/escrow.test.ts` now
carries 24 `expectFail` assertions including `ReleaseNotAuthorized`, `Unauthorized`,
`InvalidStatus`, `EmployerCancelAfterFundedDisallowed`, `AuthorityRotationDuringDispute`,
`InvalidEmployeeShare`, `EscrowNotTerminal`, `SelfPaymentNotAllowed`,
`InvalidCommissionRate`, `InvalidFeeRecipient`, plus the v1.4.0 additions
`InvalidPlatformAuthority` (6065) and `InvalidEscrowKind` (6067) and the v1.5.0
missing-destination-ATA negative (`AccountNotInitialized`, 3012). Suite total is **103**
(46 escrow + 57 hourly). Any change to a fixed-price money path should add the matching
negative test rather than assume the constraint holds.

**LiteSVM is not the only harness.** `backend/scripts/e2e/` drives the *backend's own*
instruction builders against a real `solana-test-validator` and a real migrated Postgres
(13 scenarios; `README.md` has the invocation). Scenario 10 replays backend-built
instructions inside LiteSVM specifically because a live validator cannot warp its clock —
that is how the `close_period` `rent_payer` bug and the 30-day `trigger_auto_release_*` gate
were proved. When you change an instruction the backend builds, run that harness too.

## Deploy and ops

```bash
make build-deploy       # cargo build-sbf --arch v3 --tools-version v1.55 → target/deploy-v3/ (the DEPLOYABLE .so)
make sizes              # both artifacts vs the 700,040 B mainnet ProgramData allocation
make deploy-devnet      # build-deploy + anchor upgrade target/deploy-v3/worqen_escrow.so (needs Agave >= 4.0.0)
make idl-devnet         # anchor idl upgrade/init → Program Metadata Program (republish every deploy)
make verify-devnet      # scripts/verify.sh — solana-verify reproducible Docker build + verify-from-repo
make config-status RPC_URL=...                                   # read Config: paused, authority, treasury, allowlist
make pause   RPC_URL=... AUTHORITY_KEYPAIR=...                   # emergency kill-switch (blocks new money only)
make unpause RPC_URL=... AUTHORITY_KEYPAIR=...
make bootstrap-config RPC_URL=... AUTHORITY_KEYPAIR=... FEE_RECIPIENT=... ALLOWED_MINTS=...
bun scripts/set-platform-authority.ts                            # required before any open_period works
```

`bootstrap-config.ts` is idempotent (`init_config` + allowlist reconcile) and never silently
changes `fee_recipient` or `authority`. Since v1.5.0 it **also creates the treasury ATA for
every allowlisted mint** — mandatory, because no instruction creates it any more (devnet
already has `5ViqKdpw…` USDC and `C72ZqNq8…` EURC). `pause.ts` is just
`update_config(paused=…)` signed by `Config.authority`.

- **Two artifacts, and the deploy takes the one the tests do not.** `anchor build` →
  `target/deploy/worqen_escrow.so` (default arch, LiteSVM + IDL); `make build-deploy` →
  `target/deploy-v3/worqen_escrow.so` (SBPFv3, −154 KB, every cluster upload). Never
  `solana program deploy` the first one.
- **Devnet** is a manual path: `make deploy-devnet` locally (needs an **Agave ≥ 4.0.0** CLI on
  PATH — a 3.x client rejects the v3 file with `ELF error: invalid file header` before it ever
  reaches the cluster), or the `deploy.yml` `workflow_dispatch` (devnet only — `anchor upgrade`
  + `anchor idl upgrade` + a LiteSVM smoke run, keypair from the `devnet` GH environment
  secret, shredded afterwards). Then `make idl-devnet`, then `bun run generate:client`. Update
  `devnet-deployment.json` when the deployed artifact changes.
- **The on-chain IDL lives in the Program Metadata Program** (`D5EDchbf…`), not a legacy
  Anchor account — 1.1.2 dropped the `__idl_*` handlers. Publication is multi-transaction and
  is easily throttled by public-RPC 429s; if `anchor idl fetch` + zlib decompress shows a
  truncated stream, just re-run `make idl-devnet`.
- **Mainnet is never auto-deployed.** `release.yml` fires on a `v*` tag: `solana-verify
  build -- --features mainnet`, publishes the `.so`/IDL/hashes as a GitHub Release, then
  (behind the protected `mainnet-beta` environment) writes an upgrade **buffer** and hands
  its authority to the Squads multisig for manual execution.
- Release profile (`Cargo.toml`): `overflow-checks = true`, `lto = "fat"`,
  `codegen-units = 1`. Toolchain: rust `stable`, Anchor 1.1.2, Solana CLI 4.0.0.
- Push to the `worqen-solana` remote, not `origin` (which still points at the legacy
  `Worqen-Escrow` repo). Default branch `master`.

## Cross-repo sync checklist

Run through this whenever the IDL changes.

| Changed | Must also update |
|---|---|
| Any instruction, account, arg, error or event | `bun run generate:client` → commit `frontend/apps/dashboard/lib/solana-wallet/generated/`, **and** refresh `backend/tests/fixtures/worqen_escrow_idl.json` |
| A field in `state/escrow.rs` | `backend/app/services/solana.py:152-162` byte offsets (employer 75, employee 107, platform_authority 139, status 230, employer_confirmed 231, employee_confirmed 232, funded_at 241, employee/employer_share_resolved 641/649) — correct today, silently corrupted by any insert or reorder |
| A field in `state/hourly.rs` | `backend/app/services/hourly/onchain.py` `_PERIOD_LAYOUT` — hand-written offsets. It was missing `vault_rent_reserve` until 2026-08-17, which shifted everything from `invoice_count` on by 8 bytes; the backend now reads `invoice_count` from chain to pick the invoice PDA index, so a repeat of that bug breaks staging immediately rather than latently |
| A hourly instruction's args, accounts or seeds | `backend/app/services/hourly/instructions.py` (hand-built discriminators + seeds) and the **vendored IDL** at `backend/tests/fixtures/worqen_escrow_idl.json` — refresh it after `anchor build`, or `backend/tests/test_idl_fixture_sync.py` fails on drift |
| `EscrowStatus`, `EscrowKind` or `HourlyStatus` values | `backend/app/core/enums/escrow.py` — integers are 1:1 with the chain; backend adds off-chain-only `DRAFT` (escrow) and `CLOSED` (hourly period). Never reorder |
| An error variant | Append at the **end** of `errors.rs`. Anchor numbers variants positionally from 6000 — inserting renumbers every later code and breaks the generated frontend error map |
| The mint allowlist or a new token | `Config.allowed_mints` on-chain **and** the decimals tables in `backend/app/core/enums/payment.py` + `frontend/apps/dashboard/lib/solana-wallet/config.ts` |

Direction of traffic: the dashboard builds only fixed-price instructions client-side
(create/deposit/release/cancel/confirm/dispute) through the generated client; **every hourly
transaction is compiled server-side** by `backend/app/services/hourly/period_service.py`
(`_compile_unsigned`) and the browser only signs the serialized transaction — so hourly
follows the backend's `ESCROW_PROGRAM_ID`, not the frontend's. On-chain `hire_id` =
`sha256(hire UUID bytes)`; the invoice PDA index the backend passes comes from a DB
`COUNT(*)` that must equal on-chain `HourlyPeriod.invoice_count` exactly or staging fails on
the seeds constraint, permanently, for that hire-week.

## Common pitfalls

- Shipping a program change without `bun run generate:client` — the frontend then builds
  instructions against a stale IDL layout.
- Inserting a struct field mid-account instead of carving it from `reserved` — silently
  corrupts the backend's hardcoded byte offsets.
- Inserting an `EscrowError` variant instead of appending — renumbers every later code.
- Forgetting `anchor build` before `bun test` — the suite loads a stale
  `target/deploy/worqen_escrow.so` and tests the old program.
- Relying on the Stop hook to format Rust: it only runs prettier on this repo's TS. Run
  `make lint` yourself; CI fails on `cargo fmt --check` and clippy `-D warnings`.
- Assuming a *deployed service* speaks the deployed program. Devnet is v1.5.0 (2026-08-17), so
  the pinned arbitrator, the pause-gated deposits, the `open_period` co-signer **and the short
  token account lists** are all live — but staging's backend/frontend still run pre-campaign
  code, so their deposits, `open_period` and every token payout fail until `dev` is redeployed.
  `devnet-deployment.json` is the authority on what is deployed.
- Deploying `target/deploy/worqen_escrow.so`, or deploying with a 3.x Agave client. The first
  uploads 154 KB of dead relocation data; the second refuses the v3 file outright.
- Passing the same mutable account twice in one instruction. Anchor 1.1.2 now rejects it with
  **2040 `ConstraintDuplicateMutableAccount`** — an employer who is also the fee recipient, or
  a self-pay slot, now errors where 0.32 silently allowed it.
- Building a `_token` payout without pre-creating the destination ATA — v1.5.0 removed every
  payout-side `init_if_needed`, so it fails 3012 `AccountNotInitialized`.
- Reusing a retired error code. 6015/6018/6032/6033/6045 are `ReservedXXXX` on purpose:
  Anchor numbers positionally, so reusing one silently remaps an old client's error map.
- Adding a happy-path test only — the fixed-price suite already has that blind spot.
- Assuming `escrow.auto_release_at` pays out. It does not; only `dispute_deadline` drives a
  forced payout, via `trigger_auto_release_*`.

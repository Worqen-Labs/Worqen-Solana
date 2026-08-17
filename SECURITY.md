# Security Policy

The Worqen Escrow program holds user funds on the Solana blockchain. We take
security seriously and welcome reports from security researchers.

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Email: **security@worqen.com**

If possible, include:

- A clear description of the issue and its potential impact.
- Steps to reproduce, ideally with a minimal proof-of-concept.
- The affected program ID, instruction(s), and account schema version.
- Your name / handle for acknowledgement (optional).

We will acknowledge your report within **72 hours** and aim to provide an
initial assessment within **7 days**. Coordinated public disclosure
follows fix-and-deploy on the affected cluster.

## Scope

In scope:

- The Rust program source in `programs/worqen-escrow/` — 47 instructions across the
  fixed-price/milestone `Escrow` engine and the hourly v2 `HourlyPeriod` +
  `HourlyInvoice` engine (v1.5.0, Anchor 1.1.2 — the version devnet runs).
- The deployed program on Solana devnet:
  `FinhtLJ4PVBwVi8tGwWoCzN3vDpcMofBZFXFzmntGxEh` (worqen-escrow v1.5.0, account
  schema version 2, deployed 2026-08-17 at slot 484755229, on-chain bytes
  sha256 `73ea91040ce236e8486fdcd509540f8e3507cf524547d576a307653bc9050c65`).
  Its mainnet counterpart id
  `HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs` (built with `--features mainnet`)
  is in scope for source review; nothing is deployed there yet.
- Issues that allow:
  - Theft, freezing, or loss of escrowed funds.
  - Bypassing authorization checks (employer / employee / platform).
  - Bricking instructions or accounts (denial of service via on-chain state).
  - Incorrect commission accounting.
  - Replay or double-spend on terminal escrow states.

Out of scope:

- Superseded prior deployments — the `6Ftag…` program (worqen-escrow v1.0.0–v1.1.0,
  devnet, replaced by `Finht…` in v1.2.0 after its upgrade-authority key was lost, so
  it can never be patched), and the older `GDCB…` (v2) / `GVST…` (v1.1) programs that
  predate the `worqen-escrow` rewrite. None hold new funds.
- Off-chain components (frontend, backend RPC, indexers) — those have
  separate disclosure channels.
- Issues that require a malicious validator, root-level wallet
  compromise, or social engineering of the platform's upgrade
  authority — outside the on-chain trust boundary.
- Findings that depend on Solana runtime bugs (report those to
  [Anza](https://github.com/anza-xyz/agave/security)).

## Disclosure Process

1. Report received → acknowledged within 72h.
2. We reproduce and assess severity (Critical / High / Medium / Low).
3. Fix is developed on a private branch with the reporter looped in.
4. Fix is deployed to devnet, then audited internally; mainnet
   deployment follows external review.
5. Public disclosure with credit to the reporter (unless they request
   anonymity).

## Bug Bounty

A formal bounty program is **not yet active**. We do, however, offer
discretionary rewards for high-quality reports of in-scope issues,
sized to severity. Once mainnet is live, a public bounty will replace
this discretionary tier.

## Past Audits

| Date       | Auditor          | Scope                                 | Report      |
|------------|------------------|---------------------------------------|-------------|
| 2026-05-29 | Internal (multi-agent) | worqen-escrow v1 — all 33 instructions + account/Config schema | On request  |
| 2026-06-01 | Internal (pre-mainnet) | Commission-retention v1.1.0 + money-path review | On request  |
| TBD        | External               | mainnet candidate                     | _Pending_   |

## Security Architecture

Key properties enforced by the on-chain program:

- **PDA isolation** — every escrow has its own vault PDA; the program
  is the only signer that can move funds.
- **Constraint-based validation** — Anchor's `#[account]` constraints
  enforce status, mint, owner, and PDA-seed checks before the handler
  body runs.
- **Drain-actual-balance** — SOL outflows clear the actual vault
  balance, not the recorded amount, defeating dust-DoS attacks that
  would otherwise leave the vault below rent-exempt minimum.
- **Mint + owner gates** — every SPL token destination is constrained
  on both `mint` and `owner`, preventing redirection attacks.
- **Bounded time gates** — `auto_release_at` capped at 1 year,
  `dispute_deadline` capped at 90 days; both required to be in the
  future at write time.
- **No direct lamport manipulation** — every transfer goes through a
  System Program or SPL Token CPI, with PDA signer-seeds.
- **Reproducible builds** — pinned toolchain plus `solana-verify`
  registry submission; see [README.md](./README.md#9-build-test-deploy) for
  the verification procedure (`make verify-devnet` / `scripts/verify.sh`).

## On-Chain Security.txt

The deployed program embeds a [`solana-security-txt`][sst] section in
its `.so` so wallets and explorers can surface this policy directly.
View it on Solscan:

<https://solscan.io/account/FinhtLJ4PVBwVi8tGwWoCzN3vDpcMofBZFXFzmntGxEh?cluster=devnet>

[sst]: https://github.com/neodyme-labs/solana-security-txt

## v1.5.0 — Destination token accounts are the caller's responsibility (live on devnet 2026-08-17)

The 18 non-vault `associated_token::mint` / `associated_token::authority`
constraints are replaced by the `owner` + `mint` idiom the program already used
for `platform_token_account`, and every `init_if_needed` on a payout destination
is gone. The three **vault** ATA constraints (`deposit_token`,
`fund_period_token`, `settle_period_token`) are untouched — there the canonical
derivation *is* the security binding.

What the program now guarantees about a destination token account is exactly:
`owner == <employee | employer | fee_recipient from the escrow/period>` and
`mint == <escrow/period mint>`. It no longer requires the canonical ATA address,
and it never creates the account. **A missing destination makes the instruction
fail with `AccountNotInitialized` instead of self-healing**, so every caller must
prepend an idempotent ATA create (the backend's `ensure_token_accounts`), and
`scripts/bootstrap-config.ts` now creates the treasury ATA per allowlisted mint.

This removes the accidental subsidy where the platform hot key silently paid
~0.00204 SOL of unrecoverable rent for each new worker ATA, and drops the
`token_mint` / pure-derivation `employee` / `employer` / `fee_recipient` /
`associated_token_program` / `system_program` slots that only existed to feed
those constraints — a breaking account-list change on 10 instructions (46 slots
removed in total). Account **structs** and discriminators are unchanged, so no
stored state migrated.

v1.5.0 also moved the program from Anchor 0.32.1 to **1.1.2**, which adds a
built-in duplicate-mutable-account guard (**runtime error 2040
`ConstraintDuplicateMutableAccount`**) and removes the legacy `__idl_*` handlers
— the on-chain IDL now lives in the Program Metadata Program account
`D5EDchbfDVyCfgF1SmVTXutyDAiSU4R5ZYWyH3urwXZC`. The deployed artifact is
**SBPFv3** (751,720 B, `target/deploy-v3/`), which requires an Agave ≥ 4.0.0
client to upload; the LiteSVM suite (103 tests) exercises the default-arch build
of identical source.

## v1.1.0 — Commission retained on non-happy-path settlements (2026-06-01)

Earlier versions refunded the platform commission to the employer on dispute
resolution, auto-release, cancellation, and mutual cancellation, so the platform
had no financial incentive to stall a dispute. **As of v1.1.0 this is reversed
by product decision:** the platform retains its full commission on all of these
paths — routed to the treasury (`fee_recipient`), never returned to the employer.
Freelancers are unaffected (they receive exactly the amount awarded).

This is a **breaking instruction-signature change**: `resolve_dispute_sol/token`,
`trigger_auto_release_sol/token`, `cancel_escrow_sol/token`, and
`mutual_cancel_sol/token` now require the `fee_recipient` account (SOL) or
`fee_recipient` + `platform_token_account` (token). All off-chain instruction
builders (backend custodial signing + `escrow.py`, frontend `escrow-program.ts`)
must pass them in IDL order, and any path that creates the treasury ATA must do
so idempotently. The anti-stall property is now preserved operationally rather
than by code (the platform fee no longer depends on the dispute outcome).

## Operational kill-switch (pause)

The Config PDA carries a `paused` flag. The gate is structural: `!config.paused` is
checked by exactly the instructions that take the `Config` account, and by all of them.
That is **15 instructions** (14 `require!` sites — `stage_invoice_sol` and
`stage_invoice_token` share `stage_common`):

| Gated instruction | Why |
|---|---|
| `create_escrow` | new escrow intake |
| `deposit_sol` / `_token` | new money into an existing escrow vault |
| `pay_with_commission_sol` / `_token` | new money, no escrow |
| `batch_pay_with_commission_sol` / `_token` | same, fanned out |
| `open_period` | new hourly period |
| `fund_period_sol` / `_token` | hourly intake |
| `raise_weekly_cap` | raises the ceiling on future hourly intake |
| `stage_invoice_sol` / `_token` | new earmark against a period vault |
| `approve_invoice_sol` / `_token` | payout on a counterparty's say-so, no time lock |

Since v1.4.0 `deposit_sol` and `deposit_token` carry the `Config` account and check the
flag, closing the hole where a paused program still accepted money into an
already-created escrow (R-34). `deposit_more_sol` / `deposit_more_token` no longer
exist — v1.4.0 removed them along with `release_partial_sol` / `_token` and
`close_unfunded_escrow_sol`, none of which had any caller. The invariant is now
"pause = no new money enters any vault, no new obligation is created, and nothing gets
paid out early".

Pause can **never** block `release`, `confirm`, `dispute`, `resolve`,
`auto_release`, `close`, `mutual_cancel`, `finalize_invoice_*`,
`settle_period_*`, or `close_period_*` — so it can never strand funds already in
escrow; every party can still withdraw. `tests/hourly.test.ts` asserts this
explicitly.

`approve_invoice_*` is deliberately inside the gate even though it is a payout
path. It is the only payout callable at will by a non-platform party with no
time lock, so leaving it open would let a compromised employer (or the backend
key that platform-signs it on the employer's behalf) drain every staged invoice
in one slot and erase the platform's freeze window. Gating it costs nothing:
the same invoice still pays out through the pause-free `finalize_invoice_*`
once `release_at` passes, so a pause only removes the ability to *skip* the
review window.

Operate it with the `Config.authority` key (under the hood: `scripts/pause.ts`,
an `update_config(paused=…)` call):

```bash
make config-status RPC_URL=https://<rpc>                                    # read state, no key
make pause   RPC_URL=https://<rpc> AUTHORITY_KEYPAIR=~/config-authority.json  # EMERGENCY stop
make unpause RPC_URL=https://<rpc> AUTHORITY_KEYPAIR=~/config-authority.json  # resume
```

Rehearse on devnet before mainnet so the response is muscle memory.

## Mainnet key custody & authority split

Four roles with four risk profiles — never collapse them into one key:

| Role | Can do | Worst case if leaked | Mainnet custody |
|---|---|---|---|
| **Upgrade authority** | Replace program bytecode | Total loss of all escrowed funds | **2-of-3 Squads multisig** (or HSM). Rarely used; M-of-N friction is fine. |
| **Config authority** | Pause, set treasury + default bps, hand off authority | Grief (pause) + redirect *future* commission; **cannot drain principal or upgrade** | Fast key you can `make pause` with in seconds — optimize for response speed. |
| **Platform authority** | Per-escrow resolve / force-release / auto-release | Move funds only within the dispute/release rules of funded escrows | Hot backend key in a secrets manager / KMS; monitor its activity. |
| **fee_recipient (treasury)** | Receive commission; **never signs** | — | Cold / receive-only or multisig wallet. |

`fee_recipient` is snapshotted onto each escrow at create time, so changing
`Config.fee_recipient` affects only *future* escrows — a compromised Config
authority cannot retroactively reroute money already in flight.

Bootstrap with a deployer key, then hand the Config authority to the multisig
(two-step), and move the upgrade authority separately:

```bash
make bootstrap-config RPC_URL=https://<rpc> AUTHORITY_KEYPAIR=~/deployer.json \
  FEE_RECIPIENT=<treasury> ALLOWED_MINTS=<usdc>,<usdt>,<eurc>

# two-step Config authority handoff
#   update_config(new_pending_authority = <squads>)   # propose (current authority)
#   accept_authority(<squads>)                          # accept  (the multisig)

# upgrade authority handoff
solana program set-upgrade-authority <program-id> --new-upgrade-authority <squads>
```

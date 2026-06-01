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

- The Rust program source in `programs/worqen-escrow/`.
- The deployed program on Solana devnet:
  `6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ` (worqen-escrow v1.1.0).
- Issues that allow:
  - Theft, freezing, or loss of escrowed funds.
  - Bypassing authorization checks (employer / employee / platform).
  - Bricking instructions or accounts (denial of service via on-chain state).
  - Incorrect commission accounting.
  - Replay or double-spend on terminal escrow states.

Out of scope:

- Superseded prior deployments — the `GDCB…` (v2) and `GVST…` (v1.1)
  programs predate this `worqen-escrow` rewrite and hold no new funds.
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
  registry submission; see [README.md](./README.md#verification) for
  the verification procedure.

## On-Chain Security.txt

The deployed program embeds a [`solana-security-txt`][sst] section in
its `.so` so wallets and explorers can surface this policy directly.
View it on Solscan:

<https://solscan.io/account/6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ?cluster=devnet>

[sst]: https://github.com/neodyme-labs/solana-security-txt

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

The Config PDA carries a `paused` flag. When set, the program rejects every
instruction that brings *new* money into the system — `create_escrow`,
`deposit_*`, and `pay_with_commission_*` (incl. the batch variants). It can
**never** block `release`, `confirm`, `dispute`, `resolve`, `auto_release`,
`close`, or `mutual_cancel`. Pausing therefore halts intake without ever
stranding funds already in escrow — every party can still withdraw.

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

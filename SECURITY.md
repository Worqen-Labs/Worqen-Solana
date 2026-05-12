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
  `GDCBqN8AVU5i2xXdeTNwBmCCsd9Y8rfiH1JDKA8UjDYh`.
- Issues that allow:
  - Theft, freezing, or loss of escrowed funds.
  - Bypassing authorization checks (employer / employee / platform).
  - Bricking instructions or accounts (denial of service via on-chain state).
  - Incorrect commission accounting.
  - Replay or double-spend on terminal escrow states.

Out of scope:

- The legacy v1.1 program at
  `GVST6WJqsj1BmFSRy1a9Xi2DK8BZtzjiFGkjQCRSmaUW` (deprecated; in-flight
  v1 escrows only).
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
| 2026-05-03 | Internal (v1→v2) | All 20 instructions, account schema   | On request  |
| TBD        | External         | v2 mainnet candidate                  | _Pending_   |

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

<https://solscan.io/account/GDCBqN8AVU5i2xXdeTNwBmCCsd9Y8rfiH1JDKA8UjDYh?cluster=devnet>

[sst]: https://github.com/neodyme-labs/solana-security-txt

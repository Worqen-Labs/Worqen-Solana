# Worqen Escrow

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Anchor](https://img.shields.io/badge/Anchor-0.32.1-9945FF.svg)](https://www.anchor-lang.com/)
[![Solana](https://img.shields.io/badge/Solana-1.18%2B-14F195.svg)](https://solana.com/)
[![Version](https://img.shields.io/badge/version-1.0.0-brightgreen.svg)](#)
[![Schema](https://img.shields.io/badge/account_schema-v2-blue.svg)](#)

Worqen Escrow is a trustless, three-party payment escrow protocol for the
[Worqen](https://worqen.com) job marketplace, deployed on Solana.

> Employers lock funds when hiring. Workers are paid only after confirmed
> delivery. Disputes are mediated by the platform, with a permissionless
> deadline-based fallback so funds never get stuck.

- **Status:** v1.0 — devnet-deployed, audit-ready, mainnet pending external review.
- **Program ID (devnet):** [`GDCBqN8AVU5i2xXdeTNwBmCCsd9Y8rfiH1JDKA8UjDYh`](https://solscan.io/account/GDCBqN8AVU5i2xXdeTNwBmCCsd9Y8rfiH1JDKA8UjDYh?cluster=devnet)
- **Legacy v1.1 ID:** `GVST6WJqsj1BmFSRy1a9Xi2DK8BZtzjiFGkjQCRSmaUW` (still live for in-flight v1 escrows)
- **Security contact:** [`security@worqen.com`](mailto:security@worqen.com) — see [SECURITY.md](./SECURITY.md)

---

## Table of Contents

- [Why this exists](#why-this-exists)
- [Features](#features)
- [How it works](#how-it-works)
- [Repository layout](#repository-layout)
- [Build & test](#build--test)
- [Deploy](#deploy)
- [Verification](#verification)
- [Security](#security)
- [License](#license)

---

## Why this exists

Freelance and gig-marketplace payments are bilateral by default: the
employer pays directly, or the platform holds the money in a custodial
bank account. Both sides have problems:

- **Direct pay** leaves the worker exposed to non-payment after delivery.
- **Custodial escrow** moves the entire trust burden onto the platform —
  the worker's money sits on the platform's balance sheet, subject to
  the platform's solvency, freezes, and reversible payment rails.

Worqen Escrow puts the funds on-chain in a Program Derived Address
(PDA) that **no individual party can move unilaterally**. The platform
mediates disputes but cannot withdraw worker funds, and a built-in
deadline guarantees the funds are eventually paid even if the platform
disappears. Workers can be paid in USDC, USDT, SOL, or any SPL token,
including 0-SOL workers (the program initializes their token account
on demand).

## Features

- **Three-party design** — employer, worker, platform_authority. All
  three are distinct addresses, enforced at create time.
- **Native SOL and SPL tokens** — every flow has matching `_sol` and
  `_token` variants. Token destinations are constrained on both `mint`
  and `owner` to prevent redirection.
- **Configurable commission** — `commission_rate_bps`, basis points, max
  10% (1000 bps). Held in the vault, paid to the platform on release,
  refunded to the employer on dispute resolution.
- **Milestone groups** — link related escrows by `escrow_group_id` and
  `sequence_in_group`/`total_in_group` for hires with multiple
  deliverables.
- **Partial releases** — pay slice-by-slice. Cumulative-delta commission
  math guarantees sum-of-slices == single-release commission for the
  same total.
- **Mandatory dispute deadline** — every dispute carries a deadline (≤90
  days). After the deadline, **anyone** can force-resolve the dispute,
  guaranteeing the worker gets paid even if the platform goes silent.
- **Auto-release for stuck escrows** — optional `auto_release_at` (≤1
  year) for non-disputed escrows.
- **Direct-pay path** — `pay_with_commission_*` for trusted hires / tips
  / invoice settlement: atomic split-pay with no escrow state.
- **Rent recovery** — `close_escrow_*` reclaims the ~0.005 SOL storage
  rent (and ~0.002 SOL token-vault rent) on terminal escrows.
- **Embedded `solana-security-txt`** — disclosure policy and security
  contact are baked into the on-chain bytecode and surfaced by Solscan
  and Solana Explorer.

## How it works

```
                            ┌─────────────────────────────┐
                            │ create_escrow + deposit_*   │
   ┌───────────┐            ▼            ┌──────────────┐
   │  Created  │  ─────────────────►     │   Funded     │
   └─────┬─────┘                         └───────┬──────┘
         │ cancel (employer)                     │
         ▼                                       │ confirm_completion
   ┌───────────┐                                 ▼
   │ Cancelled │  ◄── cancel (platform) ── ┌─────────────────┐
   └───────────┘                           │ PendingRelease  │
                                           └────────┬────────┘
              raise_dispute (employer/worker)       │ release_* (employer/platform/worker)
                                ▼                   ▼
                            ┌──────────┐       ┌──────────┐
                            │ Disputed │       │ Released │
                            └─────┬────┘       └──────────┘
              resolve_dispute_*   │ trigger_auto_release_* (after deadline, anyone)
                       │          ▼
                       │     ┌──────────┐
                       └────►│ Resolved │
                             └──────────┘

  Terminal (Released | Resolved | Cancelled) → close_escrow_* → account closed, rent refunded
```

## Repository layout

```
.
├── programs/worqen-escrow/        # Anchor program crate
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                  # #[program] entrypoint, 20 instructions
│       ├── errors.rs               # EscrowError 6000–6033
│       ├── events.rs               # 8 #[event] structs
│       ├── state/
│       │   └── escrow.rs           # Escrow account (v2), PDA seeds, math
│       └── instructions/           # One file per instruction handler
│           ├── create_escrow.rs
│           ├── deposit_{sol,token}.rs
│           ├── confirm_completion.rs
│           ├── release_{sol,token}.rs
│           ├── release_partial_{sol,token}.rs
│           ├── raise_dispute.rs
│           ├── resolve_dispute_{sol,token}.rs
│           ├── cancel_escrow_{sol,token}.rs
│           ├── trigger_auto_release_{sol,token}.rs
│           ├── update_platform_authority.rs
│           ├── close_escrow_{sol,token}.rs
│           └── pay_with_commission_{sol,token}.rs
├── tests/worqen-escrow.ts         # Anchor / Mocha test suite
├── scripts/                       # Local integration scripts
├── migrations/
├── Anchor.toml
├── Cargo.toml                     # workspace
├── rust-toolchain.toml            # pinned for reproducible builds
├── devnet-deployment.json         # canonical devnet deployment manifest
├── SECURITY.md                    # disclosure policy
├── LICENSE                        # Apache-2.0
└── README.md
```

## Build & test

Prerequisites:

- [Rust](https://rustup.rs/) — version pinned by `rust-toolchain.toml` (1.79.0)
- [Solana CLI](https://docs.solanalabs.com/cli/install) 1.18+
- [Anchor CLI](https://www.anchor-lang.com/docs/installation) 0.32.1
- Node.js 18+ and either `yarn` or `bun`

```bash
# Install JS deps
bun install   # or: yarn install

# Build the program (release with overflow checks + LTO)
anchor build

# Run the full test suite (spawns a local validator)
anchor test

# Run against an existing local validator
solana-test-validator                # terminal 1
anchor test --skip-local-validator   # terminal 2

# Lint TypeScript
yarn lint        # check
yarn lint:fix    # apply
```

Anchor.toml pins the test runner to `ts-mocha` with a 1,000,000ms
timeout. Tests live in `tests/worqen-escrow.ts`.

## Deploy

```bash
# Devnet
anchor build
solana config set --url devnet
anchor deploy --provider.cluster devnet

# Push the IDL to the deployed program (off-chain clients consume this)
anchor idl init --provider.cluster devnet --filepath target/idl/worqen_escrow.json $(solana address -k target/deploy/worqen_escrow-keypair.json)
# Or, upgrading an existing IDL:
anchor idl upgrade --provider.cluster devnet --filepath target/idl/worqen_escrow.json $(solana address -k target/deploy/worqen_escrow-keypair.json)
```

For mainnet, the upgrade authority should live on a hardware wallet or
in a multisig — never a hot keypair. The current devnet upgrade
authority is documented in [`devnet-deployment.json`](./devnet-deployment.json).

## Verification

The program is built reproducibly so anyone can confirm the on-chain
bytecode matches this repository. We use
[`solana-verify`][solana-verify] — the official tool maintained by the
Anza foundation and the OtterSec registry.

```bash
# Install
cargo install solana-verify

# Build reproducibly (uses the official solanafoundation/solana-verifiable-build Docker image)
solana-verify build

# Verify the deployed program against this repository
solana-verify verify-from-repo \
  --url devnet \
  --program-id GDCBqN8AVU5i2xXdeTNwBmCCsd9Y8rfiH1JDKA8UjDYh \
  --library-name worqen_escrow \
  https://github.com/worqen-labs/worqen-escrow
```

A successful verification submits the proof to the OtterSec verified
build registry; Solscan and Solana Explorer then display **"Program is
verified"** on the program page.

The on-chain bytecode also embeds a `solana-security-txt` section with
the project name, security contact (`security@worqen.com`), and a URL
to this repository's [SECURITY.md](./SECURITY.md). Explorers surface
this automatically.

[solana-verify]: https://github.com/Ellipsis-Labs/solana-verifiable-build

## Security

**Do not open a public issue for security reports.** Email
[`security@worqen.com`](mailto:security@worqen.com). See [SECURITY.md](./SECURITY.md)
for scope, expected response times, and disclosure process.

Key hardening in v2 (relative to v1):

- Token destinations constrained on both `mint` and `owner`.
- SOL drain-actual-balance defeats 1-lamport dust DoS.
- `init_if_needed` for employee ATAs unblocks 0-SOL workers.
- Employer cannot unilaterally cancel after `Funded`.
- `dispute_deadline` mandatory and bounded (≤90 days).
- Authority rotation blocked while `Disputed`.
- `close_escrow_*` reclaims rent on terminal escrows.

## License

Apache License 2.0. See [LICENSE](./LICENSE).

Copyright © 2026 Worqen OÜ.

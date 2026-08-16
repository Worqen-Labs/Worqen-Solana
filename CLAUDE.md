# CLAUDE.md (solana)

Worqen on-chain escrow program. Anchor/Rust. This checkout's GitHub home is `Worqen-Solana` (default branch `master`); the local remote may still be named after the old `Worqen-Escrow` repo — verify with `git remote -v` and push to the `worqen-solana` remote.

## What this program does

Escrow for the Worqen marketplace: fixed-price escrows (native SOL + SPL tokens: USDC, USDT, EURC), milestone escrows, and hourly v2 (`HourlyPeriod` + per-invoice `HourlyInvoice` PDAs, 19 instructions) — pre-funded weekly escrow; each invoice gets a 7-day review window with permissionless finalize, employer/platform instant approve (approve_invoice_*), and either-party dispute. v1.3.0.

## Working here

- `Anchor.toml` / `Makefile` / `scripts/` drive build + test; `tests/` is the TS test suite; `devnet-deployment.json` records the devnet deploy.
- The frontend `@solana/kit` client at `frontend/apps/dashboard/lib/solana-wallet/generated/` is codama-generated from `target/idl/worqen_escrow.json` — rerun `bun run generate:client` after every program change.
- Backend submits/verifies via its Solana service layer; escrow state transitions must stay in sync with `backend` enums.
- The generic `/solana-dev` skill covers Anchor/testing patterns; `SECURITY.md` lists program-specific invariants.
- Lint gate: this repo is prettier-formatted (the workspace Stop hook runs `prettier --check` on touched TS here, not biome).

## Devnet keys (v2 redeploy, 2026-08-08)

The original v1 program `6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ` on devnet has upgrade
authority `14AZtBTAyX9E9GsYryS5BEUT62mDeSDiwHkoKs9rEaSk`, whose keypair is **NOT on this
machine** (it stayed with the original macOS dev checkout). We therefore cannot upgrade `6Ftag`
in place; hourly v2 deploys to devnet as a **new program id** with keys we control:

| role | address | keypair file (secret — never commit the bytes) |
|---|---|---|
| v2 program id (devnet/localnet) | `FinhtLJ4PVBwVi8tGwWoCzN3vDpcMofBZFXFzmntGxEh` | `~/.config/solana/worqen-escrow-v2-program.json` |
| v2 upgrade authority | `Gg5L88vFoL32Dw64qXX4SirD8SHPfCjJEqm3Qrjjh6zz` | `~/.config/solana/devnet-escrow.json` |
| deployer / fee payer | `MPq6BwTsfBNmA7DwdaRGLeX8Bg67Kj5sFwsiYDexste` | `~/.config/solana/id.json` |
| platform ops key (backend ESCROW_WALLET) | `64PF1jbXinCFteyegYpkPJ25fHKibPeVGJsjmc4AH46H` | backend `.env` ESCROW_WALLET_PRIVATE_KEY |

To deploy hourly v2 to devnet with the new id: set the non-mainnet `declare_id!` and
`Anchor.toml [programs.devnet]/[localnet]` to `Finht…`, rebuild, `solana program deploy
--program-id ~/.config/solana/worqen-escrow-v2-program.json --upgrade-authority
~/.config/solana/devnet-escrow.json`, then repoint backend `ESCROW_PROGRAM_ID` + frontend
`NEXT_PUBLIC_ESCROW_PROGRAM_ID` to `Finht…`, regenerate the codama client, `init_config`, and
`update_config(new_platform_authority = 64PF1…)` before any hourly period can open. Fund the
deployer + platform + QA employer/employee on devnet (faucet or manual — no auto-fill).
Localnet keeps using `6Ftag` via `--bpf-program` (no id change needed there).

## Hard rules

Same as workspace: no code comments, no AI attribution on commits, conventional commit messages, push only when asked.

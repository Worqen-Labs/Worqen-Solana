# CLAUDE.md (solana)

Worqen on-chain escrow program. Anchor/Rust. This checkout's GitHub home is `Worqen-Solana` (default branch `master`); the local remote may still be named after the old `Worqen-Escrow` repo — verify with `git remote -v` and push to the `worqen-solana` remote.

## What this program does

Escrow for the Worqen marketplace: fixed-price escrows (native SOL + SPL tokens: USDC, USDT, EURC), milestone escrows, and hourly GWS (`HourlyPeriod`, 11 instructions) — pre-funded weekly escrow with ≤7 frozen 7-day tranches per week and permissionless finalize. v1.1.0 retains commission handling.

## Working here

- `Anchor.toml` / `Makefile` / `scripts/` drive build + test; `tests/` is the TS test suite; `devnet-deployment.json` records the devnet deploy.
- Frontend consumes this program via `frontend/apps/dashboard/lib/solana-wallet/` (IDL copy at `lib/solana-wallet/idl/worqen_escrow.ts` — regenerate/copy after program changes or the frontend types lie).
- Backend submits/verifies via its Solana service layer; escrow state transitions must stay in sync with `backend` enums.
- The generic `/solana-dev` skill covers Anchor/testing patterns; `SECURITY.md` lists program-specific invariants.
- Lint gate: this repo is prettier-formatted (the workspace Stop hook runs `prettier --check` on touched TS here, not biome).

## Hard rules

Same as workspace: no code comments, no AI attribution on commits, conventional commit messages, push only when asked.

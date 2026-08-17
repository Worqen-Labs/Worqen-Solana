# CLAUDE.md (solana)

Worqen on-chain escrow program. Anchor/Rust. This checkout's GitHub home is `Worqen-Solana` (default branch `master`); the local remote may still be named after the old `Worqen-Escrow` repo — verify with `git remote -v` and push to the `worqen-solana` remote.

## What this program does

Escrow for the Worqen marketplace: fixed-price escrows (native SOL + SPL tokens: USDC, USDT, EURC), milestone escrows, and hourly v2 (`HourlyPeriod` + per-invoice `HourlyInvoice` PDAs, 19 instructions) — pre-funded weekly escrow; each invoice gets a 7-day review window with permissionless finalize, employer/platform instant approve (approve_invoice_*), and either-party dispute. Source is **v1.5.0** — 47 instructions (v1.4.0 dropped the 5 callerless ones: `release_partial_sol/_token`, `deposit_more_sol/_token`, `close_unfunded_escrow_sol`), `create_escrow` pins `platform_authority` to Config, `deposit_sol/_token` are pause-gated, and `open_period` needs a `platform_signer` co-signature. **Devnet runs v1.5.0 too** — deployed 2026-08-17 at slot **484755229** (tx `64q6nFoc…`), on-chain bytes sha256 `73ea9104…`, byte-identical to `target/deploy-v3/worqen_escrow.so` (751,720 B). v1.5.0 is the size/CU campaign: the ATA-constraint diet (18 non-vault `associated_token::` constraints → `token::mint`/`token::authority` checks; **46 account slots removed across 10 token instructions**; `init_if_needed` dropped on every payout destination), Anchor 0.32.1 → **1.1.2**, and an SBPFv3 deploy artifact. Instruction count, account structs and discriminators are unchanged — only the account *lists* of those 10 token instructions shrank.

**The program no longer creates destination token accounts.** Only the three *vault* ATAs (`deposit_token`, `fund_period_token`, `settle_period_token`) still self-create; every payout destination must already exist or the instruction fails `AccountNotInitialized` (3012). Callers prepend idempotent creates — backend `ensure_token_account_ixs`, frontend `buildEnsureTokenAccountInstructions`, and `scripts/bootstrap-config.ts` for the treasury ATA per allowlisted mint (already done on devnet: `5ViqKdpw…` USDC, `C72ZqNq8…` EURC). The program also stopped requiring the *canonical* ATA — it now checks only `owner` + `mint`, so canonicality is an off-chain concern.

## Two build artifacts — the tested one is NOT the deployed one

Since the SBPFv3 switch this repo produces **two `.so` files from the same source**, and they are
not interchangeable:

| artifact | built by | used for | bytes |
|---|---|---|--:|
| `target/deploy/worqen_escrow.so` | `anchor build` / `make build` | LiteSVM tests, IDL generation | 905,720 |
| `target/deploy-v3/worqen_escrow.so` | `make build-deploy` | **every cluster deploy** | 751,720 |

`make build-deploy` runs `cargo build-sbf --arch v3 --tools-version v1.55` directly, because the
Anchor CLI pins its own platform-tools and cannot be handed the arch flags — `anchor build` stays
in the loop only for the IDL. The v3 build drops `.rel.dyn` + `.dynsym` + `.dynstr` + `.dynamic`
**and** the relocation stubs inside `.text`: **−154,000 B (−17.0%)**, with no measurable CU change
(`init_config` 16,617 → 16,509). `make sizes` prints both against the mainnet allocation.

**Hard toolchain floor: Agave ≥ 4.0.0 on both the client and the cluster.** Measured on this
machine: `solana-cargo-build-sbf` 3.1.11 emits the v3 artifact fine, but *nothing in the 3.x
runtime accepts it* — `solana program deploy` (3.1.11) fails `ELF error: invalid file header`, a
`--bpf-program`-loaded 3.1.11 test validator answers `Program is not deployed` /
`UnsupportedProgramId`, and **LiteSVM 1.1.0 refuses it at `addProgramFromFile` with `invalid
account data for instruction`, even with `FeatureSet.allEnabled()`**. The split is SIMD-0377:
agave 3.1.11 gates v3 on `BUwGLeF3…` ("SIMD-0178, SIMD-0179 and SIMD-0189") while platform-tools
v1.55 emits the SIMD-0377 flavour that agave 4.0.0 gates on `5cC3foj7…`. On a 4.0.0 test validator
the same file deploys and runs (verified: `init_config` executed, 16,509 CU). Platform-tools
v1.52 cannot compile the v3 target at all. `SOLANA_VERSION` is pinned to `4.0.0` in all three
workflows for exactly this reason — do not lower it. A cluster that has not activated SIMD-0377
makes the CLI refuse the upload, which fails the deploy loudly instead of bricking the program.

That floor is why **the LiteSVM suite runs against the default-arch build**: the 103 tests cover
the same source, not the same bytes. `tests/*.ts` honour `WORQEN_SO_PATH` if you need to point the
suite at another artifact (which is how the v3 rejection above was measured).

It is also why **`solana-verify` output no longer matches the deployed bytes**: `solana-verify
build` pins its own container toolchain and has no `--arch` knob, so `release.yml` publishes the
reproducible default-arch `.so` (`program-hash.txt`) *and* the deployed `worqen_escrow.v3.so`
(`program-hash-v3.txt`), and the mainnet buffer carries the v3 one. On-chain source verification of
the shipped bytes is an open gap, not an oversight.

**Mainnet size ledger.** ProgramData is `45 + program_len` bytes and rent is `(128 + data_len) ×
6960` lamports. The deployed mainnet ProgramData holds 700,040 B, so v3 at 751,720 B still needs
`solana program extend HShWcYbT… 51725` ≈ **0.360 SOL** — down from the 2.084 SOL the pre-campaign
999,496-byte artifact required, but not yet zero.

## Working here

- `Anchor.toml` / `Makefile` / `scripts/` drive build + test; `tests/` is the TS test suite; `devnet-deployment.json` records the devnet deploy.
- The frontend `@solana/kit` client at `frontend/apps/dashboard/lib/solana-wallet/generated/` is codama-generated from `target/idl/worqen_escrow.json` — rerun `bun run generate:client` after every program change.
- Backend submits/verifies via its Solana service layer; escrow state transitions must stay in sync with `backend` enums.
- The generic `/solana-dev` skill covers Anchor/testing patterns; `SECURITY.md` lists program-specific invariants.
- Lint gate: this repo is prettier-formatted (the workspace Stop hook runs `prettier --check` on touched TS here, not biome).

## Program ids & keys

`declare_id!` is cfg-split (`src/lib.rs`): `--features mainnet` = `HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs`; **every other build (devnet, localnet, LiteSVM tests) = `FinhtLJ4PVBwVi8tGwWoCzN3vDpcMofBZFXFzmntGxEh`**. Anchor.toml matches for all clusters. The original v1 program `6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ` is dead — its upgrade-authority keypair (`14AZtBTAyX9E9GsYryS5BEUT62mDeSDiwHkoKs9rEaSk`) stayed on the original macOS checkout and is not recoverable, so `6Ftag` can never be upgraded and has no hourly-v2 instructions. Keys we control:

| role | address | keypair file (secret — never commit the bytes) |
|---|---|---|
| v2 program id (devnet/localnet) | `FinhtLJ4PVBwVi8tGwWoCzN3vDpcMofBZFXFzmntGxEh` | `~/.config/solana/worqen-escrow-v2-program.json` |
| v2 upgrade authority | `Gg5L88vFoL32Dw64qXX4SirD8SHPfCjJEqm3Qrjjh6zz` | `~/.config/solana/devnet-escrow.json` |
| deployer / fee payer | `MPq6BwTsfBNmA7DwdaRGLeX8Bg67Kj5sFwsiYDexste` | `~/.config/solana/id.json` |
| platform ops key (backend ESCROW_WALLET) | `64PF1jbXinCFteyegYpkPJ25fHKibPeVGJsjmc4AH46H` | backend `.env` ESCROW_WALLET_PRIVATE_KEY |

The `Finht…` devnet deploy is DONE and current (v1.5.0 since 2026-08-17, recorded in
`devnet-deployment.json`); the real backend `.env` and frontend `.env.local` already point at it.
Both upgrades are breaking for old clients — the **deployed staging services** (backend :8001,
dev.worqen.com) still run pre-campaign code, so their deposit builders (no config account),
`open_period` (no platform co-signer) **and every token-payout builder** (v1.4.0-era account lists,
now 46 slots too long) fail against devnet until the `dev` branches are pushed and redeployed;
`create_escrow` still works from old backends. As of 2026-08-17 the off-chain
defaults agree: `backend/app/core/config.py` defaults to `Finht…` on every non-mainnet network
(and its mainnet boot guard rejects `6Ftag…`/`GDCB…`/`GVST…`), the frontend falls back to `Finht…`
off-mainnet, and all three `.env.example` files advertise the right id. The dead ids survive only
in the dashboard legal pages (`docs/RISK-REGISTER.md` R-13). **Redeploys: `make deploy-devnet`** — it
builds the SBPFv3 artifact first (`make build-deploy`) and then `anchor upgrade`s `target/deploy-v3/worqen_escrow.so`
with the devnet wallet, so it needs an **Agave ≥ 4.0.0 CLI on PATH**; a 3.x client fails `ELF error:
invalid file header` before it touches the cluster. Never hand `solana program deploy` the
`target/deploy/` artifact — that is the test/IDL build. Then `make idl-devnet` (Anchor 1.x, publishes
to the Program Metadata Program) and regenerate the codama client. `Config.platform_authority`
must be set (`scripts/set-platform-authority.ts`) before any hourly period can open, and
`make bootstrap-config` must have created the treasury ATA for every allowlisted mint before any
token payout can settle. Tests never need a validator — they are LiteSVM, loading
`target/deploy/worqen_escrow.so` in-process.

**On-chain IDL now lives in the Program Metadata Program.** Anchor 1.1.2 dropped the legacy
`__idl_*` handlers, so the canonical `idl`-seed metadata account is
`D5EDchbfDVyCfgF1SmVTXutyDAiSU4R5ZYWyH3urwXZC`; `make idl-devnet` (`anchor idl upgrade`, falling back
to `init`) republishes it. Publication is a multi-transaction write and fought public-RPC 429
throttling on 2026-08-17 — if `anchor idl fetch` + zlib decompress shows a truncated stream, just
re-run `make idl-devnet` until it completes.

**Anchor 1.1.2 deploy trap — DONE 2026-08-17, kept as a historical note (and a template for
mainnet's first IDL migration).** The legacy devnet IDL account `2Y1y…` could only be closed through
the *deployed program's own* `__idl_close_account` handler, which the 1.x build no longer contains.
It was therefore closed FIRST, with the old CLI, while devnet still ran the 0.32-built v1.4.0
(`~/.avm/bin/anchor-0.32.1 idl close FinhtLJ4PVBwVi8tGwWoCzN3vDpcMofBZFXFzmntGxEh --provider.cluster devnet
--provider.wallet ~/.config/solana/devnet-escrow.json`; rent recovered), and only then was the
program upgraded and `anchor idl init` (1.x, PMP-backed) run. Had the order been reversed the legacy
account would have been unclosable forever. Mainnet has never published an IDL, so it needs no close.

## Hard rules

Same as workspace (`../CLAUDE.md`): no code comments, no AI attribution on commits, conventional commit messages, push only when asked.

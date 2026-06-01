# Worqen Escrow — Mainnet Launch Runbook (do this next)

A click‑by‑click checklist with links. For the *why* behind each role, see
[`MAINNET.md`](./MAINNET.md). Run phases in order. `<…>` = a value you fill in.

## Your fixed values (already set)
| Thing | Value |
|---|---|
| Program ID (mainnet) | `HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs` |
| Program keypair | `~/.config/solana/worqen-escrow-mainnet-program.json` (⚠️ back up offline) |
| Squads **vault** (upgrade authority) | `CPVcsjPtSzYJ1NuVkBHkqb4FNvzr6JhqMPB4jGP45JFD` |
| Squads app | https://app.squads.so/squads/CPVcsjPtSzYJ1NuVkBHkqb4FNvzr6JhqMPB4jGP45JFD/home |
| Treasury (`fee_recipient`) | `5jVqeEsYJaMtHZ4p3qKAhLgupo5Vre3anZWurKnQgDa8` (verified wallet you control) |
| Mainnet RPC | Helius (verified). Per shell: `export RPC_URL="https://mainnet.helius-rpc.com/?api-key=YOUR_KEY"` — **never commit the key** |

✅ Done already: Squads multisig created · mainnet program ID + keypair generated ·
escrow code feature‑flagged & CI‑green · frontend/backend mainnet guards in place.

---

## Phase 1 — Provision (do these now)

### 1.1 Mainnet RPC endpoint — ✅ DONE (Helius, verified working)
The api-key is a **secret** — keep it out of git. Set it once per terminal session (this
is what `$RPC_URL` means in every command below):
```bash
export RPC_URL="https://mainnet.helius-rpc.com/?api-key=YOUR_HELIUS_KEY"
```
Key handling: on the **backend**, store `SOLANA_RPC_URL` as a `gh secret` (it carries the
key). On the **frontend** (Phase 5.2) the URL is bundled into the browser, so the key is
public — restrict it to your domain in the Helius dashboard (Access Control) or use a
separate frontend-only key.

### 1.2 Treasury (`fee_recipient`) — ✅ CHOSEN
`5jVqeEsYJaMtHZ4p3qKAhLgupo5Vre3anZWurKnQgDa8` — verified on mainnet as a normal wallet
you control. All commission lands here; keep its private key safe (consider moving it
behind a multisig later). Baked into the commands below.

### 1.3 Deployer keypair — ✅ GENERATED → **fund with ~8 SOL**
- Address: `7GTsbFX9gywsrNQouUuvw5bChAeL9GCp24acMPKEC5Sn`
- File: `~/.config/solana/worqen-deployer-mainnet.json` (⚠️ back up offline)
- Send ~**8 SOL** (exchange/wallet → Solana network → paste the address). Pays the one-time
  deploy rent (~4.9 SOL, recoverable) + fees; also becomes the Config/pause key.
- Verify: `solana balance 7GTsbFX9gywsrNQouUuvw5bChAeL9GCp24acMPKEC5Sn --url $RPC_URL`

### 1.4 Platform-authority hot key — ✅ GENERATED → **fund with ~2 SOL**
- Address (= `ESCROW_WALLET_ADDRESS`): `FonMQCHztDpr8UaZH6f2LvZr79a9XzJgKoLp9Dpz79pK`
- File: `~/.config/solana/worqen-platform-authority-mainnet.json` (⚠️ back up offline)
- Send ~**2 SOL**. The backend signs releases/resolves + sponsors embedded-wallet gas.
- At the backend flip (Phase 5.1), get its base58 secret for `ESCROW_WALLET_PRIVATE_KEY`:
```bash
cd ~/Worqen/backend && uv run python -c "import json,base58; print(base58.b58encode(bytes(json.load(open('$HOME/.config/solana/worqen-platform-authority-mainnet.json')))).decode())"
```

### 1.5 Top up the Squads vault a little (~0.05 SOL)
So it can pay the fee when it later executes upgrades. Send SOL to `CPVcs…`.

---

## Phase 2 — Deploy the program (genesis)

Docs: Solana deploy guide https://docs.anza.xyz/cli/examples/deploy-a-program · verifiable builds https://github.com/Ellipsis-Labs/solana-verifiable-build

```bash
cd ~/Worqen/Worqen-Escrow

# 2.1 Build the reproducible mainnet artifact + matching IDL (needs Docker for solana-verify)
solana-verify build --library-name worqen_escrow -- --features mainnet
anchor build -- --features mainnet          # produces target/idl with the mainnet id (for the ops scripts)

# 2.2 Point the CLI at mainnet, paying with the deployer
solana config set --url $RPC_URL --keypair ~/.config/solana/worqen-deployer-mainnet.json
solana balance                              # confirm ~6–10 SOL

# 2.3 Genesis deploy (this is the irreversible, money step)
solana program deploy target/deploy/worqen_escrow.so \
  --program-id ~/.config/solana/worqen-escrow-mainnet-program.json \
  --with-compute-unit-price 100000 --max-sign-attempts 100

# 2.4 Confirm it landed (Authority should be the deployer for now)
solana program show HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs

# 2.5 Hand the upgrade authority to your Squads vault — the crown jewel.
#     --skip flag is REQUIRED because the new authority is a PDA that can't sign a CLI tx.
solana program set-upgrade-authority HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs \
  --new-upgrade-authority CPVcsjPtSzYJ1NuVkBHkqb4FNvzr6JhqMPB4jGP45JFD \
  --skip-new-upgrade-authority-signer-check

# 2.6 Verify: "Authority" now reads CPVcs… (no single key can upgrade anymore)
solana program show HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs
```
View it: https://solscan.io/account/HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs

---

## Phase 3 — Configure the program on‑chain

**Verify each mint on Solscan before allowlisting** (paste must match exactly):
- USDC `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` → https://solscan.io/token/EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
- USDT `Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB` → https://solscan.io/token/Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB
- EURC `HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr` → https://solscan.io/token/HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr

```bash
# 3.1 Create Config: treasury + commission default + stablecoin allowlist (idempotent).
#     The deployer becomes the Config authority — KEEP it; it's your fast pause key.
make bootstrap-config RPC_URL=$RPC_URL AUTHORITY_KEYPAIR=~/.config/solana/worqen-deployer-mainnet.json \
  FEE_RECIPIENT=5jVqeEsYJaMtHZ4p3qKAhLgupo5Vre3anZWurKnQgDa8 \
  ALLOWED_MINTS=EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v,Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB,HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr

# 3.2 Confirm: paused=false, authority=deployer, fee_recipient=treasury, 3 mints listed
make config-status RPC_URL=$RPC_URL
```

---

## Phase 4 — GitHub secrets (for future upgrades via Squads)

Docs: environments https://docs.github.com/en/actions/deployment/targeting-different-environments/using-environments-for-deployment · secrets https://docs.github.com/en/actions/security-guides/using-secrets-in-github-actions

1. Repo **Settings → Environments → New environment** → name it **`mainnet-beta`** → add **Required reviewers** (you / the team) so no upgrade buffer is written without approval.
2. Add its secrets:
```bash
gh secret set MAINNET_PROGRAM_ID       -e mainnet-beta -b "HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs"        -R Worqen-Labs/Worqen-Escrow
gh secret set MAINNET_UPGRADE_MULTISIG -e mainnet-beta -b "CPVcsjPtSzYJ1NuVkBHkqb4FNvzr6JhqMPB4jGP45JFD"        -R Worqen-Labs/Worqen-Escrow
gh secret set MAINNET_DEPLOYER_KEYPAIR -e mainnet-beta -R Worqen-Labs/Worqen-Escrow < ~/.config/solana/worqen-deployer-mainnet.json
```
(I can run the first two for you — they're public values. The third is your keypair, so you run it.)

**Future upgrades:** push a git tag `vX.Y.Z` → `release.yml` builds a verified buffer + hands it to the vault → approve the upgrade in the Squads app (**Developers → Programs**, docs https://docs.squads.so/ ). No manual redeploy ever again.

---

## Phase 5 — Flip the apps to mainnet (only AFTER Phases 2–3)

> ⚠️ Before flipping prod: make sure existing **devnet** escrow rows in the prod DB are terminal/cleaned, or the reconciler will alert on them as "missing on mainnet." Ping me — I'll add a cutover filter if needed.

### 5.1 Backend (prod) — note the `.env`‑rewrite gotcha: every var needs a `gh variable`/`secret` **and** a line in `build.yml` (prod stack)
Non‑secret → `gh variable set`; private keys → `gh secret set`.
```
SOLANA_NETWORK=mainnet
SOLANA_RPC_URL=$RPC_URL
ESCROW_PROGRAM_ID=HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs
ESCROW_FEE_RECIPIENT=5jVqeEsYJaMtHZ4p3qKAhLgupo5Vre3anZWurKnQgDa8
ESCROW_WALLET_ADDRESS=<platform-authority pubkey>          # from 1.4
ESCROW_WALLET_PRIVATE_KEY=<base58 secret>                  # from 1.4  (secret!)
WALLET_ENCRYPTION_KEY=<existing prod key>                  # keep current (secret!)
```
The backend **refuses to boot** if any of these are missing or still devnet — that's the safety net. *(I'll prepare the exact `gh` commands + `build.yml` diff once you confirm the values.)*

### 5.2 Frontend (prod, Vercel) — docs https://vercel.com/docs/projects/environment-variables (set on **Production**)
```
NEXT_PUBLIC_SOLANA_CLUSTER=mainnet-beta
NEXT_PUBLIC_SOLANA_RPC_URL=$RPC_URL
NEXT_PUBLIC_ESCROW_PROGRAM_ID=HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs
```
Then redeploy prod. (The app throws on mainnet if the program ID is missing — intentional.)

---

## Phase 6 — Go live

```bash
# 6.1 Rehearse the kill-switch once (then leave unpaused)
make pause   RPC_URL=$RPC_URL AUTHORITY_KEYPAIR=~/.config/solana/worqen-deployer-mainnet.json
make config-status RPC_URL=$RPC_URL          # paused=true
make unpause RPC_URL=$RPC_URL AUTHORITY_KEYPAIR=~/.config/solana/worqen-deployer-mainnet.json
```
6.2 **Real‑money smoke test** with a few $ of USDC (team accounts): deposit → release, then dispute → resolve, then cancel/refund. Confirm balances + the off‑chain mirror match.
6.3 **Soft launch**: open to a small group / steer to small jobs first, watch the monitoring alerts, then ramp.

---

## ✅ Deployed to mainnet (2026-06-02)
On-chain is **live + configured + secured**. The apps still point at devnet — go-live (Phases 4–6) is what's left.
- Program: `HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs` — deploy tx `2DBEFeXvqRhxhm1VvHBRDbCreek44vjdxfQDSpLHYiwSuFPoGydB84Zz6pCzTaqubGgPLYdXrzHPQhMjr8XZdXYn`
- Upgrade authority: `CPVcsjPtSzYJ1NuVkBHkqb4FNvzr6JhqMPB4jGP45JFD` (Squads multisig) — verified
- Config PDA: `F6cqSgcBv2Jo9ttHoPRVuQDvYv6yMVhFdkrJUNe8dsyP` — treasury `5jVqe…`, 5% default, USDC/USDT/EURC, `paused=false`, Config authority `7GTs…` (deployer = fast pause key). init tx `63NqXyZQvbqxx6EFcLkiox2knDXFSGZejk3m8KnQM4PWKJHJPMbbyc3WbtHPm999hYVupRJEWT9vN3URzt9MMPFZ`
- **Next:** Phase 4 (GitHub secrets) · Phase 5 (flip backend + frontend env to mainnet) · Phase 6 (pause rehearsal, real-$ smoke, soft launch)

## Quick links
- Squads (yours): https://app.squads.so/squads/CPVcsjPtSzYJ1NuVkBHkqb4FNvzr6JhqMPB4jGP45JFD/home · docs https://docs.squads.so/
- Helius: https://dashboard.helius.dev/
- Solana deploy: https://docs.anza.xyz/cli/examples/deploy-a-program · verify: https://github.com/Ellipsis-Labs/solana-verifiable-build
- Explorer: https://explorer.solana.com/address/HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs · Solscan: https://solscan.io/account/HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs
- GitHub envs: https://docs.github.com/en/actions/deployment/targeting-different-environments/using-environments-for-deployment
- Vercel env: https://vercel.com/docs/projects/environment-variables
```

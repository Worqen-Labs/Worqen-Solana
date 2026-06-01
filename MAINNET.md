# Worqen Escrow — Mainnet Deploy Guide (first‑timer friendly)

This is a plain‑language companion to the deploy commands. Read it once top‑to‑bottom
before you touch mainnet — the individual steps only make sense once the roles click.

Program ID (mainnet): **`HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs`**
Program keypair: `~/.config/solana/worqen-escrow-mainnet-program.json` — **back it up offline.**

---

## 1. The mental model

When you "deploy to mainnet" you're putting **upgradeable program code** on‑chain that will
**hold other people's money** (escrowed SOL/USDC). Two facts drive everything else:

1. **The code is upgradeable.** Solana programs aren't frozen — whoever holds the program's
   *upgrade authority* can replace the code at any time. That's great for shipping fixes…
   and catastrophic if the wrong person gets that power: they could deploy malicious code
   that drains every escrow.
2. **A handful of keys control everything.** There is no "admin password." Control is just
   keypairs. So mainnet security is really **key management** — who holds which key, and what
   happens if a key leaks.

So the whole game is: **give each power to the right kind of key, and make the most dangerous
power (upgrading the code) impossible for any single key to use alone.** That last part is what
Squads is for.

---

## 2. The five roles (learn these — everything references them)

| Role | What it can do | If it leaks… | Who should hold it |
|---|---|---|---|
| **Program keypair** | Sets the program's *address* at first deploy | Pre‑deploy: someone could grab the address. Post‑deploy: ~harmless (not used for upgrades) | You. Use once, then back it up offline. |
| **Upgrade authority** | **Replace the program's code** | 🔴 Total loss — attacker ships code that drains all escrows | **A multisig (Squads).** This is the crown jewel. |
| **Config authority** | Pause, set treasury + fee %, manage the mint allowlist, hand off authority | 🟠 Can pause (annoying) + redirect *future* commission. **Cannot drain escrows or upgrade.** | A fast key you can pause with in seconds (or the multisig). |
| **`fee_recipient` (treasury)** | *Receives* commission. **Never signs anything.** | 🟢 Nothing — it only collects | A cold / receive‑only / multisig wallet. |
| **Platform authority** | Per‑escrow: sign releases/resolves, sponsor gas | 🟠 Can move funds *within the escrow rules*; can't upgrade or change config | A hot key in the backend (secrets manager), funded with SOL. |

Key insight: **these are different keys with very different blast radii.** Never collapse them
into one wallet. The upgrade authority is the one that can lose *everything*, so it gets the
strongest protection (a multisig). The treasury only receives, so it can be a simple cold
wallet.

---

## 3. What is a multisig? What is Squads?

**Multisig = "multi‑signature."** Instead of one private key being able to act, an action needs
**M‑of‑N** approvals. Example **2‑of‑3**: you set up three member wallets; any action needs at
least two of them to approve. No single leaked key can do anything on its own.

**Squads** ([app.squads.so](https://app.squads.so)) is the standard, audited multisig for
Solana — an on‑chain program plus a web app. You create a "Squads," add member wallets, set a
threshold, and Squads gives you a **multisig address**. Anything that address controls now
requires M‑of‑N member approvals, coordinated through the Squads UI.

**Why we need it for the upgrade authority:** if your upgrade authority were a single key on
your laptop and that laptop/key were ever compromised, the attacker replaces the escrow program
with one that sends every locked dollar to themselves. With a 2‑of‑3 Squads as the upgrade
authority, an attacker who steals one member key **still can't upgrade the program** — they'd
need to compromise two members at once. For a fund‑custody program with no external audit, this
multisig is the single most important protection you have. It's the thing that makes "one
leaked key" survivable.

Squads is also handy as your **treasury** wallet and (optionally) your **Config authority**, so
sensitive settings changes also need M‑of‑N.

### Creating your Squads (one‑time, ~minutes, costs a fraction of a SOL in rent)

1. Go to [app.squads.so](https://app.squads.so), connect a wallet (e.g. Phantom) on
   **mainnet**.
2. **Create Squads** → give it a name (e.g. "Worqen Escrow").
3. **Add members**: paste the wallet addresses of the people/devices that should be able to
   approve. For a small team, three members is typical.
4. **Set the threshold**: e.g. **2 of 3** (two approvals required). Pick a number you can
   actually reach quickly but that no single person controls.
5. Confirm + fund the Squads with a little SOL (for transaction rent/fees).
6. Copy the **Squads/vault address** — that's what you'll pass everywhere as
   `<SQUADS_MULTISIG>` (and set as `MAINNET_UPGRADE_MULTISIG`).

> Threshold tip: 2‑of‑3 is the common balance of security vs. availability. Higher N is safer
> but means more people must be online to upgrade or (if you point pause at it) to pause. That's
> why we keep the **pause** kill‑switch on a *fast* key — see §6.

---

## 4. How an upgrade actually works after launch (the buffer dance)

The escrow program is ~700 KB — too big to send in one transaction. So upgrades happen in two
moves, and our CI already automates the hard part:

1. You push a git **tag** → `release.yml` does a reproducible build and uploads the new code to
   a temporary **buffer** account on‑chain, then hands that buffer's authority to your Squads.
2. In the **Squads app → Programs**, you (and the other members) review and **approve** a single
   "upgrade from buffer" transaction. Once M members approve, the live program swaps to the new
   code. Done.

So day‑to‑day you never hold the upgrade authority yourself — you propose, the multisig approves.
Nothing reaches mainnet without M‑of‑N sign‑off.

---

## 5. The deploy sequence (how the roles come together)

The **first** deploy ("genesis") is special: there's no program yet, so you can't "upgrade from
a buffer." You deploy it directly once, then immediately hand the upgrade authority to Squads.
After that, every future change uses the buffer dance in §4.

```
                ┌─ you (deployer key, funded ~6–10 SOL) ─┐
genesis deploy ─┤  solana program deploy  (one time)     │
                └─ then set-upgrade-authority → SQUADS ───┘   ← crown jewel handed off

bootstrap ─────  make bootstrap-config  → writes treasury + mint allowlist into Config
                 (deployer is Config authority for now…)
handoff ───────  …then hand Config authority to your fast-pause key (or Squads)

go-live ───────  flip backend + frontend env to mainnet (program id HShW…, treasury, RPC)
                 → backend boot-guard refuses to start if anything is half-set (safety net)

later ─────────  upgrades: git tag → release.yml buffer → Squads approves (§4)
```

The exact copy‑paste commands live in the chat runbook / `SECURITY.md`. The point of *this* doc
is that you understand **why** each step exists:

- **Deploy then hand off**: you must sign the genesis deploy yourself (the multisig can't create
  a brand‑new program), so you deploy with a normal funded key, then transfer the upgrade
  authority to Squads so no single key keeps that power.
- **bootstrap-config**: turns on the program's settings — which treasury gets commission, which
  stablecoins are allowed (USDC/USDT/EURC). It's idempotent (safe to re‑run).
- **env flip**: the apps only point at mainnet once you set the env vars; the backend literally
  refuses to boot on mainnet with leftover devnet config, so you can't half‑switch by accident.

---

## 6. The pause kill‑switch (your emergency brake)

The program has a `paused` flag (in Config). When on, it blocks **new** money entering
(create/deposit/direct‑pay) but **never** blocks releases, disputes, or closes — so pausing can
*never* trap funds; people can always get their money out. If anything looks wrong on mainnet,
hit pause first, investigate second.

```bash
make config-status RPC_URL=<MAINNET_RPC>                                   # read state
make pause   RPC_URL=<MAINNET_RPC> AUTHORITY_KEYPAIR=<config-authority>    # stop new escrows
make unpause RPC_URL=<MAINNET_RPC> AUTHORITY_KEYPAIR=<config-authority>    # resume
```

Because pause is your emergency brake, it's best to keep the **Config authority on a fast key**
(one person can pause instantly) rather than behind the slow M‑of‑N multisig — a leaked Config
key can only grief/redirect‑future‑fees, never drain. Keep the *upgrade* authority on the strict
multisig; keep *pause* fast.

---

## 7. First‑timer glossary

- **RPC endpoint** — the server your tools/app talk to the blockchain through. The free public
  mainnet RPC is rate‑limited and unsuitable for real money; use a dedicated one (Helius free
  tier is fine to start). Our code *requires* a real one on mainnet.
- **Rent** — SOL you "park" to keep an account alive on‑chain. The program deploy parks ~4.9 SOL
  of rent (recoverable if you ever close the program). Per‑escrow rent is small and reclaimed by
  the daily close task.
- **Buffer** — a temporary on‑chain account holding new program code before an upgrade swaps to
  it (see §4).
- **Genesis deploy** — the very first deploy of the program (vs. later upgrades).
- **`declare_id` / program ID** — the program's on‑chain address. Mainnet has its own
  (`HShW…`); devnet stays `6Ftag…`. Our build picks the right one via a `--features mainnet`
  flag, so you never edit it by hand.

---

## 8. Pre‑flight checklist

- [ ] Squads multisig created (threshold chosen, members added, funded) → have its address
- [ ] Cold treasury wallet address ready
- [ ] Paid mainnet RPC URL (Helius/QuickNode/Triton)
- [ ] Deployer keypair funded with ~6–10 SOL
- [ ] Platform‑authority hot keypair funded with a few SOL
- [ ] Program keypair (`worqen-escrow-mainnet-program.json`) **backed up offline**
- [ ] Decided who holds the Config authority (fast key recommended for pause)
- [ ] Plan for existing devnet escrow rows in prod (so the reconciler doesn't alert on them)
- [ ] Rehearsed `make pause` / `make unpause` on devnet so it's muscle memory

When these are checked, run the genesis‑deploy runbook (chat / `SECURITY.md`), then do a small
real‑money smoke test (deposit → release, dispute → resolve, cancel) before opening to users.
Soft‑launch with low exposure, then ramp.
```

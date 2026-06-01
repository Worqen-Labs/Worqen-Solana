#!/usr/bin/env bun
/**
 * Worqen Escrow — one-time Config bootstrap (init, then reconcile). Idempotent.
 *
 * First run on a fresh cluster: creates the singleton Config PDA via
 * init_config (the signer becomes the initial authority and pays rent).
 * Subsequent runs: report current state and add_allowed_mint for any
 * ALLOWED_MINTS not yet present. It NEVER silently changes fee_recipient,
 * commission, or authority on an existing Config — those it only reports
 * (mutate them deliberately with update_config / a handoff).
 *
 * Run from the repo root after `anchor build`:
 *
 *   RPC_URL=https://<mainnet-rpc> \
 *   AUTHORITY_KEYPAIR=~/path/initial-authority.json \
 *   FEE_RECIPIENT=<treasury pubkey> \
 *   DEFAULT_BPS=500 \
 *   ALLOWED_MINTS=<usdc>,<usdt>,<eurc> \
 *     bun scripts/bootstrap-config.ts
 *
 * Mainnet stablecoin mints (verify before use):
 *   USDC  EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
 *   USDT  Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB
 *   EURC  HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr
 *
 * After bootstrap, hand Config.authority to your multisig (two-step):
 *   update_config(new_pending_authority=<squads>)  then  accept_authority(<squads>).
 */
import * as anchor from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";

const IDL_PATH = process.env.IDL_PATH ?? "target/idl/worqen_escrow.json";

function die(msg: string): never {
  console.error(`✗ ${msg}`);
  process.exit(1);
}

function expandHome(p: string): string {
  return p.startsWith("~") ? p.replace(/^~/, homedir()) : p;
}

function loadKeypair(path: string): Keypair {
  const raw = JSON.parse(readFileSync(expandHome(path), "utf8"));
  return Keypair.fromSecretKey(Uint8Array.from(raw));
}

function pubkey(label: string, value: string): PublicKey {
  try {
    return new PublicKey(value.trim());
  } catch {
    return die(`${label} is not a valid pubkey: "${value}"`);
  }
}

function parseMints(): PublicKey[] {
  const raw = (process.env.ALLOWED_MINTS ?? "").trim();
  if (!raw) return [];
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => pubkey("ALLOWED_MINTS entry", s));
}

async function main(): Promise<void> {
  const rpcUrl = process.env.RPC_URL;
  if (!rpcUrl) die("RPC_URL is required (no silent default)");
  const keypairPath = process.env.AUTHORITY_KEYPAIR;
  if (!keypairPath)
    die(
      "AUTHORITY_KEYPAIR is required (the initial Config.authority + rent payer)",
    );

  const defaultBps = Number(process.env.DEFAULT_BPS ?? "500");
  if (!Number.isInteger(defaultBps) || defaultBps < 0 || defaultBps > 1000) {
    die(
      `DEFAULT_BPS must be an integer in [0, 1000] (got "${process.env.DEFAULT_BPS}")`,
    );
  }
  const wantMints = parseMints();

  const idl = JSON.parse(readFileSync(IDL_PATH, "utf8")) as anchor.Idl;
  const signer = loadKeypair(keypairPath);
  const connection = new Connection(rpcUrl, "confirmed");
  const provider = new anchor.AnchorProvider(
    connection,
    new anchor.Wallet(signer),
    {
      commitment: "confirmed",
    },
  );
  const program = new anchor.Program(idl, provider);
  const programId = program.programId;
  const [configPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    programId,
  );

  console.log(`\nWorqen Escrow — bootstrap Config  (RPC ${rpcUrl})`);
  console.log(`  program     ${programId.toBase58()}`);
  console.log(`  config PDA  ${configPda.toBase58()}`);
  console.log(`  signer      ${signer.publicKey.toBase58()}\n`);

  // null when the account does not exist yet; otherwise the decoded Config.
  const existing: any = await program.account.config
    .fetch(configPda)
    .catch(() => null);

  if (!existing) {
    const feeStr = process.env.FEE_RECIPIENT;
    if (!feeStr)
      die(
        "FEE_RECIPIENT is required to initialize a fresh Config (treasury pubkey)",
      );
    const feeRecipient = pubkey("FEE_RECIPIENT", feeStr);
    if (feeRecipient.equals(PublicKey.default))
      die("FEE_RECIPIENT must not be the zero pubkey");

    console.log("No Config found — initializing:");
    console.log(`  fee_recipient  ${feeRecipient.toBase58()}`);
    console.log(`  default_bps    ${defaultBps}`);
    console.log(
      `  allowed_mints  ${wantMints.length ? wantMints.map((m) => m.toBase58()).join(", ") : "(none — SOL only)"}\n`,
    );

    const sig = await program.methods
      .initConfig(feeRecipient, defaultBps, wantMints)
      .accountsPartial({ config: configPda, authority: signer.publicKey })
      .rpc();
    console.log(`✓ init_config — tx ${sig}\n`);
    return;
  }

  // Config exists — report, then add any missing mints.
  const have: PublicKey[] = existing.allowedMints;
  console.log("Config already exists:");
  console.log(
    `  authority      ${existing.authority.toBase58()}${existing.authority.equals(signer.publicKey) ? " (= signer)" : " (≠ signer — handoff already done?)"}`,
  );
  console.log(`  fee_recipient  ${existing.feeRecipient.toBase58()}`);
  console.log(`  default_bps    ${existing.defaultCommissionBps}`);
  console.log(`  paused         ${existing.paused}`);
  console.log(
    `  allowed_mints  ${have.length ? have.map((m) => m.toBase58()).join(", ") : "(none)"}\n`,
  );

  const missing = wantMints.filter((w) => !have.some((h) => h.equals(w)));
  if (missing.length === 0) {
    console.log(
      "Nothing to reconcile — allowlist already contains every requested mint.\n",
    );
    return;
  }
  if (!existing.authority.equals(signer.publicKey)) {
    die(
      `cannot add mints: signer is not Config.authority (${existing.authority.toBase58()}). ` +
        "Run with the authority key (or via the multisig).",
    );
  }
  for (const mint of missing) {
    const sig = await program.methods
      .addAllowedMint(mint)
      .accountsPartial({ config: configPda, authority: signer.publicKey })
      .rpc();
    console.log(`✓ add_allowed_mint ${mint.toBase58()} — tx ${sig}`);
  }
  console.log();
}

main().catch((e) => die(e?.message ?? String(e)));

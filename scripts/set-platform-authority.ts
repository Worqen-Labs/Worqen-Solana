#!/usr/bin/env bun
import * as anchor from "@anchor-lang/core";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";

const RPC_URL = process.env.RPC_URL ?? "https://api.devnet.solana.com";
const AUTHORITY_KEYPAIR =
  process.env.AUTHORITY_KEYPAIR ?? "~/.config/solana/id.json";
const PLATFORM_AUTHORITY = process.env.PLATFORM_AUTHORITY;
const IDL_PATH = process.env.IDL_PATH ?? "target/idl/worqen_escrow.json";

function expandHome(p: string): string {
  return p.startsWith("~") ? p.replace(/^~/, homedir()) : p;
}
function loadKeypair(path: string): Keypair {
  return Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(expandHome(path), "utf8"))),
  );
}

if (!PLATFORM_AUTHORITY) {
  console.error("PLATFORM_AUTHORITY env required");
  process.exit(1);
}

const idl = JSON.parse(readFileSync(IDL_PATH, "utf8"));
const connection = new Connection(RPC_URL, "confirmed");
const authority = loadKeypair(AUTHORITY_KEYPAIR);
const wallet = new anchor.Wallet(authority);
const provider = new anchor.AnchorProvider(connection, wallet, {
  commitment: "confirmed",
});
const program = new anchor.Program(idl, provider);
const [configPda] = PublicKey.findProgramAddressSync(
  [Buffer.from("config")],
  program.programId,
);

const platform = new PublicKey(PLATFORM_AUTHORITY);
const tx = await program.methods
  .updateConfig(null, null, null, null, platform)
  .accounts({ config: configPda, authority: authority.publicKey })
  .rpc();

const cfg = await program.account.config.fetch(configPda);
console.log("✓ update_config — tx", tx);
console.log("  config PDA          ", configPda.toBase58());
console.log("  platform_authority  ", cfg.platformAuthority.toBase58());
console.log("  fee_recipient       ", cfg.feeRecipient.toBase58());
console.log("  paused              ", cfg.paused);

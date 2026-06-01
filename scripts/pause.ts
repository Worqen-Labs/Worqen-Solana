#!/usr/bin/env bun
/**
 * Worqen Escrow — emergency pause / unpause / status (the operational kill-switch).
 *
 * The on-chain `paused` flag on the Config PDA blocks only NEW money entering
 * the system: create_escrow / deposit_* / pay_with_commission_*. It can NEVER
 * block release, dispute, auto-release or close — so pausing can never strand
 * user funds. Flip it the instant something looks wrong on mainnet.
 *
 * Run from the repo root after `anchor build` (so target/idl/worqen_escrow.json
 * exists; override with IDL_PATH):
 *
 *   RPC_URL=https://<your-rpc> bun scripts/pause.ts status
 *   RPC_URL=https://<your-rpc> AUTHORITY_KEYPAIR=~/path/config-authority.json \
 *     bun scripts/pause.ts pause
 *   RPC_URL=https://<your-rpc> AUTHORITY_KEYPAIR=~/path/config-authority.json \
 *     bun scripts/pause.ts unpause
 *
 * The signer MUST be the current Config.authority. On mainnet that is the fast
 * key (or fast multisig) you reserve for emergencies — see SECURITY.md
 * "Operational kill-switch". `status` needs no keypair.
 */
import * as anchor from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";

const IDL_PATH = process.env.IDL_PATH ?? "target/idl/worqen_escrow.json";
const COMMANDS = ["status", "pause", "unpause"] as const;
type Command = (typeof COMMANDS)[number];

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

// cfg is the decoded Config account (anchor returns an untyped object here).
function printConfig(
  programId: PublicKey,
  configPda: PublicKey,
  cfg: any,
): void {
  const mints: PublicKey[] = cfg.allowedMints;
  console.log(`  program        ${programId.toBase58()}`);
  console.log(`  config PDA     ${configPda.toBase58()}`);
  console.log(
    `  paused         ${cfg.paused ? "\u{1F534} TRUE — new escrows blocked" : "\u{1F7E2} false — open"}`,
  );
  console.log(`  authority      ${cfg.authority.toBase58()}`);
  if (cfg.pendingAuthority && !cfg.pendingAuthority.equals(PublicKey.default)) {
    console.log(
      `  pendingAuth    ${cfg.pendingAuthority.toBase58()} (handoff in progress)`,
    );
  }
  console.log(`  fee_recipient  ${cfg.feeRecipient.toBase58()}`);
  console.log(`  default_bps    ${cfg.defaultCommissionBps}`);
  console.log(
    `  allowed_mints  ${mints.length ? mints.map((m) => m.toBase58()).join(", ") : "(none — SOL only)"}`,
  );
}

async function main(): Promise<void> {
  const cmd = (process.argv[2] ?? "status").toLowerCase() as Command;
  if (!COMMANDS.includes(cmd)) {
    die(`unknown command "${cmd}" — use one of: ${COMMANDS.join(" | ")}`);
  }

  const rpcUrl = process.env.RPC_URL;
  if (!rpcUrl) {
    die(
      "RPC_URL is required (no silent default — point it at your mainnet/devnet RPC)",
    );
  }

  const keypairPath = process.env.AUTHORITY_KEYPAIR;
  if (!keypairPath && cmd !== "status") {
    die(
      "AUTHORITY_KEYPAIR is required for pause/unpause (the Config.authority keypair)",
    );
  }

  const idl = JSON.parse(readFileSync(IDL_PATH, "utf8")) as anchor.Idl;
  // `status` needs no real signer; a throwaway wallet is enough to read state.
  const signer = keypairPath ? loadKeypair(keypairPath) : Keypair.generate();
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

  console.log(`\nWorqen Escrow — ${cmd.toUpperCase()}  (RPC ${rpcUrl})\n`);

  const before = await program.account.config.fetch(configPda).catch(() => {
    die(
      `Config PDA ${configPda.toBase58()} not found — is the program initialized on this cluster?`,
    );
  });

  if (cmd === "status") {
    printConfig(programId, configPda, before);
    return;
  }

  const target = cmd === "pause";
  if (before.paused === target) {
    console.log(`Already ${target ? "paused" : "unpaused"} — nothing to do.\n`);
    printConfig(programId, configPda, before);
    return;
  }
  if (!before.authority.equals(signer.publicKey)) {
    die(
      `signer ${signer.publicKey.toBase58()} is NOT the Config.authority ` +
        `(${before.authority.toBase58()}). Use the authority keypair.`,
    );
  }

  const sig = await program.methods
    .updateConfig(null, null, target, null)
    .accountsPartial({ config: configPda, authority: signer.publicKey })
    .rpc();

  console.log(`✓ ${target ? "PAUSED" : "UNPAUSED"} — tx ${sig}\n`);
  printConfig(
    programId,
    configPda,
    await program.account.config.fetch(configPda),
  );
}

main().catch((e) => die(e?.message ?? String(e)));

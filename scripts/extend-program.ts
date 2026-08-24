#!/usr/bin/env bun
/**
 * Worqen Escrow — extend the program's ProgramData account so a larger binary
 * can be deployed. `solana program extend` cannot be used here: the CLI insists
 * on the upgrade authority signing locally, and ours is a Squads vault PDA with
 * no keypair. On-chain, however, the legacy ExtendProgram instruction is
 * permissionless (no authority account) while `enable_extend_program_checked`
 * (2oMRZEDWT2tqtYMofhmmfQ8SsjqUFzT6sYXppQDavxwz) is inactive — this script
 * aborts if that feature account has appeared.
 *
 *   RPC_URL=https://<mainnet-rpc> bun scripts/extend-program.ts            # simulate only
 *   RPC_URL=https://<mainnet-rpc> SEND=yes bun scripts/extend-program.ts   # broadcast
 *
 * PAYER_KEYPAIR defaults to ~/.config/solana/worqen-deployer-mainnet.json.
 * SO_PATH defaults to target/deploy-v3/worqen_escrow.so; additional_bytes is
 * computed as (45 + so_size) - current ProgramData space and never guessed.
 */
import {
  ComputeBudgetProgram,
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import { readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";

const PROGRAM_ID = new PublicKey(
  "HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs",
);
const LOADER_ID = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");
const SYSTEM_ID = new PublicKey("11111111111111111111111111111111");
const EXTEND_CHECKED_FEATURE = new PublicKey(
  "2oMRZEDWT2tqtYMofhmmfQ8SsjqUFzT6sYXppQDavxwz",
);
const PROGRAMDATA_METADATA_BYTES = 45;
const EXTEND_PROGRAM_DISCRIMINANT = 6;

function die(msg: string): never {
  console.error(`✗ ${msg}`);
  process.exit(1);
}

function expandHome(p: string): string {
  return p.startsWith("~") ? p.replace(/^~/, homedir()) : p;
}

async function main(): Promise<void> {
  const rpcUrl = process.env.RPC_URL ?? die("RPC_URL is required");
  const soPath = process.env.SO_PATH ?? "target/deploy-v3/worqen_escrow.so";
  const payerPath = expandHome(
    process.env.PAYER_KEYPAIR ??
      "~/.config/solana/worqen-deployer-mainnet.json",
  );
  const send = process.env.SEND === "yes";

  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(payerPath, "utf8"))),
  );
  const connection = new Connection(rpcUrl, "confirmed");

  const feature = await connection.getAccountInfo(EXTEND_CHECKED_FEATURE);
  if (feature !== null) {
    die(
      "enable_extend_program_checked feature account exists on this cluster — " +
        "the legacy permissionless ExtendProgram path is (or is about to be) " +
        "closed. Extend must now be signed by the upgrade authority via a " +
        "Squads proposal instead.",
    );
  }

  const [programData] = PublicKey.findProgramAddressSync(
    [PROGRAM_ID.toBuffer()],
    LOADER_ID,
  );
  const account = await connection.getAccountInfo(programData);
  if (!account) die(`ProgramData ${programData.toBase58()} not found`);
  if (!account.owner.equals(LOADER_ID)) {
    die(`ProgramData owner is ${account.owner.toBase58()}, not the loader`);
  }

  const soSize = statSync(soPath).size;
  const currentSpace = account.data.length;
  const targetSpace = PROGRAMDATA_METADATA_BYTES + soSize;
  const additionalBytes = targetSpace - currentSpace;
  const rentTarget =
    await connection.getMinimumBalanceForRentExemption(targetSpace);
  const rentDelta = Math.max(0, rentTarget - account.lamports);
  const payerBalance = await connection.getBalance(payer.publicKey);

  console.log(`\nWorqen Escrow — EXTEND ProgramData  (RPC ${rpcUrl})\n`);
  console.log(`  program          ${PROGRAM_ID.toBase58()}`);
  console.log(`  programdata      ${programData.toBase58()}`);
  console.log(`  artifact         ${soPath} (${soSize} bytes)`);
  console.log(`  current space    ${currentSpace}`);
  console.log(`  target space     ${targetSpace}`);
  console.log(`  additional_bytes ${additionalBytes}`);
  console.log(
    `  rent delta       ${rentDelta} lamports (${rentDelta / 1e9} SOL)`,
  );
  console.log(
    `  payer            ${payer.publicKey.toBase58()} (${payerBalance / 1e9} SOL)\n`,
  );

  if (additionalBytes <= 0) {
    console.log("ProgramData is already large enough — nothing to do.");
    return;
  }
  if (payerBalance < rentDelta + 10_000) {
    die("payer cannot cover the rent delta plus fees");
  }

  const data = Buffer.alloc(8);
  data.writeUInt32LE(EXTEND_PROGRAM_DISCRIMINANT, 0);
  data.writeUInt32LE(additionalBytes, 4);

  const tx = new Transaction().add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: 20_000 }),
    ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 100_000 }),
    new TransactionInstruction({
      programId: LOADER_ID,
      keys: [
        { pubkey: programData, isSigner: false, isWritable: true },
        { pubkey: PROGRAM_ID, isSigner: false, isWritable: true },
        { pubkey: SYSTEM_ID, isSigner: false, isWritable: false },
        { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      ],
      data,
    }),
  );
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = (
    await connection.getLatestBlockhash("confirmed")
  ).blockhash;
  tx.sign(payer);

  const sim = await connection.simulateTransaction(tx);
  if (sim.value.err) {
    console.error(sim.value.logs?.join("\n"));
    die(`simulation failed: ${JSON.stringify(sim.value.err)}`);
  }
  console.log("✓ simulation ok");
  sim.value.logs?.forEach((l) => console.log(`    ${l}`));

  if (!send) {
    console.log("\nDry run only — re-run with SEND=yes to broadcast.");
    return;
  }

  const sig = await sendAndConfirmTransaction(connection, tx, [payer], {
    commitment: "confirmed",
  });
  console.log(`\n✓ EXTENDED — tx ${sig}`);
  const after = await connection.getAccountInfo(programData);
  console.log(`  new space        ${after?.data.length}`);
  console.log(`  new lamports     ${after?.lamports}`);
}

main().catch((e) => die(e?.message ?? String(e)));

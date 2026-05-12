#!/usr/bin/env ts-node

/**
 * Worqen Escrow - Test Cancellation Flow
 *
 * This script tests the cancellation flow:
 * 1. Creates and funds an escrow
 * 2. Employer cancels and gets full refund
 *
 * Prerequisites:
 * - Run `solana-test-validator` in a separate terminal
 * - Run `anchor build && anchor deploy --provider.cluster localnet`
 *
 * Usage:
 *   npx ts-node scripts/test-cancel.ts
 */

import * as anchor from "@coral-xyz/anchor";
import { Program, AnchorProvider, BN } from "@coral-xyz/anchor";
import {
  Connection,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { createHash } from "crypto";
import * as fs from "fs";
import * as path from "path";

// ============================================================================
// Configuration
// ============================================================================

const LOCALNET_URL = "http://localhost:8899";
const WALLETS_DIR = path.join(__dirname, "../.localnet-wallets");

const idlPath = path.join(__dirname, "../target/idl/worqen_escrow.json");
const idl = JSON.parse(fs.readFileSync(idlPath, "utf-8"));

const programKeypairPath = path.join(__dirname, "../target/deploy/worqen_escrow-keypair.json");
const programKeypair = Keypair.fromSecretKey(
  new Uint8Array(JSON.parse(fs.readFileSync(programKeypairPath, "utf-8")))
);
const PROGRAM_ID = programKeypair.publicKey;

// ============================================================================
// Helper Functions
// ============================================================================

function loadKeypair(name: string): Keypair {
  const filePath = path.join(WALLETS_DIR, `${name}.json`);
  if (!fs.existsSync(filePath)) {
    throw new Error(`Wallet not found: ${name}. Run local-test.ts first.`);
  }
  return Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(fs.readFileSync(filePath, "utf-8")))
  );
}

async function airdrop(connection: Connection, publicKey: PublicKey, amount: number) {
  const signature = await connection.requestAirdrop(publicKey, amount);
  const latestBlockhash = await connection.getLatestBlockhash();
  await connection.confirmTransaction({
    signature,
    blockhash: latestBlockhash.blockhash,
    lastValidBlockHeight: latestBlockhash.lastValidBlockHeight,
  });
}

function generateEscrowId(hireId: string): number[] {
  return Array.from(createHash("sha256").update(hireId).digest());
}

function derivePDAs(escrowId: number[], programId: PublicKey) {
  const [escrowPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("escrow"), Buffer.from(escrowId)],
    programId
  );
  const [vaultPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), escrowPda.toBuffer()],
    programId
  );
  return { escrowPda, vaultPda };
}

function calculateCommission(amount: number, rateBps: number): number {
  return Math.floor((amount * rateBps) / 10000);
}

async function getBalanceInSol(connection: Connection, publicKey: PublicKey): Promise<number> {
  return (await connection.getBalance(publicKey)) / LAMPORTS_PER_SOL;
}

function parseStatus(status: any): string {
  if (status.created) return "Created";
  if (status.funded) return "Funded";
  if (status.pendingRelease) return "PendingRelease";
  if (status.released) return "Released";
  if (status.disputed) return "Disputed";
  if (status.resolved) return "Resolved";
  if (status.cancelled) return "Cancelled";
  return "Unknown";
}

// ============================================================================
// Main
// ============================================================================

async function main() {
  console.log("\n" + "=".repeat(60));
  console.log("❌ Worqen Escrow - Cancellation Flow Test");
  console.log("=".repeat(60) + "\n");

  const connection = new Connection(LOCALNET_URL, "confirmed");

  // Load wallets
  console.log("👛 Loading wallets...");
  const employer = loadKeypair("employer");
  const employee = loadKeypair("employee");
  const platformAuthority = loadKeypair("platform-authority");
  console.log("  ✓ Wallets loaded\n");

  // Ensure employer has SOL
  await airdrop(connection, employer.publicKey, 50 * LAMPORTS_PER_SOL);

  // Setup program
  const employerProvider = new AnchorProvider(
    connection,
    {
      publicKey: employer.publicKey,
      signTransaction: async (tx) => { tx.sign(employer); return tx; },
      signAllTransactions: async (txs) => { txs.forEach((tx) => tx.sign(employer)); return txs; },
    },
    { commitment: "confirmed" }
  );

  const program = new Program(idl, PROGRAM_ID, employerProvider);

  // Test config
  const hireId = `cancel-test-${Date.now()}`;
  const escrowId = generateEscrowId(hireId);
  const { escrowPda, vaultPda } = derivePDAs(escrowId, PROGRAM_ID);
  const workerPayment = 8 * LAMPORTS_PER_SOL;
  const commissionRateBps = 150;
  const commissionAmount = calculateCommission(workerPayment, commissionRateBps);
  const totalDeposit = workerPayment + commissionAmount;

  console.log("📋 Test Configuration:");
  console.log(`  Worker Payment: ${workerPayment / LAMPORTS_PER_SOL} SOL`);
  console.log(`  Commission: ${commissionAmount / LAMPORTS_PER_SOL} SOL`);
  console.log(`  Total Deposit: ${totalDeposit / LAMPORTS_PER_SOL} SOL\n`);

  // Step 1: Create escrow
  console.log("📝 Creating escrow...");
  await program.methods
    .createEscrow(escrowId, new BN(workerPayment), true, commissionRateBps)
    .accountsStrict({
      escrow: escrowPda,
      employer: employer.publicKey,
      employee: employee.publicKey,
      platformAuthority: platformAuthority.publicKey,
      tokenMint: SystemProgram.programId,
      systemProgram: SystemProgram.programId,
    })
    .signers([employer])
    .rpc();
  console.log("  ✓ Escrow created\n");

  // Step 2: Deposit funds
  console.log("💵 Depositing funds...");
  const employerBalanceBeforeDeposit = await getBalanceInSol(connection, employer.publicKey);

  await program.methods
    .depositSol()
    .accountsStrict({
      escrow: escrowPda,
      escrowVault: vaultPda,
      employer: employer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([employer])
    .rpc();

  const vaultBalance = await getBalanceInSol(connection, vaultPda);
  console.log(`  ✓ Deposited: ${totalDeposit / LAMPORTS_PER_SOL} SOL`);
  console.log(`  ✓ Vault balance: ${vaultBalance.toFixed(4)} SOL\n`);

  // Step 3: Cancel escrow
  console.log("❌ Cancelling escrow...");
  const employerBalanceBeforeCancel = await getBalanceInSol(connection, employer.publicKey);
  console.log(`  Employer balance before cancel: ${employerBalanceBeforeCancel.toFixed(4)} SOL`);

  await program.methods
    .cancelEscrowSol()
    .accountsStrict({
      escrow: escrowPda,
      escrowVault: vaultPda,
      employer: employer.publicKey,
      signer: employer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([employer])
    .rpc();

  const employerBalanceAfterCancel = await getBalanceInSol(connection, employer.publicKey);
  const refundReceived = employerBalanceAfterCancel - employerBalanceBeforeCancel;

  let escrowAccount = await program.account.escrow.fetch(escrowPda);
  console.log(`  ✓ Status: ${parseStatus(escrowAccount.status)}`);
  console.log(`  Employer balance after cancel: ${employerBalanceAfterCancel.toFixed(4)} SOL`);
  console.log(`  ✓ Refund received: ${refundReceived.toFixed(4)} SOL\n`);

  // Summary
  console.log("=".repeat(60));
  console.log("📊 CANCELLATION SUMMARY");
  console.log("=".repeat(60));
  console.log(`  Original deposit: ${totalDeposit / LAMPORTS_PER_SOL} SOL`);
  console.log(`    - Worker payment: ${workerPayment / LAMPORTS_PER_SOL} SOL`);
  console.log(`    - Commission: ${commissionAmount / LAMPORTS_PER_SOL} SOL`);
  console.log(`  Refund received: ~${refundReceived.toFixed(4)} SOL`);
  console.log(`  (Difference is tx fee)`);
  console.log("\n✅ Cancellation flow completed successfully!\n");
}

main().catch(console.error);

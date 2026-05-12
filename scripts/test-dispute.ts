#!/usr/bin/env ts-node

/**
 * Worqen Escrow - Test Dispute Flow
 *
 * This script tests the dispute resolution flow:
 * 1. Creates and funds an escrow
 * 2. Employee raises a dispute
 * 3. Platform resolves with 60/40 split
 *
 * Prerequisites:
 * - Run `solana-test-validator` in a separate terminal
 * - Run `anchor build && anchor deploy --provider.cluster localnet`
 *
 * Usage:
 *   npx ts-node scripts/test-dispute.ts
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
  console.log("⚖️  Worqen Escrow - Dispute Flow Test");
  console.log("=".repeat(60) + "\n");

  const connection = new Connection(LOCALNET_URL, "confirmed");

  // Load wallets
  console.log("👛 Loading wallets...");
  const employer = loadKeypair("employer");
  const employee = loadKeypair("employee");
  const platformAuthority = loadKeypair("platform-authority");
  console.log("  ✓ Wallets loaded\n");

  // Ensure wallets have SOL
  await airdrop(connection, employer.publicKey, 50 * LAMPORTS_PER_SOL);
  await airdrop(connection, employee.publicKey, 1 * LAMPORTS_PER_SOL);
  await airdrop(connection, platformAuthority.publicKey, 1 * LAMPORTS_PER_SOL);

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

  const employeeProvider = new AnchorProvider(
    connection,
    {
      publicKey: employee.publicKey,
      signTransaction: async (tx) => { tx.sign(employee); return tx; },
      signAllTransactions: async (txs) => { txs.forEach((tx) => tx.sign(employee)); return txs; },
    },
    { commitment: "confirmed" }
  );

  const platformProvider = new AnchorProvider(
    connection,
    {
      publicKey: platformAuthority.publicKey,
      signTransaction: async (tx) => { tx.sign(platformAuthority); return tx; },
      signAllTransactions: async (txs) => { txs.forEach((tx) => tx.sign(platformAuthority)); return txs; },
    },
    { commitment: "confirmed" }
  );

  const programEmployer = new Program(idl, PROGRAM_ID, employerProvider);
  const programEmployee = new Program(idl, PROGRAM_ID, employeeProvider);
  const programPlatform = new Program(idl, PROGRAM_ID, platformProvider);

  // Test config
  const hireId = `dispute-test-${Date.now()}`;
  const escrowId = generateEscrowId(hireId);
  const { escrowPda, vaultPda } = derivePDAs(escrowId, PROGRAM_ID);
  const workerPayment = 10 * LAMPORTS_PER_SOL;
  const commissionRateBps = 150;
  const commissionAmount = calculateCommission(workerPayment, commissionRateBps);

  console.log("📋 Test Configuration:");
  console.log(`  Worker Payment: ${workerPayment / LAMPORTS_PER_SOL} SOL`);
  console.log(`  Commission: ${commissionAmount / LAMPORTS_PER_SOL} SOL\n`);

  // Step 1: Create and fund escrow
  console.log("📝 Creating and funding escrow...");
  await programEmployer.methods
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

  await programEmployer.methods
    .depositSol()
    .accountsStrict({
      escrow: escrowPda,
      escrowVault: vaultPda,
      employer: employer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([employer])
    .rpc();

  console.log("  ✓ Escrow created and funded\n");

  // Step 2: Employee raises dispute
  console.log("⚠️  Employee raising dispute...");
  const disputeReason = "Employer changed requirements after I completed the work";

  await programEmployee.methods
    .raiseDispute(Buffer.from(disputeReason))
    .accountsStrict({
      escrow: escrowPda,
      signer: employee.publicKey,
    })
    .signers([employee])
    .rpc();

  let escrowAccount = await programEmployer.account.escrow.fetch(escrowPda);
  console.log(`  ✓ Status: ${parseStatus(escrowAccount.status)}`);
  console.log(`  ✓ Reason: "${disputeReason}"\n`);

  // Step 3: Platform resolves dispute
  console.log("⚖️  Platform resolving dispute (60% to employee, 40% to employer)...");

  const employeeShare = Math.floor(workerPayment * 0.6); // 60% to employee
  const employerShare = workerPayment - employeeShare;   // 40% to employer
  const totalToEmployer = employerShare + commissionAmount; // + commission refund

  const employerBefore = await getBalanceInSol(connection, employer.publicKey);
  const employeeBefore = await getBalanceInSol(connection, employee.publicKey);

  await programPlatform.methods
    .resolveDisputeSol(new BN(employeeShare))
    .accountsStrict({
      escrow: escrowPda,
      escrowVault: vaultPda,
      employer: employer.publicKey,
      employee: employee.publicKey,
      platformAuthority: platformAuthority.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([platformAuthority])
    .rpc();

  const employerAfter = await getBalanceInSol(connection, employer.publicKey);
  const employeeAfter = await getBalanceInSol(connection, employee.publicKey);

  escrowAccount = await programEmployer.account.escrow.fetch(escrowPda);
  console.log(`  ✓ Status: ${parseStatus(escrowAccount.status)}`);
  console.log(`  ✓ Employee received: ${(employeeAfter - employeeBefore).toFixed(4)} SOL (60%)`);
  console.log(`  ✓ Employer received: ${(employerAfter - employerBefore).toFixed(4)} SOL (40% + commission refund)\n`);

  // Summary
  console.log("=".repeat(60));
  console.log("📊 DISPUTE RESOLUTION SUMMARY");
  console.log("=".repeat(60));
  console.log(`  Original worker payment: ${workerPayment / LAMPORTS_PER_SOL} SOL`);
  console.log(`  Commission (refunded): ${commissionAmount / LAMPORTS_PER_SOL} SOL`);
  console.log(`  Employee got: ${employeeShare / LAMPORTS_PER_SOL} SOL (60%)`);
  console.log(`  Employer got: ${totalToEmployer / LAMPORTS_PER_SOL} SOL (40% + commission)`);
  console.log(`  Platform got: 0 SOL (no commission on disputes)`);
  console.log("\n✅ Dispute flow completed successfully!\n");
}

main().catch(console.error);

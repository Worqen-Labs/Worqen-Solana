#!/usr/bin/env ts-node

/**
 * Worqen Escrow - Local Testing Script
 *
 * This script demonstrates the full escrow flow locally:
 * 1. Creates local wallets (employer, employee, platform)
 * 2. Airdrops SOL to test accounts
 * 3. Creates an escrow
 * 4. Deposits funds
 * 5. Confirms completion
 * 6. Releases payment
 *
 * Prerequisites:
 * - Run `solana-test-validator` in a separate terminal
 * - Run `anchor build && anchor deploy --provider.cluster localnet`
 *
 * Usage:
 *   npx ts-node scripts/local-test.ts
 *   # or
 *   bun scripts/local-test.ts
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

// Load IDL
const idlPath = path.join(__dirname, "../target/idl/worqen_escrow.json");
const idl = JSON.parse(fs.readFileSync(idlPath, "utf-8"));

// Load deployed program ID from keypair
const programKeypairPath = path.join(
  __dirname,
  "../target/deploy/worqen_escrow-keypair.json"
);
const programKeypair = Keypair.fromSecretKey(
  new Uint8Array(JSON.parse(fs.readFileSync(programKeypairPath, "utf-8")))
);
const PROGRAM_ID = programKeypair.publicKey;

// ============================================================================
// Helper Functions
// ============================================================================

/** Generate or load a keypair from file */
function getOrCreateKeypair(name: string): Keypair {
  const filePath = path.join(WALLETS_DIR, `${name}.json`);

  if (fs.existsSync(filePath)) {
    const secretKey = new Uint8Array(JSON.parse(fs.readFileSync(filePath, "utf-8")));
    console.log(`  ✓ Loaded existing wallet: ${name}`);
    return Keypair.fromSecretKey(secretKey);
  }

  const keypair = Keypair.generate();
  fs.mkdirSync(WALLETS_DIR, { recursive: true });
  fs.writeFileSync(filePath, JSON.stringify(Array.from(keypair.secretKey)));
  console.log(`  ✓ Created new wallet: ${name}`);
  return keypair;
}

/** Airdrop SOL to an account */
async function airdrop(
  connection: Connection,
  publicKey: PublicKey,
  amount: number = 10 * LAMPORTS_PER_SOL
): Promise<void> {
  const signature = await connection.requestAirdrop(publicKey, amount);
  const latestBlockhash = await connection.getLatestBlockhash();
  await connection.confirmTransaction({
    signature,
    blockhash: latestBlockhash.blockhash,
    lastValidBlockHeight: latestBlockhash.lastValidBlockHeight,
  });
}

/** Get account balance in SOL */
async function getBalanceInSol(
  connection: Connection,
  publicKey: PublicKey
): Promise<number> {
  const balance = await connection.getBalance(publicKey);
  return balance / LAMPORTS_PER_SOL;
}

/** Generate escrow ID from hire ID */
function generateEscrowId(hireId: string): number[] {
  return Array.from(createHash("sha256").update(hireId).digest());
}

/** Derive PDAs */
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

/** Calculate commission */
function calculateCommission(amount: number, rateBps: number): number {
  return Math.floor((amount * rateBps) / 10000);
}

/** Sleep for ms */
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Parse escrow status */
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
// Main Test Flow
// ============================================================================

async function main() {
  console.log("\n" + "=".repeat(60));
  console.log("🚀 Worqen Escrow - Local Testing Script");
  console.log("=".repeat(60) + "\n");

  // Connect to localnet
  console.log("📡 Connecting to localnet...");
  const connection = new Connection(LOCALNET_URL, "confirmed");

  try {
    const version = await connection.getVersion();
    console.log(`  ✓ Connected to Solana ${version["solana-core"]}\n`);
  } catch (error) {
    console.error("  ✗ Failed to connect to localnet!");
    console.error("  → Make sure `solana-test-validator` is running\n");
    process.exit(1);
  }

  // Check if program is deployed
  console.log("📦 Checking program deployment...");
  console.log(`  Program ID: ${PROGRAM_ID.toBase58()}`);
  const programAccount = await connection.getAccountInfo(PROGRAM_ID);
  if (!programAccount) {
    console.error("  ✗ Program not deployed!");
    console.error("  → Run: anchor build && anchor deploy --provider.cluster localnet\n");
    process.exit(1);
  }
  console.log("  ✓ Program is deployed\n");

  // Create/load wallets
  console.log("👛 Setting up wallets...");
  const employer = getOrCreateKeypair("employer");
  const employee = getOrCreateKeypair("employee");
  const platformAuthority = getOrCreateKeypair("platform-authority");

  console.log(`\n  Employer:          ${employer.publicKey.toBase58()}`);
  console.log(`  Employee:          ${employee.publicKey.toBase58()}`);
  console.log(`  Platform Authority: ${platformAuthority.publicKey.toBase58()}\n`);

  // Airdrop SOL
  console.log("💰 Airdropping SOL to wallets...");
  await airdrop(connection, employer.publicKey, 100 * LAMPORTS_PER_SOL);
  console.log("  ✓ Employer: 100 SOL");
  await airdrop(connection, employee.publicKey, 1 * LAMPORTS_PER_SOL);
  console.log("  ✓ Employee: 1 SOL (for tx fees)");
  await airdrop(connection, platformAuthority.publicKey, 1 * LAMPORTS_PER_SOL);
  console.log("  ✓ Platform: 1 SOL (for tx fees)\n");

  // Setup Anchor provider and program
  const employerProvider = new AnchorProvider(
    connection,
    {
      publicKey: employer.publicKey,
      signTransaction: async (tx) => {
        tx.sign(employer);
        return tx;
      },
      signAllTransactions: async (txs) => {
        txs.forEach((tx) => tx.sign(employer));
        return txs;
      },
    },
    { commitment: "confirmed" }
  );

  const program = new Program(idl, PROGRAM_ID, employerProvider);

  // Test configuration
  const hireId = `test-hire-${Date.now()}`;
  const escrowId = generateEscrowId(hireId);
  const { escrowPda, vaultPda } = derivePDAs(escrowId, PROGRAM_ID);
  const workerPayment = 5 * LAMPORTS_PER_SOL; // 5 SOL
  const commissionRateBps = 150; // 1.5%
  const commissionAmount = calculateCommission(workerPayment, commissionRateBps);
  const totalDeposit = workerPayment + commissionAmount;

  console.log("📋 Test Configuration:");
  console.log(`  Hire ID:        ${hireId}`);
  console.log(`  Worker Payment: ${workerPayment / LAMPORTS_PER_SOL} SOL`);
  console.log(`  Commission:     ${commissionAmount / LAMPORTS_PER_SOL} SOL (${commissionRateBps / 100}%)`);
  console.log(`  Total Deposit:  ${totalDeposit / LAMPORTS_PER_SOL} SOL`);
  console.log(`  Escrow PDA:     ${escrowPda.toBase58()}`);
  console.log(`  Vault PDA:      ${vaultPda.toBase58()}\n`);

  // ============================================================================
  // STEP 1: Create Escrow
  // ============================================================================
  console.log("=".repeat(60));
  console.log("📝 STEP 1: Creating Escrow");
  console.log("=".repeat(60));

  const createTx = await program.methods
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

  console.log(`  ✓ Transaction: ${createTx}`);

  let escrowAccount = await program.account.escrow.fetch(escrowPda);
  console.log(`  ✓ Status: ${parseStatus(escrowAccount.status)}`);
  console.log(`  ✓ Amount: ${escrowAccount.amount.toNumber() / LAMPORTS_PER_SOL} SOL`);
  console.log(`  ✓ Commission: ${escrowAccount.commissionAmount.toNumber() / LAMPORTS_PER_SOL} SOL\n`);

  // ============================================================================
  // STEP 2: Deposit Funds
  // ============================================================================
  console.log("=".repeat(60));
  console.log("💵 STEP 2: Depositing Funds");
  console.log("=".repeat(60));

  const employerBalanceBefore = await getBalanceInSol(connection, employer.publicKey);
  console.log(`  Employer balance before: ${employerBalanceBefore.toFixed(4)} SOL`);

  const depositTx = await program.methods
    .depositSol()
    .accountsStrict({
      escrow: escrowPda,
      escrowVault: vaultPda,
      employer: employer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([employer])
    .rpc();

  console.log(`  ✓ Transaction: ${depositTx}`);

  const employerBalanceAfter = await getBalanceInSol(connection, employer.publicKey);
  const vaultBalance = await getBalanceInSol(connection, vaultPda);

  console.log(`  Employer balance after: ${employerBalanceAfter.toFixed(4)} SOL`);
  console.log(`  Vault balance: ${vaultBalance.toFixed(4)} SOL`);

  escrowAccount = await program.account.escrow.fetch(escrowPda);
  console.log(`  ✓ Status: ${parseStatus(escrowAccount.status)}\n`);

  // ============================================================================
  // STEP 3: Confirm Completion (Employer)
  // ============================================================================
  console.log("=".repeat(60));
  console.log("✅ STEP 3: Employer Confirms Completion");
  console.log("=".repeat(60));

  const confirmTx = await program.methods
    .confirmCompletion()
    .accountsStrict({
      escrow: escrowPda,
      signer: employer.publicKey,
    })
    .signers([employer])
    .rpc();

  console.log(`  ✓ Transaction: ${confirmTx}`);

  escrowAccount = await program.account.escrow.fetch(escrowPda);
  console.log(`  ✓ Status: ${parseStatus(escrowAccount.status)}`);
  console.log(`  ✓ Employer Confirmed: ${escrowAccount.employerConfirmed}\n`);

  // ============================================================================
  // STEP 4: Release Payment
  // ============================================================================
  console.log("=".repeat(60));
  console.log("🎉 STEP 4: Releasing Payment");
  console.log("=".repeat(60));

  const employeeBalanceBefore = await getBalanceInSol(connection, employee.publicKey);
  const platformBalanceBefore = await getBalanceInSol(connection, platformAuthority.publicKey);

  console.log(`  Employee balance before: ${employeeBalanceBefore.toFixed(4)} SOL`);
  console.log(`  Platform balance before: ${platformBalanceBefore.toFixed(4)} SOL`);

  const releaseTx = await program.methods
    .releaseSol()
    .accountsStrict({
      escrow: escrowPda,
      escrowVault: vaultPda,
      employee: employee.publicKey,
      platformAuthority: platformAuthority.publicKey,
      authority: employer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .signers([employer])
    .rpc();

  console.log(`  ✓ Transaction: ${releaseTx}`);

  const employeeBalanceAfter = await getBalanceInSol(connection, employee.publicKey);
  const platformBalanceAfter = await getBalanceInSol(connection, platformAuthority.publicKey);

  console.log(`  Employee balance after: ${employeeBalanceAfter.toFixed(4)} SOL`);
  console.log(`  Platform balance after: ${platformBalanceAfter.toFixed(4)} SOL`);
  console.log(`  → Employee received: ${(employeeBalanceAfter - employeeBalanceBefore).toFixed(4)} SOL`);
  console.log(`  → Platform received: ${(platformBalanceAfter - platformBalanceBefore).toFixed(4)} SOL`);

  escrowAccount = await program.account.escrow.fetch(escrowPda);
  console.log(`  ✓ Status: ${parseStatus(escrowAccount.status)}\n`);

  // ============================================================================
  // Summary
  // ============================================================================
  console.log("=".repeat(60));
  console.log("📊 SUMMARY");
  console.log("=".repeat(60));
  console.log(`  ✓ Escrow created and funded`);
  console.log(`  ✓ Work confirmed by employer`);
  console.log(`  ✓ Payment released successfully`);
  console.log(`  ✓ Employee received: ${workerPayment / LAMPORTS_PER_SOL} SOL`);
  console.log(`  ✓ Platform received commission: ${commissionAmount / LAMPORTS_PER_SOL} SOL`);
  console.log("\n🎉 Full escrow flow completed successfully!\n");

  // Print wallet info for reference
  console.log("=".repeat(60));
  console.log("📁 Wallet Files Location");
  console.log("=".repeat(60));
  console.log(`  ${WALLETS_DIR}/`);
  console.log(`    ├── employer.json`);
  console.log(`    ├── employee.json`);
  console.log(`    └── platform-authority.json\n`);
}

// Run main
main().catch((error) => {
  console.error("\n❌ Error:", error);
  process.exit(1);
});

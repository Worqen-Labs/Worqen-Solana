#!/usr/bin/env ts-node

/**
 * Worqen Escrow - Token Escrow Testing Script (Localnet)
 *
 * Tests the full SPL token escrow flow with a mock USDT token (6 decimals):
 * 1. Creates a test SPL token mint (simulating USDT)
 * 2. Mints tokens to employer
 * 3. Creates a token escrow
 * 4. Deposits tokens
 * 5. Confirms completion
 * 6. Releases payment (tokens to employee, commission to platform)
 *
 * Prerequisites:
 * - Run `solana-test-validator` in a separate terminal
 * - Run `anchor build && anchor deploy --provider.cluster localnet`
 *
 * Usage:
 *   npx ts-node scripts/test-token-escrow.ts
 *   # or
 *   bun scripts/test-token-escrow.ts
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
import {
  createMint,
  mintTo,
  getOrCreateAssociatedTokenAccount,
  getAccount,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";

// @ts-ignore - spl-token Signer type mismatch with web3.js Keypair
type AnyProgram = any;
import { createHash } from "crypto";
import * as fs from "fs";
import * as path from "path";

// ============================================================================
// Configuration
// ============================================================================

const LOCALNET_URL = "http://localhost:8899";
const WALLETS_DIR = path.join(__dirname, "../.localnet-wallets");
const TOKEN_DECIMALS = 6; // USDT uses 6 decimals

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

function getOrCreateKeypair(name: string): Keypair {
  const filePath = path.join(WALLETS_DIR, `${name}.json`);

  if (fs.existsSync(filePath)) {
    const secretKey = new Uint8Array(
      JSON.parse(fs.readFileSync(filePath, "utf-8"))
    );
    console.log(`  Loaded existing wallet: ${name}`);
    return Keypair.fromSecretKey(secretKey);
  }

  const keypair = Keypair.generate();
  fs.mkdirSync(WALLETS_DIR, { recursive: true });
  fs.writeFileSync(filePath, JSON.stringify(Array.from(keypair.secretKey)));
  console.log(`  Created new wallet: ${name}`);
  return keypair;
}

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

function formatTokenAmount(amount: number | bigint, decimals: number): string {
  const divisor = 10 ** decimals;
  return (Number(amount) / divisor).toFixed(decimals);
}

// ============================================================================
// Main Test Flow
// ============================================================================

async function main() {
  console.log("\n" + "=".repeat(60));
  console.log("Worqen Escrow - Token Escrow Test (Mock USDT)");
  console.log("=".repeat(60) + "\n");

  // Connect to localnet
  console.log("Connecting to localnet...");
  const connection = new Connection(LOCALNET_URL, "confirmed");

  try {
    const version = await connection.getVersion();
    console.log(`  Connected to Solana ${version["solana-core"]}\n`);
  } catch (error) {
    console.error("  Failed to connect to localnet!");
    console.error("  Make sure `solana-test-validator` is running\n");
    process.exit(1);
  }

  // Check if program is deployed
  console.log("Checking program deployment...");
  console.log(`  Program ID: ${PROGRAM_ID.toBase58()}`);
  const programAccount = await connection.getAccountInfo(PROGRAM_ID);
  if (!programAccount) {
    console.error("  Program not deployed!");
    console.error(
      "  Run: anchor build && anchor deploy --provider.cluster localnet\n"
    );
    process.exit(1);
  }
  console.log("  Program is deployed\n");

  // Create/load wallets
  console.log("Setting up wallets...");
  const employer = getOrCreateKeypair("employer");
  const employee = getOrCreateKeypair("employee");
  const platformAuthority = getOrCreateKeypair("platform-authority");
  const mintAuthority = getOrCreateKeypair("mint-authority");

  console.log(`\n  Employer:          ${employer.publicKey.toBase58()}`);
  console.log(`  Employee:          ${employee.publicKey.toBase58()}`);
  console.log(
    `  Platform Authority: ${platformAuthority.publicKey.toBase58()}`
  );
  console.log(`  Mint Authority:    ${mintAuthority.publicKey.toBase58()}\n`);

  // Airdrop SOL (needed for tx fees and account rent)
  console.log("Airdropping SOL for transaction fees...");
  await airdrop(connection, employer.publicKey, 10 * LAMPORTS_PER_SOL);
  console.log("  Employer: 10 SOL");
  await airdrop(connection, employee.publicKey, 1 * LAMPORTS_PER_SOL);
  console.log("  Employee: 1 SOL");
  await airdrop(connection, platformAuthority.publicKey, 1 * LAMPORTS_PER_SOL);
  console.log("  Platform: 1 SOL");
  await airdrop(connection, mintAuthority.publicKey, 1 * LAMPORTS_PER_SOL);
  console.log("  Mint Authority: 1 SOL\n");

  // ============================================================================
  // STEP 0: Create Mock USDT Token
  // ============================================================================
  console.log("=".repeat(60));
  console.log("STEP 0: Creating Mock USDT Token (6 decimals)");
  console.log("=".repeat(60));

  const tokenMint = await createMint(
    connection,
    mintAuthority, // payer
    mintAuthority.publicKey, // mint authority
    null, // freeze authority
    TOKEN_DECIMALS // decimals (USDT = 6)
  );
  console.log(`  Token Mint: ${tokenMint.toBase58()}`);

  // Create token accounts for all parties
  const employerTokenAccount = await getOrCreateAssociatedTokenAccount(
    connection,
    employer,
    tokenMint,
    employer.publicKey
  );
  console.log(`  Employer ATA: ${employerTokenAccount.address.toBase58()}`);

  const employeeTokenAccount = await getOrCreateAssociatedTokenAccount(
    connection,
    employee,
    tokenMint,
    employee.publicKey
  );
  console.log(`  Employee ATA: ${employeeTokenAccount.address.toBase58()}`);

  const platformTokenAccount = await getOrCreateAssociatedTokenAccount(
    connection,
    platformAuthority,
    tokenMint,
    platformAuthority.publicKey
  );
  console.log(`  Platform ATA: ${platformTokenAccount.address.toBase58()}`);

  // Mint tokens to employer (1000 USDT)
  const mintAmount = 1000 * 10 ** TOKEN_DECIMALS; // 1000 tokens
  await mintTo(
    connection,
    mintAuthority,
    tokenMint,
    employerTokenAccount.address,
    mintAuthority,
    mintAmount
  );
  console.log(
    `  Minted ${formatTokenAmount(mintAmount, TOKEN_DECIMALS)} tokens to employer\n`
  );

  // ============================================================================
  // Setup Anchor
  // ============================================================================
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

  const program = new Program(idl, employerProvider) as any;

  // Test configuration
  const hireId = `test-token-hire-${Date.now()}`;
  const escrowId = generateEscrowId(hireId);
  const { escrowPda } = derivePDAs(escrowId, PROGRAM_ID);
  const workerPayment = 100 * 10 ** TOKEN_DECIMALS; // 100 USDT
  const commissionRateBps = 150; // 1.5%
  const commissionAmount = calculateCommission(workerPayment, commissionRateBps);
  const totalDeposit = workerPayment + commissionAmount;

  // Derive vault ATA (associated token account owned by escrow PDA)
  const vaultTokenAccount = PublicKey.findProgramAddressSync(
    [
      escrowPda.toBuffer(),
      TOKEN_PROGRAM_ID.toBuffer(),
      tokenMint.toBuffer(),
    ],
    ASSOCIATED_TOKEN_PROGRAM_ID
  )[0];

  console.log("Test Configuration:");
  console.log(`  Hire ID:        ${hireId}`);
  console.log(
    `  Worker Payment: ${formatTokenAmount(workerPayment, TOKEN_DECIMALS)} tokens`
  );
  console.log(
    `  Commission:     ${formatTokenAmount(commissionAmount, TOKEN_DECIMALS)} tokens (${commissionRateBps / 100}%)`
  );
  console.log(
    `  Total Deposit:  ${formatTokenAmount(totalDeposit, TOKEN_DECIMALS)} tokens`
  );
  console.log(`  Escrow PDA:     ${escrowPda.toBase58()}`);
  console.log(`  Vault ATA:      ${vaultTokenAccount.toBase58()}\n`);

  // ============================================================================
  // STEP 1: Create Escrow
  // ============================================================================
  console.log("=".repeat(60));
  console.log("STEP 1: Creating Token Escrow");
  console.log("=".repeat(60));

  const createTx = await program.methods
    .createEscrow(
      escrowId,
      new BN(workerPayment),
      false, // is_native = false (token escrow)
      commissionRateBps
    )
    .accountsStrict({
      escrow: escrowPda,
      employer: employer.publicKey,
      employee: employee.publicKey,
      platformAuthority: platformAuthority.publicKey,
      tokenMint: tokenMint,
      systemProgram: SystemProgram.programId,
    })
    .signers([employer])
    .rpc();

  console.log(`  Tx: ${createTx}`);

  let escrowAccount = await program.account.escrow.fetch(escrowPda);
  console.log(`  Status: ${parseStatus(escrowAccount.status)}`);
  console.log(
    `  Amount: ${formatTokenAmount(escrowAccount.amount.toNumber(), TOKEN_DECIMALS)} tokens`
  );
  console.log(
    `  Commission: ${formatTokenAmount(escrowAccount.commissionAmount.toNumber(), TOKEN_DECIMALS)} tokens`
  );
  console.log(`  Is Native: ${escrowAccount.isNative}`);
  console.log(`  Token Mint: ${escrowAccount.tokenMint.toBase58()}\n`);

  // ============================================================================
  // STEP 2: Deposit Tokens
  // ============================================================================
  console.log("=".repeat(60));
  console.log("STEP 2: Depositing Tokens");
  console.log("=".repeat(60));

  const employerBalanceBefore = await getAccount(
    connection,
    employerTokenAccount.address
  );
  console.log(
    `  Employer token balance before: ${formatTokenAmount(employerBalanceBefore.amount, TOKEN_DECIMALS)}`
  );

  const depositTx = await program.methods
    .depositToken()
    .accountsStrict({
      escrow: escrowPda,
      vaultTokenAccount: vaultTokenAccount,
      employer: employer.publicKey,
      employerTokenAccount: employerTokenAccount.address,
      tokenMint: tokenMint,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([employer])
    .rpc();

  console.log(`  Tx: ${depositTx}`);

  const employerBalanceAfter = await getAccount(
    connection,
    employerTokenAccount.address
  );
  const vaultBalance = await getAccount(connection, vaultTokenAccount);

  console.log(
    `  Employer token balance after: ${formatTokenAmount(employerBalanceAfter.amount, TOKEN_DECIMALS)}`
  );
  console.log(
    `  Vault token balance: ${formatTokenAmount(vaultBalance.amount, TOKEN_DECIMALS)}`
  );

  escrowAccount = await program.account.escrow.fetch(escrowPda);
  console.log(`  Status: ${parseStatus(escrowAccount.status)}\n`);

  // ============================================================================
  // STEP 3: Confirm Completion (Employer)
  // ============================================================================
  console.log("=".repeat(60));
  console.log("STEP 3: Employer Confirms Completion");
  console.log("=".repeat(60));

  const confirmTx = await program.methods
    .confirmCompletion()
    .accountsStrict({
      escrow: escrowPda,
      signer: employer.publicKey,
    })
    .signers([employer])
    .rpc();

  console.log(`  Tx: ${confirmTx}`);

  escrowAccount = await program.account.escrow.fetch(escrowPda);
  console.log(`  Status: ${parseStatus(escrowAccount.status)}`);
  console.log(`  Employer Confirmed: ${escrowAccount.employerConfirmed}\n`);

  // ============================================================================
  // STEP 4: Release Tokens
  // ============================================================================
  console.log("=".repeat(60));
  console.log("STEP 4: Releasing Token Payment");
  console.log("=".repeat(60));

  const employeeBalanceBefore = await getAccount(
    connection,
    employeeTokenAccount.address
  );
  const platformBalanceBefore = await getAccount(
    connection,
    platformTokenAccount.address
  );

  console.log(
    `  Employee token balance before: ${formatTokenAmount(employeeBalanceBefore.amount, TOKEN_DECIMALS)}`
  );
  console.log(
    `  Platform token balance before: ${formatTokenAmount(platformBalanceBefore.amount, TOKEN_DECIMALS)}`
  );

  const releaseTx = await program.methods
    .releaseToken()
    .accountsStrict({
      escrow: escrowPda,
      vaultTokenAccount: vaultTokenAccount,
      employee: employee.publicKey,
      employeeTokenAccount: employeeTokenAccount.address,
      platformAuthority: platformAuthority.publicKey,
      platformTokenAccount: platformTokenAccount.address,
      authority: employer.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .signers([employer])
    .rpc();

  console.log(`  Tx: ${releaseTx}`);

  const employeeBalanceAfter = await getAccount(
    connection,
    employeeTokenAccount.address
  );
  const platformBalanceAfter = await getAccount(
    connection,
    platformTokenAccount.address
  );

  console.log(
    `  Employee token balance after: ${formatTokenAmount(employeeBalanceAfter.amount, TOKEN_DECIMALS)}`
  );
  console.log(
    `  Platform token balance after: ${formatTokenAmount(platformBalanceAfter.amount, TOKEN_DECIMALS)}`
  );
  console.log(
    `  Employee received: ${formatTokenAmount(employeeBalanceAfter.amount - employeeBalanceBefore.amount, TOKEN_DECIMALS)} tokens`
  );
  console.log(
    `  Platform received: ${formatTokenAmount(platformBalanceAfter.amount - platformBalanceBefore.amount, TOKEN_DECIMALS)} tokens`
  );

  escrowAccount = await program.account.escrow.fetch(escrowPda);
  console.log(`  Status: ${parseStatus(escrowAccount.status)}\n`);

  // ============================================================================
  // Summary
  // ============================================================================
  console.log("=".repeat(60));
  console.log("SUMMARY");
  console.log("=".repeat(60));
  console.log(`  Token Mint (Mock USDT): ${tokenMint.toBase58()}`);
  console.log(`  Escrow created and funded with tokens`);
  console.log(`  Work confirmed by employer`);
  console.log(`  Payment released successfully`);
  console.log(
    `  Employee received: ${formatTokenAmount(workerPayment, TOKEN_DECIMALS)} tokens`
  );
  console.log(
    `  Platform received commission: ${formatTokenAmount(commissionAmount, TOKEN_DECIMALS)} tokens`
  );
  console.log("\nFull token escrow flow completed successfully!\n");
}

// Run main
main().catch((error) => {
  console.error("\nError:", error);
  process.exit(1);
});

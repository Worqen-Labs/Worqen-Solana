import { Connection, PublicKey, LAMPORTS_PER_SOL } from '@solana/web3.js';
import * as anchor from '@coral-xyz/anchor';
import * as path from 'path';
import * as fs from 'fs';

const PROGRAM_ID = new PublicKey('GDCBqN8AVU5i2xXdeTNwBmCCsd9Y8rfiH1JDKA8UjDYh');
const connection = new Connection('http://127.0.0.1:8899', 'confirmed');

// Load IDL
const idlPath = path.join(__dirname, '../target/idl/worqen_escrow.json');
const idl = JSON.parse(fs.readFileSync(idlPath, 'utf8'));

// Status mapping
const STATUS_MAP: Record<number, string> = {
  0: 'Created',
  1: 'Funded',
  2: 'PendingRelease',
  3: 'Released',
  4: 'Disputed',
  5: 'Resolved',
  6: 'Cancelled',
};

async function main() {
  // Check for specific escrow PDA argument
  const specificPda = process.argv[2];

  console.log('🔍 Searching for escrow accounts...\n');

  // Setup provider
  const provider = new anchor.AnchorProvider(connection, {} as any, {});
  const program = new anchor.Program(idl, provider);

  // Get all program accounts (escrows)
  const accounts = await connection.getProgramAccounts(PROGRAM_ID);

  // Filter out IDL account (larger than escrow accounts)
  const escrowAccounts = accounts.filter(acc => acc.account.data.length === 376);

  console.log(`Found ${escrowAccounts.length} escrow account(s):\n`);

  for (const acc of escrowAccounts) {
    // Skip if looking for specific PDA and this isn't it
    if (specificPda && acc.pubkey.toBase58() !== specificPda) {
      continue;
    }

    const balance = acc.account.lamports / LAMPORTS_PER_SOL;

    try {
      // Decode escrow data
      const escrow = program.coder.accounts.decode('escrow', acc.account.data);

      console.log(`📦 Escrow PDA: ${acc.pubkey.toBase58()}`);
      console.log(`   Account Rent: ${balance} SOL`);
      console.log(`   ─────────────────────────────────────`);
      console.log(`   Escrow ID: ${Buffer.from(escrow.escrowId).toString('hex')}`);
      console.log(`   Status: ${STATUS_MAP[Object.keys(escrow.status)[0] === 'created' ? 0 : 
                                          Object.keys(escrow.status)[0] === 'funded' ? 1 :
                                          Object.keys(escrow.status)[0] === 'pendingRelease' ? 2 :
                                          Object.keys(escrow.status)[0] === 'released' ? 3 :
                                          Object.keys(escrow.status)[0] === 'disputed' ? 4 :
                                          Object.keys(escrow.status)[0] === 'resolved' ? 5 : 6]}`);
      console.log(`   ─────────────────────────────────────`);
      console.log(`   Employer: ${escrow.employer.toBase58()}`);
      console.log(`   Employee: ${escrow.employee.toBase58()}`);
      console.log(`   Platform Authority: ${escrow.platformAuthority.toBase58()}`);
      console.log(`   ─────────────────────────────────────`);
      console.log(`   💰 Amount: ${Number(escrow.amount) / LAMPORTS_PER_SOL} SOL (${escrow.amount.toString()} lamports)`);
      console.log(`   💸 Commission: ${Number(escrow.commissionAmount) / LAMPORTS_PER_SOL} SOL (${escrow.commissionAmount.toString()} lamports)`);
      console.log(`   📊 Commission Rate: ${escrow.commissionRateBps / 100}%`);
      console.log(`   🏦 Vault PDA: ${escrow.vaultPda.toBase58()}`);

      // Get vault balance
      const vaultBalance = await connection.getBalance(escrow.vaultPda);
      console.log(`   💎 Vault Balance: ${vaultBalance / LAMPORTS_PER_SOL} SOL (${vaultBalance} lamports)`);
      console.log(`   ─────────────────────────────────────`);
      console.log(`   Token Mint: ${escrow.tokenMint.toBase58()}`);
      console.log(`   Is SOL: ${escrow.tokenMint.toBase58() === '11111111111111111111111111111111'}`);
      console.log(`   Created At: ${new Date(Number(escrow.createdAt) * 1000).toISOString()}`);
      console.log('═══════════════════════════════════════════\n');
    } catch (e) {
      console.log(`📦 Address: ${acc.pubkey.toBase58()}`);
      console.log(`   Balance: ${balance} SOL`);
      console.log(`   (Could not decode escrow data)`);
      console.log('---');
    }
  }

  if (escrowAccounts.length === 0) {
    console.log('No escrow accounts found yet.');
    console.log('\nTo create an escrow, run the tests or use your application.');
  }
}

main().catch(console.error);

/**
 * Worqen Escrow — modern in-process test suite (LiteSVM + bun:test).
 *
 * Runs the compiled program fully in-process via LiteSVM — no local validator,
 * no devnet. Anchor is used only to *build* instructions from the IDL; LiteSVM
 * executes them. This unlocks clock-warp tests (dispute auto-release after the
 * deadline) that were impossible against a live validator.
 *
 *   bun test
 *
 * Covers every instruction (config, SOL + token escrow lifecycles, partial
 * releases, deposit_more, batch_pay, mutual_cancel, pay_with_commission,
 * disputes + resolve, auto-release before/after deadline, pause, mint
 * allowlist, two-step config authority, per-escrow authority rotation,
 * close/close_unfunded).
 */
import { describe, test, expect, beforeAll } from "bun:test";
import { LiteSvm, Clock } from "litesvm/dist/internal";
import * as anchor from "@coral-xyz/anchor";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  Connection,
  Transaction,
  LAMPORTS_PER_SOL,
  type TransactionInstruction,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  MINT_SIZE,
  createInitializeMint2Instruction,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  createMintToInstruction,
  AccountLayout,
} from "@solana/spl-token";
import BN from "bn.js";
import fs from "fs";

const IDL = JSON.parse(
  fs.readFileSync("target/idl/worqen_escrow.json", "utf8"),
);
const PROGRAM_ID = new PublicKey(IDL.address);
const SO_PATH = "target/deploy/worqen_escrow.so";

const ZEROS32 = Array(32).fill(0);
const seed = (s: string) => Buffer.from(s);
const rand32 = () => Array.from(Keypair.generate().publicKey.toBytes());

const BPS = 500;
const SOL_AMT = new BN(0.02 * LAMPORTS_PER_SOL);
const TOK_AMT = new BN(1_000_000); // 1 token @ 6 decimals
const commission = (amt: BN) => amt.muln(BPS).divn(10000);

const DAY = 24 * 3600;
// Base wall-clock time the SVM clock is pinned to in beforeAll. LiteSVM boots
// at unixTimestamp=0; a realistic base keeps created_at/funded_at sane and lets
// us derive dispute deadlines that satisfy the on-chain [now+3d, now+90d] bound.
const BASE_TS = 1_900_000_000;

// ─────────────────────────────────────────────────────────────────────────────
// Shared harness
// ─────────────────────────────────────────────────────────────────────────────
let svm: LiteSvm;
let program: anchor.Program;
let payer: Keypair; // employer + deployer + fee payer for every tx
let configPda: PublicKey;
let treasury: Keypair; // fee_recipient / config.fee_recipient
let mint: PublicKey;
let payerAta: PublicKey;
let treasuryAta: PublicKey;

/**
 * Build, sign and submit a legacy transaction.
 *
 * `expireBlockhash()` is called first so every tx gets a fresh blockhash — two
 * otherwise-identical instructions (e.g. equal partial-release slices) would
 * share a signature and LiteSVM would reject the second as AlreadyProcessed.
 */
function buildTx(
  ixs: TransactionInstruction[],
  signers: Keypair[],
): Transaction {
  svm.expireBlockhash();
  const tx = new Transaction().add(...ixs);
  tx.recentBlockhash = svm.latestBlockhash();
  tx.feePayer = payer.publicKey;
  tx.sign(payer, ...signers);
  return tx;
}

/** Throws with the program logs on failure; returns metadata on success. */
function send(ixs: TransactionInstruction[], signers: Keypair[] = []): any {
  const res = svm.sendLegacyTransaction(buildTx(ixs, signers).serialize());
  if (res.constructor.name === "FailedTransactionMetadata") {
    throw new Error("tx failed: " + JSON.stringify((res as any).meta().logs()));
  }
  return res;
}

/** Asserts a tx fails and that `code` (Anchor error name) appears in the logs. */
function expectFail(
  ixs: TransactionInstruction[],
  signers: Keypair[],
  code: string,
) {
  const res = svm.sendLegacyTransaction(buildTx(ixs, signers).serialize());
  expect(res.constructor.name).toBe("FailedTransactionMetadata");
  const logs = (res as any).meta().logs().join("\n");
  if (!logs.includes(code)) {
    throw new Error(`expected error "${code}" in logs but got:\n${logs}`);
  }
}

function balance(pk: PublicKey): number {
  const b = svm.getBalance(pk.toBytes());
  return b === null ? 0 : Number(b);
}

function decodeEscrow(escrow: PublicKey): any {
  const acc = svm.getAccount(escrow.toBytes());
  if (acc === null) throw new Error("escrow account not found");
  return program.coder.accounts.decode("escrow", Buffer.from(acc.data()));
}

function decodeConfig(): any {
  const acc = svm.getAccount(configPda.toBytes());
  if (acc === null) throw new Error("config account not found");
  return program.coder.accounts.decode("config", Buffer.from(acc.data()));
}

function tokenBalance(ata: PublicKey): bigint {
  const acc = svm.getAccount(ata.toBytes());
  if (acc === null) return 0n;
  // Normalize: spl-token's buffer-layout bigint does not compare strictly equal
  // under Bun (Object.is(x, 0n) === false). Round-trip through String to get a
  // clean primitive bigint that toBe()/=== work on.
  return BigInt(
    AccountLayout.decode(Buffer.from(acc.data())).amount.toString(),
  );
}

function escrowExists(escrow: PublicKey): boolean {
  return svm.getAccount(escrow.toBytes()) !== null;
}

/** Current SVM clock unix time as a JS number. */
function now(): number {
  return Number(svm.getClock().unixTimestamp);
}

/** Advance the SVM clock by `seconds` (also nudges slot so blockhash logic is happy). */
function warpBy(seconds: number) {
  const c = svm.getClock();
  svm.setClock(
    new Clock(
      c.slot + 1n,
      c.epochStartTimestamp,
      c.epoch,
      c.leaderScheduleEpoch,
      c.unixTimestamp + BigInt(seconds),
    ),
  );
}

function pdas(escrowId: number[]) {
  const [escrow] = PublicKey.findProgramAddressSync(
    [seed("escrow"), Buffer.from(escrowId)],
    PROGRAM_ID,
  );
  const [vault] = PublicKey.findProgramAddressSync(
    [seed("vault"), escrow.toBuffer()],
    PROGRAM_ID,
  );
  return { escrow, vault };
}

/** Fund a fresh keypair with SOL so it can be a fee payer / hold rent if needed. */
function fundedKeypair(sol = 10): Keypair {
  const kp = Keypair.generate();
  svm.airdrop(kp.publicKey.toBytes(), BigInt(sol * LAMPORTS_PER_SOL));
  return kp;
}

/** Create + initialize an empty ATA for `owner` (allowOwnerOffCurve for PDAs). */
function ensureAta(ownerPk: PublicKey, allowOffCurve = false): PublicKey {
  const ata = getAssociatedTokenAddressSync(mint, ownerPk, allowOffCurve);
  if (svm.getAccount(ata.toBytes()) === null) {
    const ix = createAssociatedTokenAccountInstruction(
      payer.publicKey,
      ata,
      ownerPk,
      mint,
    );
    send([ix]);
  }
  return ata;
}

/** Helper: create a SOL escrow (returns ids + PDAs + the employee/platform keypairs). */
function newSolEscrow() {
  const id = rand32();
  const { escrow, vault } = pdas(id);
  const employee = Keypair.generate();
  const platformAuthority = fundedKeypair(); // funded so it can co-sign + pay ATA rent in token paths
  return { id, escrow, vault, employee, platformAuthority };
}

async function createSolEscrow(
  ctx: ReturnType<typeof newSolEscrow>,
  amount = SOL_AMT,
) {
  const ix = await program.methods
    .createEscrow(
      ctx.id,
      ZEROS32,
      0,
      0,
      amount,
      true,
      BPS,
      new BN(0),
      0,
      ZEROS32,
    )
    .accountsPartial({
      escrow: ctx.escrow,
      config: configPda,
      employer: payer.publicKey,
      employee: ctx.employee.publicKey,
      platformAuthority: ctx.platformAuthority.publicKey,
      tokenMint: SystemProgram.programId,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  send([ix]);
}

async function depositSol(ctx: ReturnType<typeof newSolEscrow>) {
  const ix = await program.methods
    .depositSol()
    .accountsPartial({
      escrow: ctx.escrow,
      escrowVault: ctx.vault,
      employer: payer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  send([ix]);
}

function newTokenEscrow() {
  const id = rand32();
  const { escrow } = pdas(id);
  const employee = Keypair.generate();
  const platformAuthority = fundedKeypair();
  const vaultAta = getAssociatedTokenAddressSync(mint, escrow, true);
  return { id, escrow, employee, platformAuthority, vaultAta };
}

async function createTokenEscrow(
  ctx: ReturnType<typeof newTokenEscrow>,
  amount = TOK_AMT,
) {
  const ix = await program.methods
    .createEscrow(
      ctx.id,
      ZEROS32,
      0,
      0,
      amount,
      false,
      BPS,
      new BN(0),
      0,
      ZEROS32,
    )
    .accountsPartial({
      escrow: ctx.escrow,
      config: configPda,
      employer: payer.publicKey,
      employee: ctx.employee.publicKey,
      platformAuthority: ctx.platformAuthority.publicKey,
      tokenMint: mint,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  send([ix]);
}

async function depositToken(ctx: ReturnType<typeof newTokenEscrow>) {
  const ix = await program.methods
    .depositToken()
    .accountsPartial({
      escrow: ctx.escrow,
      vaultTokenAccount: ctx.vaultAta,
      employer: payer.publicKey,
      employerTokenAccount: payerAta,
      tokenMint: mint,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  send([ix]);
}

async function confirm(escrow: PublicKey, signer: Keypair) {
  const ix = await program.methods
    .confirmCompletion()
    .accountsPartial({ escrow, signer: signer.publicKey })
    .instruction();
  send([ix], signer.publicKey.equals(payer.publicKey) ? [] : [signer]);
}

beforeAll(async () => {
  svm = new LiteSvm();
  svm.addProgramFromFile(PROGRAM_ID.toBytes(), SO_PATH);

  // Pin the clock to a realistic base so all on-chain timestamps look sane.
  const c0 = svm.getClock();
  svm.setClock(
    new Clock(
      c0.slot,
      c0.epochStartTimestamp,
      c0.epoch,
      c0.leaderScheduleEpoch,
      BigInt(BASE_TS),
    ),
  );

  payer = Keypair.generate();
  svm.airdrop(payer.publicKey.toBytes(), BigInt(1000 * LAMPORTS_PER_SOL));

  treasury = Keypair.generate();
  svm.airdrop(treasury.publicKey.toBytes(), BigInt(1 * LAMPORTS_PER_SOL));

  // Anchor program over a never-touched dummy connection — we only call
  // .instruction(), never .rpc(). LiteSVM does the execution.
  const conn = new Connection("http://localhost:8899");
  const wallet = new anchor.Wallet(payer);
  const provider = new anchor.AnchorProvider(conn, wallet, {
    commitment: "processed",
  });
  program = new anchor.Program(IDL as anchor.Idl, provider);
  [configPda] = PublicKey.findProgramAddressSync([seed("config")], PROGRAM_ID);

  // init_config (fee_recipient = treasury, default 500 bps, empty allowlist).
  const initIx = await program.methods
    .initConfig(treasury.publicKey, BPS, [])
    .accountsPartial({
      config: configPda,
      authority: payer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  send([initIx]);

  // SPL mint + payer ATA + mint tokens + treasury ATA.
  const mintKp = Keypair.generate();
  mint = mintKp.publicKey;
  const rent = svm.minimumBalanceForRentExemption(BigInt(MINT_SIZE));
  const createMintIx = SystemProgram.createAccount({
    fromPubkey: payer.publicKey,
    newAccountPubkey: mint,
    space: MINT_SIZE,
    lamports: Number(rent),
    programId: TOKEN_PROGRAM_ID,
  });
  const initMintIx = createInitializeMint2Instruction(
    mint,
    6,
    payer.publicKey,
    null,
  );
  send([createMintIx, initMintIx], [mintKp]);

  payerAta = getAssociatedTokenAddressSync(mint, payer.publicKey);
  send([
    createAssociatedTokenAccountInstruction(
      payer.publicKey,
      payerAta,
      payer.publicKey,
      mint,
    ),
    createMintToInstruction(mint, payerAta, payer.publicKey, 1_000_000_000_000),
  ]);

  treasuryAta = getAssociatedTokenAddressSync(mint, treasury.publicKey);
  send([
    createAssociatedTokenAccountInstruction(
      payer.publicKey,
      treasuryAta,
      treasury.publicKey,
      mint,
    ),
  ]);

  // add_allowed_mint for the test mint.
  const addMintIx = await program.methods
    .addAllowedMint(mint)
    .accountsPartial({ config: configPda, authority: payer.publicKey })
    .instruction();
  send([addMintIx]);
});

// ─────────────────────────────────────────────────────────────────────────────
// Config
// ─────────────────────────────────────────────────────────────────────────────
describe("config", () => {
  test("config is readable with expected defaults", () => {
    const cfg = decodeConfig();
    expect(cfg.paused).toBe(false);
    expect(cfg.defaultCommissionBps).toBe(BPS);
    expect(cfg.feeRecipient.toBase58()).toBe(treasury.publicKey.toBase58());
    expect(
      cfg.allowedMints.some((m: PublicKey) => m.toBase58() === mint.toBase58()),
    ).toBe(true);
  });

  test("update_config: default commission bps persists, then restores", async () => {
    const ix1 = await program.methods
      .updateConfig(null, 300, null, null)
      .accountsPartial({ config: configPda, authority: payer.publicKey })
      .instruction();
    send([ix1]);
    expect(decodeConfig().defaultCommissionBps).toBe(300);

    const ix2 = await program.methods
      .updateConfig(null, BPS, null, null)
      .accountsPartial({ config: configPda, authority: payer.publicKey })
      .instruction();
    send([ix2]);
    expect(decodeConfig().defaultCommissionBps).toBe(BPS);
  });

  test("two-step config authority handoff (and restore)", async () => {
    const newAuth = fundedKeypair();

    const proposeIx = await program.methods
      .updateConfig(null, null, null, newAuth.publicKey)
      .accountsPartial({ config: configPda, authority: payer.publicKey })
      .instruction();
    send([proposeIx]);

    const acceptIx = await program.methods
      .acceptAuthority()
      .accountsPartial({
        config: configPda,
        pendingAuthority: newAuth.publicKey,
      })
      .instruction();
    send([acceptIx], [newAuth]);
    expect(decodeConfig().authority.toBase58()).toBe(
      newAuth.publicKey.toBase58(),
    );

    // restore: new authority proposes payer, payer accepts.
    const proposeBack = await program.methods
      .updateConfig(null, null, null, payer.publicKey)
      .accountsPartial({ config: configPda, authority: newAuth.publicKey })
      .instruction();
    send([proposeBack], [newAuth]);
    const acceptBack = await program.methods
      .acceptAuthority()
      .accountsPartial({ config: configPda, pendingAuthority: payer.publicKey })
      .instruction();
    send([acceptBack]);
    expect(decodeConfig().authority.toBase58()).toBe(
      payer.publicKey.toBase58(),
    );
  });

  test("mint allowlist: add then remove rejects create_escrow, re-add restores", async () => {
    const tempMint = Keypair.generate().publicKey;

    const addIx = await program.methods
      .addAllowedMint(tempMint)
      .accountsPartial({ config: configPda, authority: payer.publicKey })
      .instruction();
    send([addIx]);
    expect(
      decodeConfig().allowedMints.some(
        (m: PublicKey) => m.toBase58() === tempMint.toBase58(),
      ),
    ).toBe(true);

    const removeIx = await program.methods
      .removeAllowedMint(tempMint)
      .accountsPartial({ config: configPda, authority: payer.publicKey })
      .instruction();
    send([removeIx]);
    expect(
      decodeConfig().allowedMints.some(
        (m: PublicKey) => m.toBase58() === tempMint.toBase58(),
      ),
    ).toBe(false);

    // create_escrow with the removed mint must fail MintNotAllowed.
    const ctx = newTokenEscrow();
    const badIx = await program.methods
      .createEscrow(
        ctx.id,
        ZEROS32,
        0,
        0,
        TOK_AMT,
        false,
        BPS,
        new BN(0),
        0,
        ZEROS32,
      )
      .accountsPartial({
        escrow: ctx.escrow,
        config: configPda,
        employer: payer.publicKey,
        employee: ctx.employee.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        tokenMint: tempMint,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    expectFail([badIx], [], "MintNotAllowed");

    // re-add to restore allowlist state.
    const readd = await program.methods
      .addAllowedMint(tempMint)
      .accountsPartial({ config: configPda, authority: payer.publicKey })
      .instruction();
    send([readd]);
    expect(
      decodeConfig().allowedMints.some(
        (m: PublicKey) => m.toBase58() === tempMint.toBase58(),
      ),
    ).toBe(true);
  });

  test("a non-allowlisted mint is rejected by create_escrow", async () => {
    const badMint = Keypair.generate().publicKey;
    const ctx = newTokenEscrow();
    const ix = await program.methods
      .createEscrow(
        ctx.id,
        ZEROS32,
        0,
        0,
        TOK_AMT,
        false,
        BPS,
        new BN(0),
        0,
        ZEROS32,
      )
      .accountsPartial({
        escrow: ctx.escrow,
        config: configPda,
        employer: payer.publicKey,
        employee: ctx.employee.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        tokenMint: badMint,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    expectFail([ix], [], "MintNotAllowed");
  });

  test("pause blocks new escrows; unpausing restores", async () => {
    const pauseIx = await program.methods
      .updateConfig(null, null, true, null)
      .accountsPartial({ config: configPda, authority: payer.publicKey })
      .instruction();
    send([pauseIx]);

    const ctx = newSolEscrow();
    const createIx = await program.methods
      .createEscrow(
        ctx.id,
        ZEROS32,
        0,
        0,
        SOL_AMT,
        true,
        BPS,
        new BN(0),
        0,
        ZEROS32,
      )
      .accountsPartial({
        escrow: ctx.escrow,
        config: configPda,
        employer: payer.publicKey,
        employee: ctx.employee.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        tokenMint: SystemProgram.programId,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    expectFail([createIx], [], "ProgramPaused");

    const unpauseIx = await program.methods
      .updateConfig(null, null, false, null)
      .accountsPartial({ config: configPda, authority: payer.publicKey })
      .instruction();
    send([unpauseIx]);
    expect(decodeConfig().paused).toBe(false);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// SOL escrow lifecycle
// ─────────────────────────────────────────────────────────────────────────────
describe("SOL escrow", () => {
  test("full lifecycle: employee paid in full (fee-on-top), treasury gets commission", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);
    expect(balance(ctx.vault)).toBe(
      SOL_AMT.add(commission(SOL_AMT)).toNumber(),
    );

    await confirm(ctx.escrow, payer);
    await confirm(ctx.escrow, ctx.employee);

    const empBefore = balance(ctx.employee.publicKey);
    const treBefore = balance(treasury.publicKey);
    const releaseIx = await program.methods
      .releaseSol(ZEROS32)
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employee: ctx.employee.publicKey,
        feeRecipient: treasury.publicKey,
        authority: payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([releaseIx]);

    expect(balance(ctx.employee.publicKey) - empBefore).toBe(
      SOL_AMT.toNumber(),
    );
    expect(balance(treasury.publicKey) - treBefore).toBe(
      commission(SOL_AMT).toNumber(),
    );
    expect(balance(ctx.vault)).toBe(0);
    expect(decodeEscrow(ctx.escrow).status.released).toBeDefined();
  });

  test("partial release: delta commission, sum-of-partials correct", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);

    const empBefore = balance(ctx.employee.publicKey);
    const treBefore = balance(treasury.publicKey);
    const half = SOL_AMT.divn(2);

    const partial = async (amt: BN) => {
      const ix = await program.methods
        .releasePartialSol(amt, ZEROS32)
        .accountsPartial({
          escrow: ctx.escrow,
          escrowVault: ctx.vault,
          employee: ctx.employee.publicKey,
          feeRecipient: treasury.publicKey,
          authority: payer.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .instruction();
      send([ix]);
    };
    await partial(half);
    await partial(SOL_AMT.sub(half));

    expect(balance(ctx.employee.publicKey) - empBefore).toBe(
      SOL_AMT.toNumber(),
    );
    expect(balance(treasury.publicKey) - treBefore).toBe(
      commission(SOL_AMT).toNumber(),
    );
    expect(balance(ctx.vault)).toBe(0);
    expect(decodeEscrow(ctx.escrow).status.released).toBeDefined();
  });

  test("partial release allowed in PendingRelease", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);
    await confirm(ctx.escrow, payer); // -> PendingRelease

    const empBefore = balance(ctx.employee.publicKey);
    const ix = await program.methods
      .releasePartialSol(SOL_AMT.divn(2), ZEROS32)
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employee: ctx.employee.publicKey,
        feeRecipient: treasury.publicKey,
        authority: payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([ix]);
    expect(balance(ctx.employee.publicKey) - empBefore).toBeGreaterThan(0);
  });

  test("deposit_more_sol: raises amount + commission and funds the vault", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);

    const vaultBefore = balance(ctx.vault);
    const add = new BN(0.01 * LAMPORTS_PER_SOL);
    const ix = await program.methods
      .depositMoreSol(add)
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employer: payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([ix]);

    expect(decodeEscrow(ctx.escrow).amount.toString()).toBe(
      SOL_AMT.add(add).toString(),
    );
    const expectedDelta = add
      .add(commission(SOL_AMT.add(add)))
      .sub(commission(SOL_AMT));
    expect(balance(ctx.vault) - vaultBefore).toBe(expectedDelta.toNumber());
  });

  test("mutual_cancel_sol: employer + employee settle with a split", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);

    const share = SOL_AMT.divn(3);
    const empBefore = balance(ctx.employee.publicKey);
    const ix = await program.methods
      .mutualCancelSol(share)
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employer: payer.publicKey,
        employee: ctx.employee.publicKey,
        feeRecipient: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([ix], [ctx.employee]);

    expect(balance(ctx.employee.publicKey) - empBefore).toBe(share.toNumber());
    expect(balance(ctx.vault)).toBe(0);
    expect(decodeEscrow(ctx.escrow).status.resolved).toBeDefined();
  });

  test("cancel (Created) then close_unfunded reclaims rent", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);

    const cancelIx = await program.methods
      .cancelEscrowSol(Buffer.from("changed mind"))
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employer: payer.publicKey,
        feeRecipient: treasury.publicKey,
        signer: payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([cancelIx]);

    const before = balance(payer.publicKey);
    const closeIx = await program.methods
      .closeUnfundedEscrowSol()
      .accountsPartial({
        escrow: ctx.escrow,
        employer: payer.publicKey,
        signer: payer.publicKey,
      })
      .instruction();
    send([closeIx]);
    expect(balance(payer.publicKey)).toBeGreaterThan(before);
    expect(escrowExists(ctx.escrow)).toBe(false);
  });

  test("close_escrow_sol: release then close reclaims vault+account rent", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);
    await confirm(ctx.escrow, payer);
    await confirm(ctx.escrow, ctx.employee);
    const releaseIx = await program.methods
      .releaseSol(ZEROS32)
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employee: ctx.employee.publicKey,
        feeRecipient: treasury.publicKey,
        authority: payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([releaseIx]);

    const before = balance(payer.publicKey);
    const closeIx = await program.methods
      .closeEscrowSol()
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employer: payer.publicKey,
        signer: payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([closeIx]);
    expect(balance(payer.publicKey)).toBeGreaterThan(before);
    expect(escrowExists(ctx.escrow)).toBe(false);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Token escrow lifecycle
// ─────────────────────────────────────────────────────────────────────────────
describe("token escrow", () => {
  test("full lifecycle (fee-on-top) to employee + treasury ATAs", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);
    expect(tokenBalance(ctx.vaultAta)).toBe(
      BigInt(TOK_AMT.add(commission(TOK_AMT)).toString()),
    );

    await confirm(ctx.escrow, payer);
    await confirm(ctx.escrow, ctx.employee);

    const employeeAta = getAssociatedTokenAddressSync(
      mint,
      ctx.employee.publicKey,
    );
    const treBefore = tokenBalance(treasuryAta);
    const releaseIx = await program.methods
      .releaseToken(ZEROS32)
      .accountsPartial({
        escrow: ctx.escrow,
        tokenMint: mint,
        vaultTokenAccount: ctx.vaultAta,
        employee: ctx.employee.publicKey,
        employeeTokenAccount: employeeAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        authority: payer.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([releaseIx]);

    expect(tokenBalance(employeeAta)).toBe(BigInt(TOK_AMT.toString()));
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(TOK_AMT).toString()),
    );
  });

  test("employee self-releases when both parties confirmed", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);
    await confirm(ctx.escrow, payer);
    await confirm(ctx.escrow, ctx.employee);

    // Pre-create the employee ATA: the employee is the release authority here
    // but holds 0 SOL, so it cannot fund the on-demand ATA init rent itself.
    const employeeAta = ensureAta(ctx.employee.publicKey);
    const treBefore = tokenBalance(treasuryAta);
    // Employee is the release authority but holds 0 SOL; payer is feePayer.
    const ix = await program.methods
      .releaseToken(ZEROS32)
      .accountsPartial({
        escrow: ctx.escrow,
        tokenMint: mint,
        vaultTokenAccount: ctx.vaultAta,
        employee: ctx.employee.publicKey,
        employeeTokenAccount: employeeAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        authority: ctx.employee.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([ix], [ctx.employee]);

    expect(tokenBalance(employeeAta)).toBe(BigInt(TOK_AMT.toString()));
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(TOK_AMT).toString()),
    );
    expect(decodeEscrow(ctx.escrow).releaseInitiator.toBase58()).toBe(
      ctx.employee.publicKey.toBase58(),
    );
  });

  test("release_partial_token: delta-commission slices pay employee + treasury ATAs", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);

    const employeeAta = getAssociatedTokenAddressSync(
      mint,
      ctx.employee.publicKey,
    );
    const treBefore = tokenBalance(treasuryAta);
    const half = TOK_AMT.divn(2);
    const partial = async (amt: BN) => {
      const ix = await program.methods
        .releasePartialToken(amt, ZEROS32)
        .accountsPartial({
          escrow: ctx.escrow,
          tokenMint: mint,
          vaultTokenAccount: ctx.vaultAta,
          employee: ctx.employee.publicKey,
          employeeTokenAccount: employeeAta,
          feeRecipient: treasury.publicKey,
          platformTokenAccount: treasuryAta,
          authority: payer.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .instruction();
      send([ix]);
    };
    await partial(half);
    await partial(TOK_AMT.sub(half));

    expect(tokenBalance(employeeAta)).toBe(BigInt(TOK_AMT.toString()));
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(TOK_AMT).toString()),
    );
    expect(tokenBalance(ctx.vaultAta)).toBe(0n);
    expect(decodeEscrow(ctx.escrow).status.released).toBeDefined();
  });

  test("deposit_more_token: top-up raises amount + commission and funds the vault ATA", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);

    const vaultBefore = tokenBalance(ctx.vaultAta);
    const add = new BN(500_000);
    const ix = await program.methods
      .depositMoreToken(add)
      .accountsPartial({
        escrow: ctx.escrow,
        vaultTokenAccount: ctx.vaultAta,
        employer: payer.publicKey,
        employerTokenAccount: payerAta,
        tokenMint: mint,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([ix]);

    expect(decodeEscrow(ctx.escrow).amount.toString()).toBe(
      TOK_AMT.add(add).toString(),
    );
    const expectedDelta = add
      .add(commission(TOK_AMT.add(add)))
      .sub(commission(TOK_AMT));
    expect(tokenBalance(ctx.vaultAta) - vaultBefore).toBe(
      BigInt(expectedDelta.toString()),
    );
  });

  test("mutual_cancel_token: employer + employee settle with a split", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);

    const employeeAta = getAssociatedTokenAddressSync(
      mint,
      ctx.employee.publicKey,
    );
    const share = TOK_AMT.divn(3);
    const ix = await program.methods
      .mutualCancelToken(share)
      .accountsPartial({
        escrow: ctx.escrow,
        tokenMint: mint,
        vaultTokenAccount: ctx.vaultAta,
        employer: payer.publicKey,
        employerTokenAccount: payerAta,
        employee: ctx.employee.publicKey,
        employeeTokenAccount: employeeAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([ix], [ctx.employee]);

    expect(tokenBalance(employeeAta)).toBe(BigInt(share.toString()));
    expect(tokenBalance(ctx.vaultAta)).toBe(0n);
    expect(decodeEscrow(ctx.escrow).status.resolved).toBeDefined();
  });

  test("cancel_escrow_token: platform cancels a funded token escrow; platform keeps the fee", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);

    const empBefore = tokenBalance(payerAta);
    const treBefore = tokenBalance(treasuryAta);
    // In Funded only platform_authority may cancel; payer is feePayer, platformAuthority co-signs.
    const ix = await program.methods
      .cancelEscrowToken(Buffer.from("platform refund"))
      .accountsPartial({
        escrow: ctx.escrow,
        vaultTokenAccount: ctx.vaultAta,
        employer: payer.publicKey,
        employerTokenAccount: payerAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        signer: ctx.platformAuthority.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .instruction();
    send([ix], [ctx.platformAuthority]);

    // Employer is refunded the worker deposit only; the platform KEEPS its
    // commission (routed to the treasury ATA), never refunded.
    expect(tokenBalance(payerAta) - empBefore).toBe(BigInt(TOK_AMT.toString()));
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(TOK_AMT).toString()),
    );
    expect(tokenBalance(ctx.vaultAta)).toBe(0n);
    expect(decodeEscrow(ctx.escrow).status.cancelled).toBeDefined();
  });

  test("close_escrow_token: release then close sweeps + closes vault ATA and escrow", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);
    await confirm(ctx.escrow, payer);
    await confirm(ctx.escrow, ctx.employee);
    const employeeAta = getAssociatedTokenAddressSync(
      mint,
      ctx.employee.publicKey,
    );
    const releaseIx = await program.methods
      .releaseToken(ZEROS32)
      .accountsPartial({
        escrow: ctx.escrow,
        tokenMint: mint,
        vaultTokenAccount: ctx.vaultAta,
        employee: ctx.employee.publicKey,
        employeeTokenAccount: employeeAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        authority: payer.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([releaseIx]);

    const before = balance(payer.publicKey);
    const closeIx = await program.methods
      .closeEscrowToken()
      .accountsPartial({
        escrow: ctx.escrow,
        vaultTokenAccount: ctx.vaultAta,
        employerTokenAccount: payerAta,
        employer: payer.publicKey,
        signer: payer.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .instruction();
    send([closeIx]);
    expect(balance(payer.publicKey)).toBeGreaterThan(before);
    expect(svm.getAccount(ctx.vaultAta.toBytes())).toBeNull();
    expect(escrowExists(ctx.escrow)).toBe(false);
  });

  test("close_unfunded_escrow_token: cancel a never-funded escrow then reclaim rent", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    // Create the empty vault ATA so cancel_escrow_token has a 0-balance vault to read.
    const vaultAta = ensureAta(ctx.escrow, true);
    const cancelIx = await program.methods
      .cancelEscrowToken(Buffer.from("changed mind"))
      .accountsPartial({
        escrow: ctx.escrow,
        vaultTokenAccount: vaultAta,
        employer: payer.publicKey,
        employerTokenAccount: payerAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        signer: payer.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .instruction();
    send([cancelIx]);

    const before = balance(payer.publicKey);
    const closeIx = await program.methods
      .closeUnfundedEscrowToken()
      .accountsPartial({
        escrow: ctx.escrow,
        employer: payer.publicKey,
        signer: payer.publicKey,
      })
      .instruction();
    send([closeIx]);
    expect(balance(payer.publicKey)).toBeGreaterThan(before);
    expect(escrowExists(ctx.escrow)).toBe(false);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Direct pay (no escrow)
// ─────────────────────────────────────────────────────────────────────────────
describe("direct pay", () => {
  test("pay_with_commission_sol: single recipient + commission to treasury", async () => {
    const recipient = Keypair.generate();
    const amount = new BN(0.006 * LAMPORTS_PER_SOL);
    const treBefore = balance(treasury.publicKey);
    const ix = await program.methods
      .payWithCommissionSol(rand32(), amount, BPS)
      .accountsPartial({
        payer: payer.publicKey,
        recipient: recipient.publicKey,
        feeRecipient: treasury.publicKey,
        config: configPda,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([ix]);
    expect(balance(recipient.publicKey)).toBe(amount.toNumber());
    expect(balance(treasury.publicKey) - treBefore).toBe(
      commission(amount).toNumber(),
    );
  });

  test("pay_with_commission_token: single recipient ATA + commission to treasury ATA", async () => {
    const recipient = Keypair.generate();
    const recipientAta = ensureAta(recipient.publicKey);
    const amount = new BN(250_000);
    const treBefore = tokenBalance(treasuryAta);
    const ix = await program.methods
      .payWithCommissionToken(rand32(), amount, BPS)
      .accountsPartial({
        config: configPda,
        payer: payer.publicKey,
        payerTokenAccount: payerAta,
        recipient: recipient.publicKey,
        recipientTokenAccount: recipientAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        tokenMint: mint,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .instruction();
    send([ix]);
    expect(tokenBalance(recipientAta)).toBe(BigInt(amount.toString()));
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(amount).toString()),
    );
  });

  test("batch_pay_with_commission_sol: multi-recipient + one commission on total", async () => {
    const r1 = Keypair.generate();
    const r2 = Keypair.generate();
    const a1 = new BN(0.005 * LAMPORTS_PER_SOL);
    const a2 = new BN(0.007 * LAMPORTS_PER_SOL);
    const total = a1.add(a2);
    const treBefore = balance(treasury.publicKey);
    const ix = await program.methods
      .batchPayWithCommissionSol(rand32(), [a1, a2], BPS)
      .accountsPartial({
        payer: payer.publicKey,
        config: configPda,
        feeRecipient: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts([
        { pubkey: r1.publicKey, isWritable: true, isSigner: false },
        { pubkey: r2.publicKey, isWritable: true, isSigner: false },
      ])
      .instruction();
    send([ix]);
    expect(balance(r1.publicKey)).toBe(a1.toNumber());
    expect(balance(r2.publicKey)).toBe(a2.toNumber());
    expect(balance(treasury.publicKey) - treBefore).toBe(
      commission(total).toNumber(),
    );
  });

  test("batch_pay_with_commission_token: multi-recipient ATAs + one commission on total", async () => {
    const r1 = Keypair.generate();
    const r2 = Keypair.generate();
    const r1Ata = ensureAta(r1.publicKey);
    const r2Ata = ensureAta(r2.publicKey);
    const a1 = new BN(300_000);
    const a2 = new BN(450_000);
    const total = a1.add(a2);
    const treBefore = tokenBalance(treasuryAta);
    const ix = await program.methods
      .batchPayWithCommissionToken(rand32(), [a1, a2], BPS)
      .accountsPartial({
        config: configPda,
        payer: payer.publicKey,
        payerTokenAccount: payerAta,
        feeTokenAccount: treasuryAta,
        tokenMint: mint,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts([
        { pubkey: r1Ata, isWritable: true, isSigner: false },
        { pubkey: r2Ata, isWritable: true, isSigner: false },
      ])
      .instruction();
    send([ix]);
    expect(tokenBalance(r1Ata)).toBe(BigInt(a1.toString()));
    expect(tokenBalance(r2Ata)).toBe(BigInt(a2.toString()));
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(total).toString()),
    );
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Disputes + resolution
// ─────────────────────────────────────────────────────────────────────────────
describe("disputes", () => {
  test("raise (valid window) then platform resolves SOL with a split", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);

    const raiseIx = await program.methods
      .raiseDispute(Buffer.from("quality"), new BN(now() + 5 * DAY))
      .accountsPartial({ escrow: ctx.escrow, signer: payer.publicKey })
      .instruction();
    send([raiseIx]);
    expect(decodeEscrow(ctx.escrow).status.disputed).toBeDefined();

    const empBefore = balance(ctx.employee.publicKey);
    const treBefore = balance(treasury.publicKey);
    const share = SOL_AMT.divn(2);
    const resolveIx = await program.methods
      .resolveDisputeSol(share)
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employer: payer.publicKey,
        employee: ctx.employee.publicKey,
        feeRecipient: treasury.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([resolveIx], [ctx.platformAuthority]);

    // Worker gets their share; the platform KEEPS the full commission (routed
    // to the treasury), never refunded to the employer.
    expect(balance(ctx.employee.publicKey) - empBefore).toBe(share.toNumber());
    expect(balance(treasury.publicKey) - treBefore).toBe(
      commission(SOL_AMT).toNumber(),
    );
    expect(balance(ctx.vault)).toBe(0);
    expect(decodeEscrow(ctx.escrow).status.resolved).toBeDefined();
  });

  test("resolve_dispute_token: platform resolves a disputed token escrow with a split", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);
    const employeeAta = ensureAta(ctx.employee.publicKey);

    const raiseIx = await program.methods
      .raiseDispute(Buffer.from("quality"), new BN(now() + 4 * DAY))
      .accountsPartial({ escrow: ctx.escrow, signer: payer.publicKey })
      .instruction();
    send([raiseIx]);

    const treBefore = tokenBalance(treasuryAta);
    const share = TOK_AMT.divn(2);
    const resolveIx = await program.methods
      .resolveDisputeToken(share)
      .accountsPartial({
        escrow: ctx.escrow,
        tokenMint: mint,
        vaultTokenAccount: ctx.vaultAta,
        employer: payer.publicKey,
        employerTokenAccount: payerAta,
        employee: ctx.employee.publicKey,
        employeeTokenAccount: employeeAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        platformAuthority: ctx.platformAuthority.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([resolveIx], [ctx.platformAuthority]);

    // Worker gets their share; the platform KEEPS the full commission (routed
    // to the treasury ATA), never refunded to the employer.
    expect(tokenBalance(employeeAta)).toBe(BigInt(share.toString()));
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(TOK_AMT).toString()),
    );
    expect(tokenBalance(ctx.vaultAta)).toBe(0n);
    expect(decodeEscrow(ctx.escrow).status.resolved).toBeDefined();
  });

  test("raise_dispute with too-short deadline is rejected (DisputeWindowTooShort)", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);
    const ix = await program.methods
      .raiseDispute(Buffer.from("too soon"), new BN(now() + 60))
      .accountsPartial({ escrow: ctx.escrow, signer: payer.publicKey })
      .instruction();
    expectFail([ix], [], "DisputeWindowTooShort");
  });

  test("worker may dispute in Funded; worker dispute in PendingRelease is rejected", async () => {
    // Worker disputes a Funded escrow — allowed.
    const a = newSolEscrow();
    await createSolEscrow(a);
    await depositSol(a);
    const raiseA = await program.methods
      .raiseDispute(Buffer.from("not paid"), new BN(now() + 4 * DAY))
      .accountsPartial({ escrow: a.escrow, signer: a.employee.publicKey })
      .instruction();
    send([raiseA], [a.employee]);
    const escA = decodeEscrow(a.escrow);
    expect(escA.status.disputed).toBeDefined();
    expect(escA.disputeRaisedBy.toBase58()).toBe(
      a.employee.publicKey.toBase58(),
    );

    // Worker disputes in PendingRelease — rejected with DisputeLockedAfterConfirm.
    const b = newSolEscrow();
    await createSolEscrow(b);
    await depositSol(b);
    await confirm(b.escrow, payer); // -> PendingRelease
    const raiseB = await program.methods
      .raiseDispute(Buffer.from("too late"), new BN(now() + 4 * DAY))
      .accountsPartial({ escrow: b.escrow, signer: b.employee.publicKey })
      .instruction();
    expectFail([raiseB], [b.employee], "DisputeLockedAfterConfirm");
  });

  test("trigger_auto_release_sol before the deadline is rejected", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);
    const raiseIx = await program.methods
      .raiseDispute(Buffer.from("dispute"), new BN(now() + 5 * DAY))
      .accountsPartial({ escrow: ctx.escrow, signer: payer.publicKey })
      .instruction();
    send([raiseIx]);

    const triggerIx = await program.methods
      .triggerAutoReleaseSol()
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employee: ctx.employee.publicKey,
        employer: payer.publicKey,
        feeRecipient: treasury.publicKey,
        caller: payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    expectFail([triggerIx], [], "DisputeDeadlineNotReached");
  });

  // ── CLOCK-WARP tests: impossible against a live validator. ──────────────────
  test("trigger_auto_release_sol AFTER deadline pays worker; platform keeps commission", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    await depositSol(ctx);
    const deadline = now() + 5 * DAY;
    const raiseIx = await program.methods
      .raiseDispute(Buffer.from("dispute"), new BN(deadline))
      .accountsPartial({ escrow: ctx.escrow, signer: payer.publicKey })
      .instruction();
    send([raiseIx]);

    // Warp to just past the deadline.
    warpBy(5 * DAY + 60);

    const empBefore = balance(ctx.employee.publicKey);
    const treBefore = balance(treasury.publicKey);
    const triggerIx = await program.methods
      .triggerAutoReleaseSol()
      .accountsPartial({
        escrow: ctx.escrow,
        escrowVault: ctx.vault,
        employee: ctx.employee.publicKey,
        employer: payer.publicKey,
        feeRecipient: treasury.publicKey,
        caller: payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([triggerIx]);

    // Worker gets the full remaining amount; the platform KEEPS the commission
    // (routed to the treasury), never refunded to the employer.
    expect(balance(ctx.employee.publicKey) - empBefore).toBe(
      SOL_AMT.toNumber(),
    );
    expect(balance(treasury.publicKey) - treBefore).toBe(
      commission(SOL_AMT).toNumber(),
    );
    expect(balance(ctx.vault)).toBe(0);
    expect(decodeEscrow(ctx.escrow).status.resolved).toBeDefined();
  });

  test("trigger_auto_release_token AFTER deadline pays worker; platform keeps commission", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);
    const employeeAta = ensureAta(ctx.employee.publicKey);
    const deadline = now() + 5 * DAY;
    const raiseIx = await program.methods
      .raiseDispute(Buffer.from("dispute"), new BN(deadline))
      .accountsPartial({ escrow: ctx.escrow, signer: payer.publicKey })
      .instruction();
    send([raiseIx]);

    warpBy(5 * DAY + 60);

    const empBefore = tokenBalance(employeeAta);
    const employerBefore = tokenBalance(payerAta);
    const treBefore = tokenBalance(treasuryAta);
    const triggerIx = await program.methods
      .triggerAutoReleaseToken()
      .accountsPartial({
        escrow: ctx.escrow,
        tokenMint: mint,
        vaultTokenAccount: ctx.vaultAta,
        employee: ctx.employee.publicKey,
        employeeTokenAccount: employeeAta,
        employer: payer.publicKey,
        employerTokenAccount: payerAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        caller: payer.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    send([triggerIx]);

    // Worker gets the full remaining amount; the platform KEEPS the commission
    // (routed to the treasury ATA); the employer no longer recovers it.
    expect(tokenBalance(employeeAta) - empBefore).toBe(
      BigInt(TOK_AMT.toString()),
    );
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(TOK_AMT).toString()),
    );
    expect(tokenBalance(payerAta) - employerBefore).toBe(0n);
    expect(tokenBalance(ctx.vaultAta)).toBe(0n);
    expect(decodeEscrow(ctx.escrow).status.resolved).toBeDefined();
  });

  test("trigger_auto_release_token before the deadline is rejected", async () => {
    const ctx = newTokenEscrow();
    await createTokenEscrow(ctx);
    await depositToken(ctx);
    const employeeAta = ensureAta(ctx.employee.publicKey);
    const raiseIx = await program.methods
      .raiseDispute(Buffer.from("dispute"), new BN(now() + 5 * DAY))
      .accountsPartial({ escrow: ctx.escrow, signer: payer.publicKey })
      .instruction();
    send([raiseIx]);

    const triggerIx = await program.methods
      .triggerAutoReleaseToken()
      .accountsPartial({
        escrow: ctx.escrow,
        tokenMint: mint,
        vaultTokenAccount: ctx.vaultAta,
        employee: ctx.employee.publicKey,
        employeeTokenAccount: employeeAta,
        employer: payer.publicKey,
        employerTokenAccount: payerAta,
        feeRecipient: treasury.publicKey,
        platformTokenAccount: treasuryAta,
        caller: payer.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    expectFail([triggerIx], [], "DisputeDeadlineNotReached");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Per-escrow platform authority rotation
// ─────────────────────────────────────────────────────────────────────────────
describe("platform authority", () => {
  test("update_platform_authority rotates the per-escrow authority", async () => {
    const ctx = newSolEscrow();
    await createSolEscrow(ctx);
    const newPlatformAuthority = Keypair.generate();
    const ix = await program.methods
      .updatePlatformAuthority()
      .accountsPartial({
        escrow: ctx.escrow,
        currentPlatformAuthority: ctx.platformAuthority.publicKey,
        newPlatformAuthority: newPlatformAuthority.publicKey,
      })
      .instruction();
    send([ix], [ctx.platformAuthority]);
    expect(decodeEscrow(ctx.escrow).platformAuthority.toBase58()).toBe(
      newPlatformAuthority.publicKey.toBase58(),
    );
  });
});

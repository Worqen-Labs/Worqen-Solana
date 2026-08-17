import { beforeAll, describe, expect, test } from "bun:test";
import * as anchor from "@anchor-lang/core";
import {
  AccountLayout,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  createInitializeMint2Instruction,
  createMintToInstruction,
  getAssociatedTokenAddressSync,
  MINT_SIZE,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import {
  Connection,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  Transaction,
  type TransactionInstruction,
} from "@solana/web3.js";
import BN from "bn.js";
import fs from "fs";
import { Clock, LiteSvm } from "litesvm/dist/internal";

const IDL = JSON.parse(
  fs.readFileSync("target/idl/worqen_escrow.json", "utf8"),
);
const PROGRAM_ID = new PublicKey(IDL.address);
const SO_PATH = process.env.WORQEN_SO_PATH ?? "target/deploy/worqen_escrow.so";

const seed = (s: string) => Buffer.from(s);
const rand32 = () => Array.from(Keypair.generate().publicKey.toBytes());

const DAY = 24 * 3600;
const BASE_TS = 1_900_000_000;
const BPS = 500;
const REVIEW = 7 * DAY;
const DURATION = 7 * DAY;
const CAP = new BN(1_000_000);
const MINTED = 1_000_000_000_000;

const commission = (amt: BN, bps = BPS) => amt.muln(bps).divn(10000);
const grossOf = (net: BN, bps = BPS) => net.add(commission(net, bps));
const n = (v: any) => Number(v.toString());

let svm: LiteSvm;
let program: anchor.Program;
let payer: Keypair;
let configPda: PublicKey;
let treasury: Keypair;
let mint: PublicKey;
let treasuryAta: PublicKey;
let platform: Keypair;
let RENT_RESERVE: bigint;

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

function send(ixs: TransactionInstruction[], signers: Keypair[] = []): any {
  const res = svm.sendLegacyTransaction(buildTx(ixs, signers).serialize());
  if (res.constructor.name === "FailedTransactionMetadata") {
    throw new Error("tx failed: " + JSON.stringify((res as any).meta().logs()));
  }
  return res;
}

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

function expectUnsignedFail(ixs: TransactionInstruction[], signers: Keypair[]) {
  svm.expireBlockhash();
  const tx = new Transaction().add(...ixs);
  tx.recentBlockhash = svm.latestBlockhash();
  tx.feePayer = payer.publicKey;
  tx.sign(payer, ...signers);
  const res = svm.sendLegacyTransaction(
    tx.serialize({ requireAllSignatures: false, verifySignatures: false }),
  );
  expect(res.constructor.name).toBe("FailedTransactionMetadata");
}

function accountOf(pk: PublicKey): any {
  return svm.getAccount(pk.toBytes());
}

function exists(pk: PublicKey): boolean {
  const acc = accountOf(pk);
  return acc !== null && acc.lamports() > 0n;
}

function lamports(pk: PublicKey): bigint {
  const acc = accountOf(pk);
  return acc === null ? 0n : acc.lamports();
}

function tokenBalance(ata: PublicKey): bigint {
  const acc = accountOf(ata);
  if (acc === null) return 0n;
  return BigInt(
    AccountLayout.decode(Buffer.from(acc.data())).amount.toString(),
  );
}

function now(): number {
  return Number(svm.getClock().unixTimestamp);
}

function warpTo(ts: number) {
  const c = svm.getClock();
  svm.setClock(
    new Clock(
      c.slot + 1n,
      c.epochStartTimestamp,
      c.epoch,
      c.leaderScheduleEpoch,
      BigInt(ts),
    ),
  );
}

function warpBy(seconds: number) {
  warpTo(now() + seconds);
}

function fundedKeypair(sol = 50): Keypair {
  const kp = Keypair.generate();
  svm.airdrop(kp.publicKey.toBytes(), BigInt(sol * LAMPORTS_PER_SOL));
  return kp;
}

function mintTo(owner: PublicKey, amount = MINTED): PublicKey {
  const ata = getAssociatedTokenAddressSync(mint, owner);
  send([
    createAssociatedTokenAccountInstruction(payer.publicKey, ata, owner, mint),
    createMintToInstruction(mint, ata, payer.publicKey, amount),
  ]);
  return ata;
}

function periodPda(hireId: number[], periodIndex: number): PublicKey {
  const idx = Buffer.alloc(4);
  idx.writeUInt32LE(periodIndex);
  return PublicKey.findProgramAddressSync(
    [seed("hourly"), Buffer.from(hireId), idx],
    PROGRAM_ID,
  )[0];
}

function solVaultPda(period: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [seed("vault"), period.toBuffer()],
    PROGRAM_ID,
  )[0];
}

function invoicePda(period: PublicKey, index: number): PublicKey {
  const idx = Buffer.alloc(2);
  idx.writeUInt16LE(index);
  return PublicKey.findProgramAddressSync(
    [seed("hourly_inv"), period.toBuffer(), idx],
    PROGRAM_ID,
  )[0];
}

function decodeConfig(): any {
  const acc = accountOf(configPda);
  if (acc === null) throw new Error("config account not found");
  return program.coder.accounts.decode("config", Buffer.from(acc.data()));
}

function decodePeriod(period: PublicKey): any {
  const acc = accountOf(period);
  if (acc === null) throw new Error("period account not found");
  return program.coder.accounts.decode("hourlyPeriod", Buffer.from(acc.data()));
}

function decodeInvoice(invoice: PublicKey): any {
  const acc = accountOf(invoice);
  if (acc === null) throw new Error("invoice account not found");
  return program.coder.accounts.decode(
    "hourlyInvoice",
    Buffer.from(acc.data()),
  );
}

function statusOf(v: any): string {
  return Object.keys(v)[0];
}

type Parties = {
  employer: Keypair;
  employerAta: PublicKey;
  employee: Keypair;
  employeeAta: PublicKey;
  platform: Keypair;
};

/**
 * Since v1.5.0 the program never creates a destination token account, so the
 * employee ATA is pre-created here exactly as the backend's
 * `ensure_token_accounts` prepends it. Pass `false` to exercise the
 * missing-destination path.
 */
function newParties(createEmployeeAta = true): Parties {
  const employer = fundedKeypair(500);
  const employee = fundedKeypair(10);
  const employerAta = mintTo(employer.publicKey);
  const employeeAta = createEmployeeAta
    ? mintTo(employee.publicKey, 0)
    : getAssociatedTokenAddressSync(mint, employee.publicKey);
  return { employer, employerAta, employee, employeeAta, platform };
}

/** A fresh 6-decimal mint, for wrong-mint negatives. */
function newMint(): PublicKey {
  const kp = Keypair.generate();
  const rent = svm.minimumBalanceForRentExemption(BigInt(MINT_SIZE));
  send(
    [
      SystemProgram.createAccount({
        fromPubkey: payer.publicKey,
        newAccountPubkey: kp.publicKey,
        space: MINT_SIZE,
        lamports: Number(rent),
        programId: TOKEN_PROGRAM_ID,
      }),
      createInitializeMint2Instruction(kp.publicKey, 6, payer.publicKey, null),
    ],
    [kp],
  );
  return kp.publicKey;
}

/** Create an empty ATA for `owner` on an arbitrary mint. */
function ensureAtaOn(mintPk: PublicKey, ownerPk: PublicKey): PublicKey {
  const ata = getAssociatedTokenAddressSync(mintPk, ownerPk);
  if (accountOf(ata) === null) {
    send([
      createAssociatedTokenAccountInstruction(
        payer.publicKey,
        ata,
        ownerPk,
        mintPk,
      ),
    ]);
  }
  return ata;
}

type Ctx = Parties & {
  hireId: number[];
  periodIndex: number;
  period: PublicKey;
  vault: PublicKey;
  isNative: boolean;
  capNet: BN;
  bps: number;
  review: number;
  duration: number;
  startAt: number;
  endAt: number;
  rentPayer: Keypair;
};

function newCtx(
  o: {
    isNative?: boolean;
    capNet?: BN;
    bps?: number;
    review?: number;
    duration?: number;
    startOffset?: number;
    parties?: Parties;
    periodIndex?: number;
  } = {},
): Ctx {
  const parties = o.parties ?? newParties();
  const hireId = rand32();
  const periodIndex = o.periodIndex ?? 0;
  const period = periodPda(hireId, periodIndex);
  const isNative = o.isNative ?? false;
  const vault = isNative
    ? solVaultPda(period)
    : getAssociatedTokenAddressSync(mint, period, true);
  const startAt = now() + (o.startOffset ?? 0);
  const duration = o.duration ?? DURATION;
  return {
    ...parties,
    hireId,
    periodIndex,
    period,
    vault,
    isNative,
    capNet: o.capNet ?? CAP,
    bps: o.bps ?? BPS,
    review: o.review ?? REVIEW,
    duration,
    startAt,
    endAt: startAt + duration,
    rentPayer: parties.employer,
  };
}

function capGross(ctx: Ctx): BN {
  return grossOf(ctx.capNet, ctx.bps);
}

async function openIx(
  ctx: Ctx,
  o: {
    authorizer?: Keypair;
    employer?: PublicKey;
    employee?: PublicKey;
    platform?: PublicKey;
    platformSigner?: PublicKey;
    feeRecipient?: PublicKey;
    tokenMint?: PublicKey;
    isNative?: boolean;
    capNet?: BN;
    bps?: number;
    review?: number;
    startAt?: number;
    duration?: number;
  } = {},
): Promise<TransactionInstruction> {
  const isNative = o.isNative ?? ctx.isNative;
  return program.methods
    .openPeriod(
      ctx.hireId,
      ctx.periodIndex,
      o.capNet ?? ctx.capNet,
      o.bps ?? ctx.bps,
      new BN(o.review ?? ctx.review),
      new BN(o.startAt ?? ctx.startAt),
      new BN(o.duration ?? ctx.duration),
      isNative,
    )
    .accountsPartial({
      config: configPda,
      hourlyPeriod: ctx.period,
      employer: o.employer ?? ctx.employer.publicKey,
      employee: o.employee ?? ctx.employee.publicKey,
      platformAuthority: o.platform ?? ctx.platform.publicKey,
      feeRecipient: o.feeRecipient ?? treasury.publicKey,
      tokenMint:
        o.tokenMint ?? (isNative ? SystemProgram.programId : (mint as any)),
      authorizer: (o.authorizer ?? ctx.employer).publicKey,
      platformSigner: o.platformSigner ?? ctx.platform.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
}

async function open(ctx: Ctx, authorizer?: Keypair) {
  const signer = authorizer ?? ctx.employer;
  send([await openIx(ctx, { authorizer: signer })], [signer, ctx.platform]);
  ctx.rentPayer = signer;
}

function onchainCapGross(ctx: Ctx): BN {
  const p = decodePeriod(ctx.period);
  return grossOf(p.weeklyCapNet, p.commissionRateBps);
}

async function fundIx(ctx: Ctx, maxFund?: BN): Promise<TransactionInstruction> {
  const bound = maxFund ?? onchainCapGross(ctx);
  if (ctx.isNative) {
    return program.methods
      .fundPeriodSol(bound)
      .accountsPartial({
        config: configPda,
        hourlyPeriod: ctx.period,
        escrowVault: ctx.vault,
        employer: ctx.employer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
  }
  return program.methods
    .fundPeriodToken(bound)
    .accountsPartial({
      config: configPda,
      hourlyPeriod: ctx.period,
      tokenMint: mint,
      vaultTokenAccount: ctx.vault,
      employer: ctx.employer.publicKey,
      employerTokenAccount: ctx.employerAta,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
}

async function fund(ctx: Ctx, maxFund?: BN) {
  send([await fundIx(ctx, maxFund)], [ctx.employer]);
}

async function raiseCapIx(
  ctx: Ctx,
  newCap: BN,
  authority?: Keypair,
): Promise<TransactionInstruction> {
  return program.methods
    .raiseWeeklyCap(newCap)
    .accountsPartial({
      config: configPda,
      hourlyPeriod: ctx.period,
      authority: (authority ?? ctx.employer).publicKey,
    })
    .instruction();
}

function nextInvoiceIndex(ctx: Ctx): number {
  return decodePeriod(ctx.period).invoiceCount;
}

async function stageIx(
  ctx: Ctx,
  amount: BN,
  o: { index?: number; signer?: Keypair; refId?: number[] } = {},
): Promise<TransactionInstruction> {
  const index = o.index ?? nextInvoiceIndex(ctx);
  const signer = o.signer ?? ctx.platform;
  const invoice = invoicePda(ctx.period, index);
  const common = {
    config: configPda,
    hourlyPeriod: ctx.period,
    invoice,
    platformAuthority: signer.publicKey,
    systemProgram: SystemProgram.programId,
  };
  if (ctx.isNative) {
    return program.methods
      .stageInvoiceSol(amount, o.refId ?? rand32())
      .accountsPartial({ ...common, escrowVault: ctx.vault })
      .instruction();
  }
  return program.methods
    .stageInvoiceToken(amount, o.refId ?? rand32())
    .accountsPartial({ ...common, vaultTokenAccount: ctx.vault })
    .instruction();
}

async function stage(ctx: Ctx, amount: BN): Promise<number> {
  const index = nextInvoiceIndex(ctx);
  send([await stageIx(ctx, amount, { index })], [ctx.platform]);
  return index;
}

async function disputeIx(
  ctx: Ctx,
  index: number,
  deadline: number,
  signer: Keypair,
): Promise<TransactionInstruction> {
  return program.methods
    .raiseInvoiceDispute(new BN(deadline), Buffer.from("bad work"))
    .accountsPartial({
      hourlyPeriod: ctx.period,
      invoice: invoicePda(ctx.period, index),
      signer: signer.publicKey,
    })
    .instruction();
}

async function finalizeIx(
  ctx: Ctx,
  index: number,
  o: {
    caller?: Keypair;
    period?: PublicKey;
    vault?: PublicKey;
    employeeTokenAccount?: PublicKey;
    platformTokenAccount?: PublicKey;
  } = {},
): Promise<TransactionInstruction> {
  const caller = o.caller ?? ctx.platform;
  const period = o.period ?? ctx.period;
  const vault = o.vault ?? ctx.vault;
  if (ctx.isNative) {
    return program.methods
      .finalizeInvoiceSol()
      .accountsPartial({
        hourlyPeriod: period,
        invoice: invoicePda(ctx.period, index),
        escrowVault: vault,
        employee: ctx.employee.publicKey,
        feeRecipient: treasury.publicKey,
        rentPayer: ctx.platform.publicKey,
        caller: caller.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
  }
  return program.methods
    .finalizeInvoiceToken()
    .accountsPartial({
      hourlyPeriod: period,
      invoice: invoicePda(ctx.period, index),
      vaultTokenAccount: vault,
      employeeTokenAccount: o.employeeTokenAccount ?? ctx.employeeAta,
      platformTokenAccount: o.platformTokenAccount ?? treasuryAta,
      rentPayer: ctx.platform.publicKey,
      caller: caller.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .instruction();
}

async function approveIx(
  ctx: Ctx,
  index: number,
  o: { approver?: Keypair; period?: PublicKey; vault?: PublicKey } = {},
): Promise<TransactionInstruction> {
  const approver = o.approver ?? ctx.employer;
  const period = o.period ?? ctx.period;
  const vault = o.vault ?? ctx.vault;
  if (ctx.isNative) {
    return program.methods
      .approveInvoiceSol()
      .accountsPartial({
        config: configPda,
        hourlyPeriod: period,
        invoice: invoicePda(ctx.period, index),
        escrowVault: vault,
        employee: ctx.employee.publicKey,
        feeRecipient: treasury.publicKey,
        rentPayer: ctx.platform.publicKey,
        approver: approver.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
  }
  return program.methods
    .approveInvoiceToken()
    .accountsPartial({
      config: configPda,
      hourlyPeriod: period,
      invoice: invoicePda(ctx.period, index),
      vaultTokenAccount: vault,
      employeeTokenAccount: ctx.employeeAta,
      platformTokenAccount: treasuryAta,
      rentPayer: ctx.platform.publicKey,
      approver: approver.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .instruction();
}

async function resolveIx(
  ctx: Ctx,
  index: number,
  share: BN,
  signer?: Keypair,
): Promise<TransactionInstruction> {
  const resolver = signer ?? ctx.platform;
  if (ctx.isNative) {
    return program.methods
      .resolveInvoiceSol(share)
      .accountsPartial({
        hourlyPeriod: ctx.period,
        invoice: invoicePda(ctx.period, index),
        escrowVault: ctx.vault,
        employer: ctx.employer.publicKey,
        employee: ctx.employee.publicKey,
        feeRecipient: treasury.publicKey,
        rentPayer: ctx.platform.publicKey,
        platformAuthority: resolver.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
  }
  return program.methods
    .resolveInvoiceToken(share)
    .accountsPartial({
      hourlyPeriod: ctx.period,
      invoice: invoicePda(ctx.period, index),
      vaultTokenAccount: ctx.vault,
      employerTokenAccount: ctx.employerAta,
      employeeTokenAccount: ctx.employeeAta,
      platformTokenAccount: treasuryAta,
      rentPayer: ctx.platform.publicKey,
      platformAuthority: resolver.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .instruction();
}

async function autoReleaseIx(
  ctx: Ctx,
  index: number,
  caller?: Keypair,
): Promise<TransactionInstruction> {
  const c = caller ?? ctx.platform;
  if (ctx.isNative) {
    return program.methods
      .autoReleaseInvoiceSol()
      .accountsPartial({
        hourlyPeriod: ctx.period,
        invoice: invoicePda(ctx.period, index),
        escrowVault: ctx.vault,
        employee: ctx.employee.publicKey,
        feeRecipient: treasury.publicKey,
        rentPayer: ctx.platform.publicKey,
        caller: c.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
  }
  return program.methods
    .autoReleaseInvoiceToken()
    .accountsPartial({
      hourlyPeriod: ctx.period,
      invoice: invoicePda(ctx.period, index),
      vaultTokenAccount: ctx.vault,
      employeeTokenAccount: ctx.employeeAta,
      platformTokenAccount: treasuryAta,
      rentPayer: ctx.platform.publicKey,
      caller: c.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .instruction();
}

async function settleIx(
  ctx: Ctx,
  caller?: Keypair,
): Promise<TransactionInstruction> {
  const c = caller ?? ctx.platform;
  if (ctx.isNative) {
    return program.methods
      .settlePeriodSol()
      .accountsPartial({
        hourlyPeriod: ctx.period,
        escrowVault: ctx.vault,
        employer: ctx.employer.publicKey,
        caller: c.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
  }
  return program.methods
    .settlePeriodToken()
    .accountsPartial({
      hourlyPeriod: ctx.period,
      tokenMint: mint,
      vaultTokenAccount: ctx.vault,
      employerTokenAccount: ctx.employerAta,
      caller: c.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
}

async function closeIx(
  ctx: Ctx,
  caller?: Keypair,
): Promise<TransactionInstruction> {
  const c = caller ?? ctx.platform;
  if (ctx.isNative) {
    return program.methods
      .closePeriodSol()
      .accountsPartial({
        hourlyPeriod: ctx.period,
        escrowVault: ctx.vault,
        employer: ctx.employer.publicKey,
        rentPayer: ctx.rentPayer.publicKey,
        caller: c.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
  }
  return program.methods
    .closePeriodToken()
    .accountsPartial({
      hourlyPeriod: ctx.period,
      vaultTokenAccount: ctx.vault,
      employer: ctx.employer.publicKey,
      employerTokenAccount: ctx.employerAta,
      rentPayer: ctx.rentPayer.publicKey,
      caller: c.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .instruction();
}

async function setPaused(paused: boolean) {
  const ix = await program.methods
    .updateConfig(null, null, paused, null, null)
    .accountsPartial({ config: configPda, authority: payer.publicKey })
    .instruction();
  send([ix]);
}

async function setPlatformAuthority(pk: PublicKey) {
  const ix = await program.methods
    .updateConfig(null, null, null, null, pk)
    .accountsPartial({ config: configPda, authority: payer.publicKey })
    .instruction();
  send([ix]);
}

async function openFunded(o: Parameters<typeof newCtx>[0] = {}): Promise<Ctx> {
  const ctx = newCtx(o);
  await open(ctx);
  await fund(ctx);
  return ctx;
}

beforeAll(async () => {
  svm = new LiteSvm();
  svm.addProgramFromFile(PROGRAM_ID.toBytes(), SO_PATH);

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
  svm.airdrop(payer.publicKey.toBytes(), BigInt(100_000 * LAMPORTS_PER_SOL));
  treasury = Keypair.generate();
  svm.airdrop(treasury.publicKey.toBytes(), BigInt(LAMPORTS_PER_SOL));
  platform = fundedKeypair(10_000);
  RENT_RESERVE = svm.minimumBalanceForRentExemption(0n);
  expect(RENT_RESERVE).toBe(890_880n);

  const conn = new Connection("http://localhost:8899");
  const wallet = new anchor.Wallet(payer);
  const provider = new anchor.AnchorProvider(conn, wallet, {
    commitment: "processed",
  });
  program = new anchor.Program(IDL as anchor.Idl, provider);
  [configPda] = PublicKey.findProgramAddressSync([seed("config")], PROGRAM_ID);

  const initIx = await program.methods
    .initConfig(treasury.publicKey, BPS, [])
    .accountsPartial({
      config: configPda,
      authority: payer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  send([initIx]);

  const mintKp = Keypair.generate();
  mint = mintKp.publicKey;
  const rent = svm.minimumBalanceForRentExemption(BigInt(MINT_SIZE));
  send(
    [
      SystemProgram.createAccount({
        fromPubkey: payer.publicKey,
        newAccountPubkey: mint,
        space: MINT_SIZE,
        lamports: Number(rent),
        programId: TOKEN_PROGRAM_ID,
      }),
      createInitializeMint2Instruction(mint, 6, payer.publicKey, null),
    ],
    [mintKp],
  );

  treasuryAta = mintTo(treasury.publicKey, 0);

  const addMintIx = await program.methods
    .addAllowedMint(mint)
    .accountsPartial({ config: configPda, authority: payer.publicKey })
    .instruction();
  send([addMintIx]);

  expect(decodeConfig().platformAuthority.toBase58()).toBe(
    PublicKey.default.toBase58(),
  );
  await setPlatformAuthority(platform.publicKey);
  expect(decodeConfig().platformAuthority.toBase58()).toBe(
    platform.publicKey.toBase58(),
  );
});

describe("hourly v2 token: open_period", () => {
  test("employer signer opens a v2 period and becomes rent_payer", async () => {
    const ctx = newCtx();
    await open(ctx);
    const p = decodePeriod(ctx.period);
    expect(p.version).toBe(2);
    expect(statusOf(p.status)).toBe("open");
    expect(p.employer.toBase58()).toBe(ctx.employer.publicKey.toBase58());
    expect(p.employee.toBase58()).toBe(ctx.employee.publicKey.toBase58());
    expect(p.platformAuthority.toBase58()).toBe(
      ctx.platform.publicKey.toBase58(),
    );
    expect(p.feeRecipient.toBase58()).toBe(treasury.publicKey.toBase58());
    expect(p.rentPayer.toBase58()).toBe(ctx.employer.publicKey.toBase58());
    expect(p.tokenMint.toBase58()).toBe(mint.toBase58());
    expect(p.isNative).toBe(false);
    expect(n(p.weeklyCapNet)).toBe(n(CAP));
    expect(p.commissionRateBps).toBe(BPS);
    expect(p.invoiceCount).toBe(0);
    expect(p.liveInvoices).toBe(0);
    expect(n(p.fundedGross)).toBe(0);
    expect(n(p.outstandingNet)).toBe(0);
    expect(n(p.outstandingCommission)).toBe(0);
    expect(n(p.periodEndAt)).toBe(n(p.periodStartAt) + DURATION);
  });

  test("platform authority may open and is recorded as rent_payer", async () => {
    const ctx = newCtx();
    await open(ctx, ctx.platform);
    const p = decodePeriod(ctx.period);
    expect(p.rentPayer.toBase58()).toBe(ctx.platform.publicKey.toBase58());
    expect(p.employer.toBase58()).toBe(ctx.employer.publicKey.toBase58());
  });

  test("a random signer cannot open (Unauthorized)", async () => {
    const ctx = newCtx();
    const stranger = fundedKeypair();
    expectFail(
      [await openIx(ctx, { authorizer: stranger })],
      [stranger, ctx.platform],
      "Unauthorized",
    );
  });

  test("employer == employee is rejected", async () => {
    const ctx = newCtx();
    expectFail(
      [await openIx(ctx, { employee: ctx.employer.publicKey })],
      [ctx.employer, ctx.platform],
      "EmployeeIsEmployer",
    );
  });

  test("commission bps above 1000 is rejected", async () => {
    const ctx = newCtx();
    expectFail(
      [await openIx(ctx, { bps: 1001 })],
      [ctx.employer, ctx.platform],
      "InvalidCommissionRate",
    );
  });

  test("duration shorter than one day is rejected", async () => {
    const ctx = newCtx();
    expectFail(
      [await openIx(ctx, { duration: DAY - 1 })],
      [ctx.employer, ctx.platform],
      "InvalidPeriodWindow",
    );
  });

  test("duration longer than 30 days is rejected", async () => {
    const ctx = newCtx();
    expectFail(
      [await openIx(ctx, { duration: 31 * DAY })],
      [ctx.employer, ctx.platform],
      "InvalidPeriodWindow",
    );
  });

  test("a window that already ended is rejected", async () => {
    const ctx = newCtx();
    expectFail(
      [await openIx(ctx, { startAt: now() - 8 * DAY, duration: 7 * DAY })],
      [ctx.employer, ctx.platform],
      "PeriodEnded",
    );
  });

  test("a fee_recipient that is not the config treasury is rejected", async () => {
    const ctx = newCtx();
    expectFail(
      [await openIx(ctx, { feeRecipient: Keypair.generate().publicKey })],
      [ctx.employer, ctx.platform],
      "InvalidFeeRecipient",
    );
  });

  test("a stranger naming themselves platform_authority is rejected", async () => {
    const ctx = newCtx();
    const stranger = fundedKeypair();
    expectFail(
      [
        await openIx(ctx, {
          authorizer: stranger,
          platform: stranger.publicKey,
          platformSigner: stranger.publicKey,
        }),
      ],
      [stranger],
      "InvalidPlatformAuthority",
    );
    expect(exists(ctx.period)).toBe(false);
  });

  test("the employer may open their own period but cannot name a hostile arbitrator", async () => {
    const ctx = newCtx();
    const hostile = fundedKeypair();
    expectFail(
      [await openIx(ctx, { platform: hostile.publicKey })],
      [ctx.employer, ctx.platform],
      "InvalidPlatformAuthority",
    );
    expect(exists(ctx.period)).toBe(false);

    send(
      [await openIx(ctx, { authorizer: ctx.employer })],
      [ctx.employer, ctx.platform],
    );
    const p = decodePeriod(ctx.period);
    expect(p.platformAuthority.toBase58()).toBe(platform.publicKey.toBase58());
    expect(p.platformAuthority.toBase58()).toBe(
      decodeConfig().platformAuthority.toBase58(),
    );
    expect(p.rentPayer.toBase58()).toBe(ctx.employer.publicKey.toBase58());
  });

  test("a period PDA cannot be squatted without the platform co-signature", async () => {
    const ctx = newCtx();
    const squatter = fundedKeypair();

    expectFail(
      [await openIx(ctx, { platformSigner: squatter.publicKey })],
      [ctx.employer, squatter],
      "InvalidPlatformAuthority",
    );
    expect(exists(ctx.period)).toBe(false);

    expectUnsignedFail([await openIx(ctx)], [ctx.employer]);
    expect(exists(ctx.period)).toBe(false);

    send([await openIx(ctx)], [ctx.employer, ctx.platform]);
    expect(statusOf(decodePeriod(ctx.period).status)).toBe("open");
  });

  test("open is blocked until config.platform_authority is set", async () => {
    const ctx = newCtx();
    await setPlatformAuthority(PublicKey.default);
    expect(decodeConfig().platformAuthority.toBase58()).toBe(
      PublicKey.default.toBase58(),
    );

    expectFail(
      [await openIx(ctx, { platform: platform.publicKey })],
      [ctx.employer, platform],
      "InvalidPlatformAuthority",
    );
    expectFail(
      [await openIx(ctx, { platform: PublicKey.default })],
      [ctx.employer, platform],
      "InvalidPlatformAuthority",
    );
    expect(exists(ctx.period)).toBe(false);

    await setPlatformAuthority(platform.publicKey);
    await open(ctx);
    expect(statusOf(decodePeriod(ctx.period).status)).toBe("open");
    expect(decodePeriod(ctx.period).platformAuthority.toBase58()).toBe(
      platform.publicKey.toBase58(),
    );
  });

  test("open is blocked while the program is paused", async () => {
    const ctx = newCtx();
    await setPaused(true);
    expectFail(
      [await openIx(ctx)],
      [ctx.employer, ctx.platform],
      "ProgramPaused",
    );
    await setPaused(false);
    await open(ctx);
    expect(statusOf(decodePeriod(ctx.period).status)).toBe("open");
  });
});

describe("hourly v2 token: fund_period_token", () => {
  test("funds the vault to cap_gross exactly; a second call is PeriodFullyFunded", async () => {
    const ctx = newCtx();
    await open(ctx);
    const gross = capGross(ctx);
    const employerBefore = tokenBalance(ctx.employerAta);
    expectFail(
      [await fundIx(ctx, gross.subn(1))],
      [ctx.employer],
      "FundExceedsMax",
    );
    expect(tokenBalance(ctx.vault)).toBe(0n);
    await fund(ctx);
    expect(tokenBalance(ctx.vault)).toBe(BigInt(gross.toString()));
    expect(employerBefore - tokenBalance(ctx.employerAta)).toBe(
      BigInt(gross.toString()),
    );
    const p = decodePeriod(ctx.period);
    expect(statusOf(p.status)).toBe("funded");
    expect(n(p.fundedGross)).toBe(n(gross));
    expect(n(p.fundedAt)).toBeGreaterThan(0);
    expectFail([await fundIx(ctx)], [ctx.employer], "PeriodFullyFunded");
  });
});

describe("hourly v2: max_fund_amount bound", () => {
  test("a stale bound protects the employer from a raise_weekly_cap front-run", async () => {
    const ctx = newCtx({ isNative: true, capNet: new BN(1_000_000) });
    await open(ctx);
    const gross = capGross(ctx);

    expectFail(
      [await fundIx(ctx, gross.subn(1))],
      [ctx.employer],
      "FundExceedsMax",
    );
    expect(lamports(ctx.vault)).toBe(0n);

    send([await fundIx(ctx, gross)], [ctx.employer]);
    expect(lamports(ctx.vault)).toBe(BigInt(gross.toString()) + RENT_RESERVE);
    expect(n(decodePeriod(ctx.period).fundedGross)).toBe(n(gross));

    const raised = new BN(100 * LAMPORTS_PER_SOL);
    send([await raiseCapIx(ctx, raised, ctx.platform)], [ctx.platform]);
    const raisedGross = grossOf(raised);
    const toFund = raisedGross.sub(gross);

    const employerBefore = lamports(ctx.employer.publicKey);
    expectFail([await fundIx(ctx, gross)], [ctx.employer], "FundExceedsMax");
    expectFail(
      [await fundIx(ctx, toFund.subn(1))],
      [ctx.employer],
      "FundExceedsMax",
    );
    expect(lamports(ctx.employer.publicKey)).toBe(employerBefore);
    expect(lamports(ctx.vault)).toBe(BigInt(gross.toString()) + RENT_RESERVE);

    send([await fundIx(ctx, toFund)], [ctx.employer]);
    expect(lamports(ctx.vault)).toBe(
      BigInt(raisedGross.toString()) + RENT_RESERVE,
    );
    expect(employerBefore - lamports(ctx.employer.publicKey)).toBe(
      BigInt(toFund.toString()),
    );
    expect(n(decodePeriod(ctx.period).fundedGross)).toBe(n(raisedGross));
    expect(BigInt(n(decodePeriod(ctx.period).vaultRentReserve))).toBe(
      RENT_RESERVE,
    );
  });
});

describe("hourly v2 token: stage_invoice_token", () => {
  test("stages nine invoices with sequential indices, then enforces cap and vault solvency", async () => {
    const ctx = await openFunded();
    const amt = new BN(100_000);
    for (let i = 0; i < 9; i++) {
      const idx = await stage(ctx, amt);
      expect(idx).toBe(i);
      const inv = decodeInvoice(invoicePda(ctx.period, i));
      expect(inv.version).toBe(1);
      expect(inv.invoiceIndex).toBe(i);
      expect(inv.period.toBase58()).toBe(ctx.period.toBase58());
      expect(n(inv.amountNet)).toBe(100_000);
      expect(n(inv.commission)).toBe(5_000);
      expect(statusOf(inv.status)).toBe("staged");
      expect(inv.rentPayer.toBase58()).toBe(ctx.platform.publicKey.toBase58());
      expect(n(inv.releaseAt)).toBe(n(inv.stagedAt) + REVIEW);
      expect(n(inv.disputeDeadline)).toBe(0);
    }
    let p = decodePeriod(ctx.period);
    expect(p.invoiceCount).toBe(9);
    expect(p.liveInvoices).toBe(9);
    expect(statusOf(p.status)).toBe("active");
    expect(n(p.totalStagedNet)).toBe(900_000);
    expect(n(p.outstandingNet)).toBe(900_000);
    expect(n(p.outstandingCommission)).toBe(45_000);

    expectFail(
      [await stageIx(ctx, new BN(200_000))],
      [ctx.platform],
      "WeeklyCapExceeded",
    );

    send([await raiseCapIx(ctx, new BN(2_000_000))], [ctx.employer]);
    expectFail(
      [await stageIx(ctx, new BN(200_000))],
      [ctx.platform],
      "VaultUnderfunded",
    );

    await fund(ctx);
    expect(tokenBalance(ctx.vault)).toBe(2_100_000n);
    const idx = await stage(ctx, new BN(200_000));
    expect(idx).toBe(9);
    p = decodePeriod(ctx.period);
    expect(p.invoiceCount).toBe(10);
    expect(p.liveInvoices).toBe(10);
    expect(n(p.totalStagedNet)).toBe(1_100_000);
    expect(n(p.outstandingCommission)).toBe(55_000);
  });

  test("only the period platform authority may stage", async () => {
    const ctx = await openFunded();
    const stranger = fundedKeypair();
    expectFail(
      [await stageIx(ctx, new BN(100_000), { signer: stranger })],
      [stranger],
      "Unauthorized",
    );
  });

  test("staging after period_end_at is rejected", async () => {
    const ctx = await openFunded();
    warpTo(ctx.endAt + 1);
    expectFail(
      [await stageIx(ctx, new BN(100_000))],
      [ctx.platform],
      "PeriodEnded",
    );
  });

  test("staging before period_start_at is rejected", async () => {
    const ctx = await openFunded({ startOffset: 2 * DAY });
    expectFail(
      [await stageIx(ctx, new BN(100_000))],
      [ctx.platform],
      "PeriodNotStarted",
    );
  });
});

describe("hourly v2: raise_weekly_cap", () => {
  test("the cap is monotonic and may be raised by employer or platform", async () => {
    const ctx = await openFunded();
    expectFail(
      [await raiseCapIx(ctx, ctx.capNet.subn(1))],
      [ctx.employer],
      "CapCannotDecrease",
    );
    send([await raiseCapIx(ctx, new BN(1_500_000))], [ctx.employer]);
    expect(n(decodePeriod(ctx.period).weeklyCapNet)).toBe(1_500_000);
    send(
      [await raiseCapIx(ctx, new BN(1_800_000), ctx.platform)],
      [ctx.platform],
    );
    expect(n(decodePeriod(ctx.period).weeklyCapNet)).toBe(1_800_000);
    const stranger = fundedKeypair();
    expectFail(
      [await raiseCapIx(ctx, new BN(1_900_000), stranger)],
      [stranger],
      "Unauthorized",
    );
  });
});

describe("hourly v2 token: raise_invoice_dispute", () => {
  test("both employer and employee can dispute their own invoices", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    const i1 = await stage(ctx, new BN(200_000));
    send(
      [await disputeIx(ctx, i0, now() + 5 * DAY, ctx.employer)],
      [ctx.employer],
    );
    send(
      [await disputeIx(ctx, i1, now() + 5 * DAY, ctx.employee)],
      [ctx.employee],
    );
    const a = decodeInvoice(invoicePda(ctx.period, i0));
    const b = decodeInvoice(invoicePda(ctx.period, i1));
    expect(statusOf(a.status)).toBe("disputed");
    expect(statusOf(b.status)).toBe("disputed");
    expect(a.disputedBy.toBase58()).toBe(ctx.employer.publicKey.toBase58());
    expect(b.disputedBy.toBase58()).toBe(ctx.employee.publicKey.toBase58());
    const p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(2);
    expect(n(p.outstandingNet)).toBe(400_000);
  });

  test("an unrelated key cannot dispute", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    const stranger = fundedKeypair();
    expectFail(
      [await disputeIx(ctx, i0, now() + 5 * DAY, stranger)],
      [stranger],
      "Unauthorized",
    );
  });

  test("disputing after release_at is rejected", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    warpBy(REVIEW + 1);
    expectFail(
      [await disputeIx(ctx, i0, now() + 5 * DAY, ctx.employer)],
      [ctx.employer],
      "DisputeWindowClosed",
    );
  });

  test("dispute deadlines outside [3d, 90d] are rejected", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    expectFail(
      [await disputeIx(ctx, i0, now() + 2 * DAY, ctx.employer)],
      [ctx.employer],
      "DisputeWindowTooShort",
    );
    expectFail(
      [await disputeIx(ctx, i0, now() + 91 * DAY, ctx.employer)],
      [ctx.employer],
      "DisputeDeadlineTooLong",
    );
    send(
      [await disputeIx(ctx, i0, now() + 3 * DAY, ctx.employer)],
      [ctx.employer],
    );
    expect(statusOf(decodeInvoice(invoicePda(ctx.period, i0)).status)).toBe(
      "disputed",
    );
  });
});

describe("hourly v2 token: finalize_invoice_token", () => {
  test("a random caller finalizes after release_at with exact money motion and rent refund", async () => {
    const ctx = await openFunded();
    const a0 = new BN(400_000);
    const a1 = new BN(300_000);
    const i0 = await stage(ctx, a0);
    const i1 = await stage(ctx, a1);

    expectFail(
      [await finalizeIx(ctx, i0)],
      [ctx.platform],
      "InvoiceWindowNotElapsed",
    );

    const invoiceAcc = invoicePda(ctx.period, i0);
    const invoiceRent = lamports(invoiceAcc);
    expect(invoiceRent).toBeGreaterThan(0n);
    const stranger = fundedKeypair();
    const employeeBefore = tokenBalance(ctx.employeeAta);
    const treasuryBefore = tokenBalance(treasuryAta);
    const vaultBefore = tokenBalance(ctx.vault);
    const platformBefore = lamports(ctx.platform.publicKey);

    warpBy(REVIEW + 1);
    send([await finalizeIx(ctx, i0, { caller: stranger })], [stranger]);

    expect(tokenBalance(ctx.employeeAta) - employeeBefore).toBe(
      BigInt(a0.toString()),
    );
    expect(tokenBalance(treasuryAta) - treasuryBefore).toBe(
      BigInt(commission(a0).toString()),
    );
    expect(vaultBefore - tokenBalance(ctx.vault)).toBe(
      BigInt(grossOf(a0).toString()),
    );
    expect(exists(invoiceAcc)).toBe(false);
    expect(lamports(ctx.platform.publicKey) - platformBefore).toBe(invoiceRent);

    const p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(1);
    expect(n(p.releasedNet)).toBe(n(a0));
    expect(n(p.outstandingNet)).toBe(n(a1));
    expect(n(p.outstandingCommission)).toBe(n(commission(a1)));
    expect(statusOf(decodeInvoice(invoicePda(ctx.period, i1)).status)).toBe(
      "staged",
    );
  });

  test("a disputed invoice cannot be finalized", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    send(
      [await disputeIx(ctx, i0, now() + 3 * DAY, ctx.employer)],
      [ctx.employer],
    );
    warpBy(REVIEW + 1);
    expectFail([await finalizeIx(ctx, i0)], [ctx.platform], "InvoiceNotStaged");
  });
});

describe("hourly v2 token: approve_invoice_token", () => {
  test("the employer approves before release_at with exact money motion and rent refund", async () => {
    const ctx = await openFunded();
    const a0 = new BN(400_000);
    const a1 = new BN(300_000);
    const i0 = await stage(ctx, a0);
    const i1 = await stage(ctx, a1);

    const invoiceAcc = invoicePda(ctx.period, i0);
    const invoiceRent = lamports(invoiceAcc);
    expect(invoiceRent).toBeGreaterThan(0n);
    expect(now()).toBeLessThan(n(decodeInvoice(invoiceAcc).releaseAt));

    const employeeBefore = tokenBalance(ctx.employeeAta);
    const treasuryBefore = tokenBalance(treasuryAta);
    const vaultBefore = tokenBalance(ctx.vault);
    const platformBefore = lamports(ctx.platform.publicKey);

    send([await approveIx(ctx, i0)], [ctx.employer]);

    expect(tokenBalance(ctx.employeeAta) - employeeBefore).toBe(
      BigInt(a0.toString()),
    );
    expect(tokenBalance(treasuryAta) - treasuryBefore).toBe(
      BigInt(commission(a0).toString()),
    );
    expect(vaultBefore - tokenBalance(ctx.vault)).toBe(
      BigInt(grossOf(a0).toString()),
    );
    expect(exists(invoiceAcc)).toBe(false);
    expect(lamports(ctx.platform.publicKey) - platformBefore).toBe(invoiceRent);

    const p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(1);
    expect(n(p.releasedNet)).toBe(n(a0));
    expect(n(p.outstandingNet)).toBe(n(a1));
    expect(n(p.outstandingCommission)).toBe(n(commission(a1)));
    expect(statusOf(decodeInvoice(invoicePda(ctx.period, i1)).status)).toBe(
      "staged",
    );
  });

  test("the platform authority may approve on the employer's behalf", async () => {
    const ctx = await openFunded();
    const amt = new BN(250_000);
    const i0 = await stage(ctx, amt);
    const employeeBefore = tokenBalance(ctx.employeeAta);
    const treasuryBefore = tokenBalance(treasuryAta);

    send(
      [await approveIx(ctx, i0, { approver: ctx.platform })],
      [ctx.platform],
    );

    expect(tokenBalance(ctx.employeeAta) - employeeBefore).toBe(
      BigInt(amt.toString()),
    );
    expect(tokenBalance(treasuryAta) - treasuryBefore).toBe(
      BigInt(commission(amt).toString()),
    );
    expect(exists(invoicePda(ctx.period, i0))).toBe(false);
    const p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(0);
    expect(n(p.releasedNet)).toBe(n(amt));
  });

  test("neither the employee nor a stranger can approve", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    const stranger = fundedKeypair();

    expectFail(
      [await approveIx(ctx, i0, { approver: ctx.employee })],
      [ctx.employee],
      "Unauthorized",
    );
    expectFail(
      [await approveIx(ctx, i0, { approver: stranger })],
      [stranger],
      "Unauthorized",
    );
    expect(statusOf(decodeInvoice(invoicePda(ctx.period, i0)).status)).toBe(
      "staged",
    );
  });

  test("a disputed invoice cannot be approved", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    send(
      [await disputeIx(ctx, i0, now() + 3 * DAY, ctx.employee)],
      [ctx.employee],
    );
    expectFail([await approveIx(ctx, i0)], [ctx.employer], "InvoiceNotStaged");
  });

  test("approving the same invoice twice fails on the closed account", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    send([await approveIx(ctx, i0)], [ctx.employer]);
    expectFail(
      [await approveIx(ctx, i0)],
      [ctx.employer],
      "AccountNotInitialized",
    );
  });

  test("approve still works after the review window has elapsed", async () => {
    const ctx = await openFunded();
    const amt = new BN(200_000);
    const i0 = await stage(ctx, amt);
    warpBy(REVIEW + 1);
    const employeeBefore = tokenBalance(ctx.employeeAta);
    send([await approveIx(ctx, i0)], [ctx.employer]);
    expect(tokenBalance(ctx.employeeAta) - employeeBefore).toBe(
      BigInt(amt.toString()),
    );
  });

  test("the pause switch blocks approve for both signers without trapping the invoice", async () => {
    const ctx = await openFunded();
    const amt = new BN(200_000);
    const i0 = await stage(ctx, amt);
    const i1 = await stage(ctx, amt);

    await setPaused(true);
    expectFail([await approveIx(ctx, i0)], [ctx.employer], "ProgramPaused");
    expectFail(
      [await approveIx(ctx, i0, { approver: ctx.platform })],
      [ctx.platform],
      "ProgramPaused",
    );
    expect(statusOf(decodeInvoice(invoicePda(ctx.period, i0)).status)).toBe(
      "staged",
    );

    warpBy(REVIEW + 1);
    const finalizedBefore = tokenBalance(ctx.employeeAta);
    send([await finalizeIx(ctx, i0)], [ctx.platform]);
    await setPaused(false);

    expect(tokenBalance(ctx.employeeAta) - finalizedBefore).toBe(
      BigInt(amt.toString()),
    );
    expect(exists(invoicePda(ctx.period, i0))).toBe(false);

    const approvedBefore = tokenBalance(ctx.employeeAta);
    send([await approveIx(ctx, i1)], [ctx.employer]);
    expect(tokenBalance(ctx.employeeAta) - approvedBefore).toBe(
      BigInt(amt.toString()),
    );
    expect(exists(invoicePda(ctx.period, i1))).toBe(false);
  });

  test("an invoice of period A cannot be approved against period B", async () => {
    const parties = newParties();
    const a = await openFunded({ parties });
    const b = await openFunded({ parties });
    const i0 = await stage(a, new BN(200_000));
    expectFail(
      [await approveIx(a, i0, { period: b.period, vault: b.vault })],
      [a.employer],
      "InvoicePeriodMismatch",
    );
  });

  test("approving one invoice leaves the employer settle refund unchanged", async () => {
    const amounts = [new BN(400_000), new BN(300_000)];

    const plain = await openFunded();
    const p0 = await stage(plain, amounts[0]);
    const p1 = await stage(plain, amounts[1]);
    expect(p1).toBe(p0 + 1);
    warpTo(plain.endAt + 1);
    const plainEmployerBefore = tokenBalance(plain.employerAta);
    send([await settleIx(plain)], [plain.platform]);
    const plainRefund = tokenBalance(plain.employerAta) - plainEmployerBefore;

    const approved = await openFunded();
    const a0 = await stage(approved, amounts[0]);
    await stage(approved, amounts[1]);
    send([await approveIx(approved, a0)], [approved.employer]);
    warpTo(approved.endAt + 1);
    const approvedEmployerBefore = tokenBalance(approved.employerAta);
    send([await settleIx(approved)], [approved.platform]);
    const approvedRefund =
      tokenBalance(approved.employerAta) - approvedEmployerBefore;

    expect(approvedRefund).toBe(plainRefund);
    expect(plainRefund).toBe(
      BigInt(
        capGross(plain)
          .sub(grossOf(amounts[0]))
          .sub(grossOf(amounts[1]))
          .toString(),
      ),
    );
  });
});

describe("hourly v2 token: resolve_invoice_token", () => {
  test("resolve splits net + commission exactly across employee, treasury and employer", async () => {
    const ctx = await openFunded();
    const amt = new BN(400_000);
    const i0 = await stage(ctx, amt);
    send(
      [await disputeIx(ctx, i0, now() + 5 * DAY, ctx.employee)],
      [ctx.employee],
    );

    const share = new BN(250_000);
    const invCommission = commission(amt);
    const treasuryLeg = commission(share);
    const employerLeg = amt.sub(share).add(invCommission.sub(treasuryLeg));

    const employeeBefore = tokenBalance(ctx.employeeAta);
    const treasuryBefore = tokenBalance(treasuryAta);
    const employerBefore = tokenBalance(ctx.employerAta);
    const vaultBefore = tokenBalance(ctx.vault);
    const invoiceAcc = invoicePda(ctx.period, i0);
    const invoiceRent = lamports(invoiceAcc);
    const platformBefore = lamports(ctx.platform.publicKey);

    send([await resolveIx(ctx, i0, share)], [ctx.platform]);

    const employeeDelta = tokenBalance(ctx.employeeAta) - employeeBefore;
    const treasuryDelta = tokenBalance(treasuryAta) - treasuryBefore;
    const employerDelta = tokenBalance(ctx.employerAta) - employerBefore;
    expect(employeeDelta).toBe(BigInt(share.toString()));
    expect(treasuryDelta).toBe(BigInt(treasuryLeg.toString()));
    expect(employerDelta).toBe(BigInt(employerLeg.toString()));
    expect(employeeDelta + treasuryDelta + employerDelta).toBe(
      BigInt(grossOf(amt).toString()),
    );
    expect(vaultBefore - tokenBalance(ctx.vault)).toBe(
      BigInt(grossOf(amt).toString()),
    );
    expect(exists(invoiceAcc)).toBe(false);
    expect(lamports(ctx.platform.publicKey) - platformBefore).toBeGreaterThan(
      0n,
    );
    expect(invoiceRent).toBeGreaterThan(0n);

    const p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(0);
    expect(n(p.releasedNet)).toBe(n(share));
    expect(n(p.outstandingNet)).toBe(0);
    expect(n(p.outstandingCommission)).toBe(0);
  });

  test("employee_share above the invoice amount is rejected", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    send(
      [await disputeIx(ctx, i0, now() + 5 * DAY, ctx.employer)],
      [ctx.employer],
    );
    expectFail(
      [await resolveIx(ctx, i0, new BN(200_001))],
      [ctx.platform],
      "EmployeeShareExceedsInvoice",
    );
  });

  test("only the platform authority may resolve", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    send(
      [await disputeIx(ctx, i0, now() + 5 * DAY, ctx.employer)],
      [ctx.employer],
    );
    const stranger = fundedKeypair();
    expectFail(
      [await resolveIx(ctx, i0, new BN(100_000), stranger)],
      [stranger],
      "Unauthorized",
    );
  });
});

describe("hourly v2 token: auto_release_invoice_token", () => {
  test("permissionless auto-release only after the long-stop deadline", async () => {
    const ctx = await openFunded();
    const amt = new BN(300_000);
    const i0 = await stage(ctx, amt);
    const deadline = now() + 3 * DAY;
    send([await disputeIx(ctx, i0, deadline, ctx.employee)], [ctx.employee]);

    expectFail(
      [await autoReleaseIx(ctx, i0)],
      [ctx.platform],
      "DisputeDeadlineNotReached",
    );

    const stranger = fundedKeypair();
    const employeeBefore = tokenBalance(ctx.employeeAta);
    const treasuryBefore = tokenBalance(treasuryAta);
    warpTo(deadline + 1);
    send([await autoReleaseIx(ctx, i0, stranger)], [stranger]);

    expect(tokenBalance(ctx.employeeAta) - employeeBefore).toBe(
      BigInt(amt.toString()),
    );
    expect(tokenBalance(treasuryAta) - treasuryBefore).toBe(
      BigInt(commission(amt).toString()),
    );
    expect(exists(invoicePda(ctx.period, i0))).toBe(false);
    const p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(0);
    expect(n(p.releasedNet)).toBe(n(amt));
  });
});

describe("hourly v2 token: settle_period_token + close_period_token", () => {
  test("settle refunds vault minus outstanding, then close reclaims rents after every invoice clears", async () => {
    const ctx = newCtx({ review: 2 * DAY });
    await open(ctx, ctx.platform);
    await fund(ctx);

    const a0 = new BN(300_000);
    const a1 = new BN(200_000);
    const i0 = await stage(ctx, a0);
    const i1 = await stage(ctx, a1);
    send(
      [await disputeIx(ctx, i0, now() + 5 * DAY, ctx.employer)],
      [ctx.employer],
    );

    expectFail([await settleIx(ctx)], [ctx.platform], "PeriodNotEnded");

    warpTo(ctx.endAt + 1);
    const outstanding = grossOf(a0).add(grossOf(a1));
    const vaultBefore = tokenBalance(ctx.vault);
    const employerBefore = tokenBalance(ctx.employerAta);
    send([await settleIx(ctx)], [ctx.platform]);

    const refunded = vaultBefore - BigInt(outstanding.toString());
    expect(refunded).toBeGreaterThan(0n);
    expect(tokenBalance(ctx.employerAta) - employerBefore).toBe(refunded);
    expect(tokenBalance(ctx.vault)).toBe(BigInt(outstanding.toString()));
    let p = decodePeriod(ctx.period);
    expect(statusOf(p.status)).toBe("settled");
    expect(n(p.refundedAmount)).toBe(Number(refunded));
    expect(p.liveInvoices).toBe(2);
    expect(statusOf(decodeInvoice(invoicePda(ctx.period, i0)).status)).toBe(
      "disputed",
    );
    expect(statusOf(decodeInvoice(invoicePda(ctx.period, i1)).status)).toBe(
      "staged",
    );

    expectFail([await settleIx(ctx)], [ctx.platform], "PeriodAlreadySettled");
    expectFail(
      [await stageIx(ctx, new BN(10_000))],
      [ctx.platform],
      "InvalidStatus",
    );
    expectFail([await closeIx(ctx)], [ctx.platform], "LiveInvoicesOutstanding");

    send([await resolveIx(ctx, i0, a0)], [ctx.platform]);
    send([await finalizeIx(ctx, i1)], [ctx.platform]);
    expect(tokenBalance(ctx.vault)).toBe(0n);

    const dust = 777n;
    send([
      createMintToInstruction(mint, ctx.vault, payer.publicKey, Number(dust)),
    ]);

    p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(0);
    const periodRent = lamports(ctx.period);
    const vaultRent = lamports(ctx.vault);
    expect(periodRent).toBeGreaterThan(0n);
    expect(vaultRent).toBeGreaterThan(0n);
    const rentPayerBefore = lamports(ctx.rentPayer.publicKey);
    const employerLamportsBefore = lamports(ctx.employer.publicKey);
    const employerTokensBefore = tokenBalance(ctx.employerAta);

    send([await closeIx(ctx)], [ctx.platform]);

    expect(exists(ctx.vault)).toBe(false);
    expect(exists(ctx.period)).toBe(false);
    expect(tokenBalance(ctx.employerAta) - employerTokensBefore).toBe(dust);
    expect(lamports(ctx.employer.publicKey) - employerLamportsBefore).toBe(
      vaultRent,
    );
    expect(lamports(ctx.rentPayer.publicKey) - rentPayerBefore).toBe(
      periodRent,
    );
  });
});

describe("hourly v2 token: caller-owned destination token accounts", () => {
  test("a wrong-owner employee token account is rejected (Unauthorized)", async () => {
    const ctx = await openFunded();
    const amt = new BN(200_000);
    const i0 = await stage(ctx, amt);
    warpBy(REVIEW + 1);

    const strangerAta = mintTo(fundedKeypair().publicKey, 0);
    const vaultBefore = tokenBalance(ctx.vault);
    expectFail(
      [await finalizeIx(ctx, i0, { employeeTokenAccount: strangerAta })],
      [ctx.platform],
      "Unauthorized",
    );
    expect(tokenBalance(strangerAta)).toBe(0n);
    expect(tokenBalance(ctx.vault)).toBe(vaultBefore);
    expect(exists(invoicePda(ctx.period, i0))).toBe(true);
  });

  test("a wrong-mint employee token account is rejected (InvalidTokenMint)", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    warpBy(REVIEW + 1);

    const wrongMintAta = ensureAtaOn(newMint(), ctx.employee.publicKey);
    expectFail(
      [await finalizeIx(ctx, i0, { employeeTokenAccount: wrongMintAta })],
      [ctx.platform],
      "InvalidTokenMint",
    );
    expect(exists(invoicePda(ctx.period, i0))).toBe(true);
  });

  test("a wrong-owner treasury token account is rejected (Unauthorized)", async () => {
    const ctx = await openFunded();
    const i0 = await stage(ctx, new BN(200_000));
    warpBy(REVIEW + 1);

    const fakeTreasuryAta = mintTo(fundedKeypair().publicKey, 0);
    expectFail(
      [await finalizeIx(ctx, i0, { platformTokenAccount: fakeTreasuryAta })],
      [ctx.platform],
      "Unauthorized",
    );
    expect(tokenBalance(fakeTreasuryAta)).toBe(0n);
    expect(exists(invoicePda(ctx.period, i0))).toBe(true);
  });

  test("a missing employee token account fails cleanly (AccountNotInitialized)", async () => {
    const ctx = await openFunded({ parties: newParties(false) });
    const amt = new BN(200_000);
    const i0 = await stage(ctx, amt);
    warpBy(REVIEW + 1);
    expect(accountOf(ctx.employeeAta)).toBeNull();

    expectFail(
      [await finalizeIx(ctx, i0)],
      [ctx.platform],
      "AccountNotInitialized",
    );
    expect(exists(invoicePda(ctx.period, i0))).toBe(true);

    // The caller creates it and the same instruction now settles.
    send(
      [
        createAssociatedTokenAccountInstruction(
          payer.publicKey,
          ctx.employeeAta,
          ctx.employee.publicKey,
          mint,
        ),
        await finalizeIx(ctx, i0),
      ],
      [ctx.platform],
    );
    expect(tokenBalance(ctx.employeeAta)).toBe(BigInt(amt.toString()));
    expect(exists(invoicePda(ctx.period, i0))).toBe(false);
  });
});

describe("hourly v2: marginal commission rounding", () => {
  test("per-invoice commissions round cumulatively and sum to commission(total staged)", async () => {
    const capNet = new BN(666_666);
    const ctx = await openFunded({ capNet });
    expect(tokenBalance(ctx.vault)).toBe(699_999n);

    const amt = new BN(333_333);
    const i0 = await stage(ctx, amt);
    const i1 = await stage(ctx, amt);
    const c0 = n(decodeInvoice(invoicePda(ctx.period, i0)).commission);
    const c1 = n(decodeInvoice(invoicePda(ctx.period, i1)).commission);
    expect(c0).toBe(16_666);
    expect(c1).toBe(16_667);
    expect(c0 + c1).toBe(n(commission(capNet)));

    const p = decodePeriod(ctx.period);
    expect(n(p.totalStagedNet)).toBe(666_666);
    expect(n(p.outstandingCommission)).toBe(33_333);
    expect(n(p.outstandingNet) + n(p.outstandingCommission)).toBe(699_999);

    const treasuryBefore = tokenBalance(treasuryAta);
    warpBy(REVIEW + 1);
    send([await finalizeIx(ctx, i0)], [ctx.platform]);
    send([await finalizeIx(ctx, i1)], [ctx.platform]);
    expect(tokenBalance(treasuryAta) - treasuryBefore).toBe(
      BigInt(commission(capNet).toString()),
    );
    expect(tokenBalance(ctx.vault)).toBe(0n);
  });
});

describe("hourly v2 native SOL lifecycle", () => {
  test("fund, stage, dispute+resolve, finalize, settle and close move lamports exactly", async () => {
    const capNet = new BN(2 * LAMPORTS_PER_SOL);
    const ctx = newCtx({ isNative: true, capNet, review: 3 * DAY });
    await open(ctx, ctx.platform);

    let p = decodePeriod(ctx.period);
    expect(p.isNative).toBe(true);
    expect(p.tokenMint.toBase58()).toBe(SystemProgram.programId.toBase58());

    const gross = capGross(ctx);
    const employerBeforeFund = lamports(ctx.employer.publicKey);
    await fund(ctx);
    expect(lamports(ctx.vault)).toBe(BigInt(gross.toString()) + RENT_RESERVE);
    expect(employerBeforeFund - lamports(ctx.employer.publicKey)).toBe(
      BigInt(gross.toString()) + RENT_RESERVE,
    );
    expect(BigInt(n(decodePeriod(ctx.period).vaultRentReserve))).toBe(
      RENT_RESERVE,
    );

    const a0 = new BN(500_000_000);
    const a1 = new BN(300_000_000);
    const i0 = await stage(ctx, a0);
    const i1 = await stage(ctx, a1);
    expect(n(decodeInvoice(invoicePda(ctx.period, i0)).commission)).toBe(
      n(commission(a0)),
    );
    expect(n(decodeInvoice(invoicePda(ctx.period, i1)).commission)).toBe(
      n(commission(a1)),
    );

    send(
      [await disputeIx(ctx, i0, now() + 5 * DAY, ctx.employee)],
      [ctx.employee],
    );

    const share = new BN(200_000_000);
    const treasuryLeg = commission(share);
    const employerLeg = a0.sub(share).add(commission(a0).sub(treasuryLeg));
    const employeeBefore = lamports(ctx.employee.publicKey);
    const treasuryBefore = lamports(treasury.publicKey);
    const employerBefore = lamports(ctx.employer.publicKey);
    const vaultBefore = lamports(ctx.vault);

    send([await resolveIx(ctx, i0, share)], [ctx.platform]);

    expect(lamports(ctx.employee.publicKey) - employeeBefore).toBe(
      BigInt(share.toString()),
    );
    expect(lamports(treasury.publicKey) - treasuryBefore).toBe(
      BigInt(treasuryLeg.toString()),
    );
    expect(lamports(ctx.employer.publicKey) - employerBefore).toBe(
      BigInt(employerLeg.toString()),
    );
    expect(vaultBefore - lamports(ctx.vault)).toBe(
      BigInt(grossOf(a0).toString()),
    );

    const stranger = fundedKeypair();
    const employeeBefore2 = lamports(ctx.employee.publicKey);
    const treasuryBefore2 = lamports(treasury.publicKey);
    warpBy(3 * DAY + 1);
    send([await finalizeIx(ctx, i1, { caller: stranger })], [stranger]);
    expect(lamports(ctx.employee.publicKey) - employeeBefore2).toBe(
      BigInt(a1.toString()),
    );
    expect(lamports(treasury.publicKey) - treasuryBefore2).toBe(
      BigInt(commission(a1).toString()),
    );

    expectFail([await settleIx(ctx)], [ctx.platform], "PeriodNotEnded");

    warpTo(ctx.endAt + 1);
    const vaultAtSettle = lamports(ctx.vault);
    const employerBefore3 = lamports(ctx.employer.publicKey);
    send([await settleIx(ctx)], [ctx.platform]);
    expect(lamports(ctx.employer.publicKey) - employerBefore3).toBe(
      vaultAtSettle - RENT_RESERVE,
    );
    expect(lamports(ctx.vault)).toBe(RENT_RESERVE);

    p = decodePeriod(ctx.period);
    expect(statusOf(p.status)).toBe("settled");
    expect(p.liveInvoices).toBe(0);
    expect(n(p.releasedNet)).toBe(n(share.add(a1)));

    const residue = 1_000_000n;
    svm.airdrop(ctx.vault.toBytes(), residue);
    expect(lamports(ctx.vault)).toBe(residue + RENT_RESERVE);
    const periodRent = lamports(ctx.period);
    expect(periodRent).toBeGreaterThan(0n);
    const employerBefore4 = lamports(ctx.employer.publicKey);
    const rentPayerBefore = lamports(ctx.rentPayer.publicKey);

    send([await closeIx(ctx)], [ctx.platform]);

    expect(lamports(ctx.vault)).toBe(0n);
    expect(exists(ctx.period)).toBe(false);
    expect(lamports(ctx.employer.publicKey) - employerBefore4).toBe(
      residue + RENT_RESERVE,
    );
    expect(lamports(ctx.rentPayer.publicKey) - rentPayerBefore).toBe(
      periodRent,
    );
  });

  test("the employer approves a native invoice immediately and the vault keeps its rent reserve", async () => {
    const capNet = new BN(2 * LAMPORTS_PER_SOL);
    const ctx = newCtx({ isNative: true, capNet, review: 3 * DAY });
    await open(ctx, ctx.platform);
    await fund(ctx);

    const a0 = new BN(500_000_000);
    const a1 = new BN(300_000_000);
    const i0 = await stage(ctx, a0);
    const i1 = await stage(ctx, a1);

    const invoiceAcc = invoicePda(ctx.period, i0);
    const invoiceRent = lamports(invoiceAcc);
    expect(now()).toBeLessThan(n(decodeInvoice(invoiceAcc).releaseAt));

    const employeeBefore = lamports(ctx.employee.publicKey);
    const treasuryBefore = lamports(treasury.publicKey);
    const vaultBefore = lamports(ctx.vault);
    const platformBefore = lamports(ctx.platform.publicKey);

    send([await approveIx(ctx, i0)], [ctx.employer]);

    expect(lamports(ctx.employee.publicKey) - employeeBefore).toBe(
      BigInt(a0.toString()),
    );
    expect(lamports(treasury.publicKey) - treasuryBefore).toBe(
      BigInt(commission(a0).toString()),
    );
    expect(vaultBefore - lamports(ctx.vault)).toBe(
      BigInt(grossOf(a0).toString()),
    );
    expect(lamports(ctx.platform.publicKey) - platformBefore).toBe(invoiceRent);
    expect(exists(invoiceAcc)).toBe(false);
    expect(lamports(ctx.vault)).toBeGreaterThanOrEqual(
      RENT_RESERVE + BigInt(grossOf(a1).toString()),
    );

    const p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(1);
    expect(n(p.releasedNet)).toBe(n(a0));
    expect(n(p.outstandingNet)).toBe(n(a1));
    expect(n(p.outstandingCommission)).toBe(n(commission(a1)));
    expect(statusOf(decodeInvoice(invoicePda(ctx.period, i1)).status)).toBe(
      "staged",
    );
  });
});

describe("hourly v2: cross-period isolation", () => {
  test("an invoice of period A cannot be finalized against period B", async () => {
    const parties = newParties();
    const a = await openFunded({ parties });
    const b = await openFunded({ parties });
    const i0 = await stage(a, new BN(200_000));
    warpBy(REVIEW + 1);
    expectFail(
      [await finalizeIx(a, i0, { period: b.period, vault: b.vault })],
      [a.platform],
      "InvoicePeriodMismatch",
    );
  });
});

describe("hourly v2: pause invariants", () => {
  test("pause blocks open/fund/stage/raise_cap but never blocks the payout path", async () => {
    const ctx = newCtx({ duration: DAY, review: DAY / 2 });
    await open(ctx, ctx.platform);
    await fund(ctx);
    const amt = new BN(200_000);
    const i0 = await stage(ctx, amt);
    const i1 = await stage(ctx, amt);
    const i2 = await stage(ctx, amt);

    const unfunded = newCtx({ parties: ctx });
    await open(unfunded);
    const fresh = newCtx({ parties: ctx });

    await setPaused(true);

    expectFail(
      [await openIx(fresh)],
      [fresh.employer, fresh.platform],
      "ProgramPaused",
    );
    expectFail([await fundIx(unfunded)], [unfunded.employer], "ProgramPaused");
    expectFail([await stageIx(ctx, amt)], [ctx.platform], "ProgramPaused");
    expectFail(
      [await raiseCapIx(ctx, new BN(2_000_000))],
      [ctx.employer],
      "ProgramPaused",
    );

    send(
      [await disputeIx(ctx, i0, now() + 3 * DAY, ctx.employer)],
      [ctx.employer],
    );
    warpBy(DAY / 2 + 1);
    send([await finalizeIx(ctx, i1)], [ctx.platform]);
    send([await resolveIx(ctx, i0, new BN(100_000))], [ctx.platform]);

    warpTo(ctx.endAt + 1);
    send([await settleIx(ctx)], [ctx.platform]);
    send([await finalizeIx(ctx, i2)], [ctx.platform]);
    send([await closeIx(ctx)], [ctx.platform]);

    expect(exists(ctx.period)).toBe(false);
    expect(exists(ctx.vault)).toBe(false);
    expect(decodePeriod(unfunded.period).fundedGross.toNumber()).toBe(0);

    await setPaused(false);
  });
});

describe("hourly v2 native SOL: rent-exemption of the lamport vault", () => {
  test("settle_period_sol keeps the rent reserve and still refunds everything else", async () => {
    const ctx = newCtx({
      isNative: true,
      capNet: new BN(10_000_000),
      review: 2 * DAY,
    });
    await open(ctx, ctx.platform);
    await fund(ctx);
    const i0 = await stage(ctx, new BN(1000));
    const outstanding = grossOf(new BN(1000));
    warpTo(ctx.endAt + 1);
    const employerBefore = lamports(ctx.employer.publicKey);
    send([await settleIx(ctx)], [ctx.platform]);

    expect(lamports(ctx.employer.publicKey) - employerBefore).toBe(
      BigInt(capGross(ctx).sub(outstanding).toString()),
    );
    expect(lamports(ctx.vault)).toBe(
      RENT_RESERVE + BigInt(outstanding.toString()),
    );
    expect(lamports(ctx.vault)).toBeGreaterThanOrEqual(RENT_RESERVE);
    const p = decodePeriod(ctx.period);
    expect(statusOf(p.status)).toBe("settled");
    expect(BigInt(n(p.vaultRentReserve))).toBe(RENT_RESERVE);
    expect(n(p.refundedAmount)).toBe(n(capGross(ctx).sub(outstanding)));
    expect(statusOf(decodeInvoice(invoicePda(ctx.period, i0)).status)).toBe(
      "staged",
    );
  });

  test("finalize_invoice_sol works for every invoice one at a time after settle", async () => {
    const ctx = newCtx({
      isNative: true,
      capNet: new BN(10_000_000),
      review: 2 * DAY,
    });
    await open(ctx, ctx.platform);
    await fund(ctx);
    const amt = new BN(476_190);
    const i0 = await stage(ctx, amt);
    const i1 = await stage(ctx, amt);
    warpTo(ctx.endAt + 1);
    send([await settleIx(ctx)], [ctx.platform]);
    expect(lamports(ctx.vault)).toBe(
      RENT_RESERVE + BigInt(grossOf(amt.muln(2)).toString()),
    );

    const employeeBefore = lamports(ctx.employee.publicKey);
    const treasuryBefore = lamports(treasury.publicKey);
    send([await finalizeIx(ctx, i0)], [ctx.platform]);
    send([await finalizeIx(ctx, i1)], [ctx.platform]);

    expect(lamports(ctx.employee.publicKey) - employeeBefore).toBe(
      BigInt(amt.muln(2).toString()),
    );
    expect(lamports(treasury.publicKey) - treasuryBefore).toBe(
      BigInt(commission(amt.muln(2)).toString()),
    );
    expect(lamports(ctx.vault)).toBe(RENT_RESERVE);
    const p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(0);
    expect(n(p.outstandingNet)).toBe(0);
    expect(n(p.outstandingCommission)).toBe(0);
  });

  test("approve_invoice_sol before settle leaves the vault at the reserve plus the outstanding remainder", async () => {
    const ctx = newCtx({
      isNative: true,
      capNet: new BN(10_000_000),
      review: 2 * DAY,
    });
    await open(ctx, ctx.platform);
    await fund(ctx);
    const a0 = new BN(1_200_000);
    const a1 = new BN(800_000);
    const i0 = await stage(ctx, a0);
    await stage(ctx, a1);

    send([await approveIx(ctx, i0)], [ctx.employer]);
    expect(lamports(ctx.vault)).toBe(
      RENT_RESERVE + BigInt(capGross(ctx).sub(grossOf(a0)).toString()),
    );

    warpTo(ctx.endAt + 1);
    const employerBefore = lamports(ctx.employer.publicKey);
    send([await settleIx(ctx)], [ctx.platform]);

    expect(lamports(ctx.employer.publicKey) - employerBefore).toBe(
      BigInt(capGross(ctx).sub(grossOf(a0)).sub(grossOf(a1)).toString()),
    );
    expect(lamports(ctx.vault)).toBe(
      RENT_RESERVE + BigInt(grossOf(a1).toString()),
    );
    expect(lamports(ctx.vault)).toBeGreaterThanOrEqual(RENT_RESERVE);
    const p = decodePeriod(ctx.period);
    expect(statusOf(p.status)).toBe("settled");
    expect(p.liveInvoices).toBe(1);
    expect(n(p.outstandingNet)).toBe(n(a1));
    expect(n(p.outstandingCommission)).toBe(n(commission(a1)));
  });

  test("approve_invoice_sol works for every invoice one at a time after settle", async () => {
    const ctx = newCtx({
      isNative: true,
      capNet: new BN(10_000_000),
      review: 2 * DAY,
    });
    await open(ctx, ctx.platform);
    await fund(ctx);
    const amt = new BN(476_190);
    const i0 = await stage(ctx, amt);
    const i1 = await stage(ctx, amt);
    warpTo(ctx.endAt + 1);
    send([await settleIx(ctx)], [ctx.platform]);
    expect(lamports(ctx.vault)).toBe(
      RENT_RESERVE + BigInt(grossOf(amt.muln(2)).toString()),
    );

    const employeeBefore = lamports(ctx.employee.publicKey);
    const treasuryBefore = lamports(treasury.publicKey);
    send([await approveIx(ctx, i0)], [ctx.employer]);
    send(
      [await approveIx(ctx, i1, { approver: ctx.platform })],
      [ctx.platform],
    );

    expect(lamports(ctx.employee.publicKey) - employeeBefore).toBe(
      BigInt(amt.muln(2).toString()),
    );
    expect(lamports(treasury.publicKey) - treasuryBefore).toBe(
      BigInt(commission(amt.muln(2)).toString()),
    );
    expect(lamports(ctx.vault)).toBe(RENT_RESERVE);
    const p = decodePeriod(ctx.period);
    expect(p.liveInvoices).toBe(0);
    expect(n(p.outstandingNet)).toBe(0);
    expect(n(p.outstandingCommission)).toBe(0);
  });

  test("close_period_sol sweeps the leftover rent reserve back to the employer", async () => {
    const ctx = newCtx({
      isNative: true,
      capNet: new BN(10_000_000),
      review: 2 * DAY,
    });
    await open(ctx, ctx.platform);
    const employerBeforeFund = lamports(ctx.employer.publicKey);
    await fund(ctx);
    const amt = new BN(400_000);
    const i0 = await stage(ctx, amt);
    warpTo(ctx.endAt + 1);
    send([await settleIx(ctx)], [ctx.platform]);
    send([await finalizeIx(ctx, i0)], [ctx.platform]);
    expect(lamports(ctx.vault)).toBe(RENT_RESERVE);

    const employerBeforeClose = lamports(ctx.employer.publicKey);
    send([await closeIx(ctx)], [ctx.platform]);

    expect(exists(ctx.vault)).toBe(false);
    expect(exists(ctx.period)).toBe(false);
    expect(lamports(ctx.employer.publicKey) - employerBeforeClose).toBe(
      RENT_RESERVE,
    );
    expect(employerBeforeFund - lamports(ctx.employer.publicKey)).toBe(
      BigInt(grossOf(amt).toString()),
    );
  });

  test("a native cap far below the rent-exempt minimum funds, stages and finalizes", async () => {
    const ctx = newCtx({
      isNative: true,
      capNet: new BN(50_000),
      review: DAY,
    });
    await open(ctx, ctx.platform);
    const gross = capGross(ctx);
    expect(BigInt(gross.toString())).toBeLessThan(RENT_RESERVE);
    await fund(ctx);

    expect(lamports(ctx.vault)).toBe(BigInt(gross.toString()) + RENT_RESERVE);
    expect(BigInt(n(decodePeriod(ctx.period).vaultRentReserve))).toBe(
      RENT_RESERVE,
    );

    const amt = new BN(50_000);
    const i0 = await stage(ctx, amt);
    warpBy(DAY + 1);
    const employeeBefore = lamports(ctx.employee.publicKey);
    const treasuryBefore = lamports(treasury.publicKey);
    send([await finalizeIx(ctx, i0)], [ctx.platform]);

    expect(lamports(ctx.employee.publicKey) - employeeBefore).toBe(
      BigInt(amt.toString()),
    );
    expect(lamports(treasury.publicKey) - treasuryBefore).toBe(
      BigInt(commission(amt).toString()),
    );
    expect(lamports(ctx.vault)).toBe(RENT_RESERVE);
    expect(exists(invoicePda(ctx.period, i0))).toBe(false);
  });

  test("a native cap below the rent-exempt minimum funds only cap_gross plus one reserve", async () => {
    const ctx = newCtx({ isNative: true, capNet: new BN(500_000) });
    await open(ctx, ctx.platform);
    const gross = capGross(ctx);
    const employerBefore = lamports(ctx.employer.publicKey);
    await fund(ctx);
    expect(lamports(ctx.vault)).toBe(BigInt(gross.toString()) + RENT_RESERVE);
    expect(employerBefore - lamports(ctx.employer.publicKey)).toBe(
      BigInt(gross.toString()) + RENT_RESERVE,
    );
    const p = decodePeriod(ctx.period);
    expect(n(p.fundedGross)).toBe(n(gross));
    expect(BigInt(n(p.vaultRentReserve))).toBe(RENT_RESERVE);
    expectFail([await fundIx(ctx)], [ctx.employer], "PeriodFullyFunded");
  });
});

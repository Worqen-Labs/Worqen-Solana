import { beforeAll, describe, expect, test } from "bun:test";
import * as anchor from "@coral-xyz/anchor";
import {
  AccountLayout,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createApproveInstruction,
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
const SO_PATH = "target/deploy/worqen_escrow.so";

const seed = (s: string) => Buffer.from(s);
const rand32 = () => Array.from(Keypair.generate().publicKey.toBytes());
const BPS = 500;
const DAY = 24 * 3600;
const BASE_TS = 1_900_000_000;

const CAP = new BN(1_000_000);
const CAP_COMMISSION = CAP.muln(BPS).divn(10000);
const CAP_GROSS = CAP.add(CAP_COMMISSION);
const REVIEW = new BN(7 * DAY);
const commission = (amt: BN) => amt.muln(BPS).divn(10000);

let svm: LiteSvm;
let program: anchor.Program;
let payer: Keypair;
let configPda: PublicKey;
let treasury: Keypair;
let mint: PublicKey;
let payerAta: PublicKey;
let treasuryAta: PublicKey;
let delegateAuth: PublicKey;

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

function tokenBalance(ata: PublicKey): bigint {
  const acc = svm.getAccount(ata.toBytes());
  if (acc === null) return 0n;
  return BigInt(
    AccountLayout.decode(Buffer.from(acc.data())).amount.toString(),
  );
}

function now(): number {
  return Number(svm.getClock().unixTimestamp);
}

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

function fundedKeypair(sol = 10): Keypair {
  const kp = Keypair.generate();
  svm.airdrop(kp.publicKey.toBytes(), BigInt(sol * LAMPORTS_PER_SOL));
  return kp;
}

function periodPdas(hireId: number[], periodIndex: number) {
  const idx = Buffer.alloc(4);
  idx.writeUInt32LE(periodIndex);
  const [period] = PublicKey.findProgramAddressSync(
    [seed("hourly"), Buffer.from(hireId), idx],
    PROGRAM_ID,
  );
  const vault = getAssociatedTokenAddressSync(mint, period, true);
  return { period, vault };
}

function decodeHourly(period: PublicKey): any {
  const acc = svm.getAccount(period.toBytes());
  if (acc === null) throw new Error("hourly period account not found");
  return program.coder.accounts.decode("hourlyPeriod", Buffer.from(acc.data()));
}

function trancheStatus(t: any): string {
  return Object.keys(t.status)[0];
}

function newHourly(periodIndex = 0) {
  const hireId = rand32();
  const { period, vault } = periodPdas(hireId, periodIndex);
  const employee = Keypair.generate();
  const platformAuthority = fundedKeypair();
  const employeeAta = getAssociatedTokenAddressSync(mint, employee.publicKey);
  return {
    hireId,
    periodIndex,
    period,
    vault,
    employee,
    platformAuthority,
    employeeAta,
  };
}

type Ctx = ReturnType<typeof newHourly>;

async function openPeriod(ctx: Ctx, cap = CAP, review = REVIEW) {
  const ix = await program.methods
    .openPeriod(ctx.hireId, ctx.periodIndex, cap, BPS, review)
    .accountsPartial({
      config: configPda,
      hourlyPeriod: ctx.period,
      employer: payer.publicKey,
      employee: ctx.employee.publicKey,
      platformAuthority: ctx.platformAuthority.publicKey,
      feeRecipient: treasury.publicKey,
      tokenMint: mint,
      vaultTokenAccount: ctx.vault,
      employeeTokenAccount: ctx.employeeAta,
      platformTokenAccount: treasuryAta,
      payer: payer.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  send([ix]);
}

async function fundPeriod(ctx: Ctx) {
  const ix = await program.methods
    .fundPeriod()
    .accountsPartial({
      config: configPda,
      hourlyPeriod: ctx.period,
      vaultTokenAccount: ctx.vault,
      employer: payer.publicKey,
      employerTokenAccount: payerAta,
      tokenMint: mint,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .instruction();
  send([ix]);
}

async function pullFundPeriod(ctx: Ctx) {
  const ix = await program.methods
    .pullFundPeriod()
    .accountsPartial({
      config: configPda,
      hourlyPeriod: ctx.period,
      vaultTokenAccount: ctx.vault,
      employerTokenAccount: payerAta,
      delegateAuthority: delegateAuth,
      tokenMint: mint,
      caller: payer.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .instruction();
  send([ix]);
}

async function stage(ctx: Ctx, amount: BN) {
  const ix = await program.methods
    .stageTranche(amount)
    .accountsPartial({
      config: configPda,
      hourlyPeriod: ctx.period,
      vaultTokenAccount: ctx.vault,
      platformAuthority: ctx.platformAuthority.publicKey,
    })
    .instruction();
  return ix;
}

async function finalize(ctx: Ctx, index: number) {
  const ix = await program.methods
    .finalizeTranche(index)
    .accountsPartial({
      hourlyPeriod: ctx.period,
      tokenMint: mint,
      vaultTokenAccount: ctx.vault,
      employee: ctx.employee.publicKey,
      employeeTokenAccount: ctx.employeeAta,
      feeRecipient: treasury.publicKey,
      platformTokenAccount: treasuryAta,
      caller: payer.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  return ix;
}

async function raiseDispute(ctx: Ctx, index: number, deadline: number) {
  const ix = await program.methods
    .raiseHourlyDispute(index, new BN(deadline), Buffer.from("bad work"))
    .accountsPartial({ hourlyPeriod: ctx.period, signer: payer.publicKey })
    .instruction();
  return ix;
}

async function resolve(ctx: Ctx, index: number, employeeShare: BN) {
  const ix = await program.methods
    .resolveHourlyTranche(index, employeeShare)
    .accountsPartial({
      hourlyPeriod: ctx.period,
      tokenMint: mint,
      vaultTokenAccount: ctx.vault,
      employer: payer.publicKey,
      employerTokenAccount: payerAta,
      employee: ctx.employee.publicKey,
      employeeTokenAccount: ctx.employeeAta,
      feeRecipient: treasury.publicKey,
      platformTokenAccount: treasuryAta,
      platformAuthority: ctx.platformAuthority.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  return ix;
}

async function triggerAuto(ctx: Ctx, index: number) {
  const ix = await program.methods
    .triggerHourlyAutoRelease(index)
    .accountsPartial({
      hourlyPeriod: ctx.period,
      tokenMint: mint,
      vaultTokenAccount: ctx.vault,
      employee: ctx.employee.publicKey,
      employeeTokenAccount: ctx.employeeAta,
      feeRecipient: treasury.publicKey,
      platformTokenAccount: treasuryAta,
      caller: payer.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  return ix;
}

async function refund(ctx: Ctx) {
  const ix = await program.methods
    .refundRemainder()
    .accountsPartial({
      hourlyPeriod: ctx.period,
      vaultTokenAccount: ctx.vault,
      employer: payer.publicKey,
      employerTokenAccount: payerAta,
      tokenMint: mint,
      signer: payer.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .instruction();
  return ix;
}

async function setPaused(paused: boolean) {
  const ix = await program.methods
    .updateConfig(null, null, paused, null)
    .accountsPartial({ config: configPda, authority: payer.publicKey })
    .instruction();
  send([ix]);
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
  svm.airdrop(payer.publicKey.toBytes(), BigInt(1000 * LAMPORTS_PER_SOL));
  treasury = Keypair.generate();
  svm.airdrop(treasury.publicKey.toBytes(), BigInt(LAMPORTS_PER_SOL));

  const conn = new Connection("http://localhost:8899");
  const wallet = new anchor.Wallet(payer);
  const provider = new anchor.AnchorProvider(conn, wallet, {
    commitment: "processed",
  });
  program = new anchor.Program(IDL as anchor.Idl, provider);
  [configPda] = PublicKey.findProgramAddressSync([seed("config")], PROGRAM_ID);
  [delegateAuth] = PublicKey.findProgramAddressSync(
    [seed("delegate_auth")],
    PROGRAM_ID,
  );

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

  const addMintIx = await program.methods
    .addAllowedMint(mint)
    .accountsPartial({ config: configPda, authority: payer.publicKey })
    .instruction();
  send([addMintIx]);
});

describe("hourly: open + fund", () => {
  test("open_period creates the account with expected defaults", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    const p = decodeHourly(ctx.period);
    expect(p.trancheCount).toBe(0);
    expect(Number(p.weeklyCapNet.toString())).toBe(CAP.toNumber());
    expect(Object.keys(p.status)[0]).toBe("open");
    expect(p.tranches.length).toBe(7);
    expect(trancheStatus(p.tranches[0])).toBe("empty");
  });

  test("re-open of the same (hire, index) is blocked", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    const ix = await program.methods
      .openPeriod(ctx.hireId, ctx.periodIndex, CAP, BPS, REVIEW)
      .accountsPartial({
        config: configPda,
        hourlyPeriod: ctx.period,
        employer: payer.publicKey,
        employee: ctx.employee.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        feeRecipient: treasury.publicKey,
        tokenMint: mint,
        vaultTokenAccount: ctx.vault,
        employeeTokenAccount: ctx.employeeAta,
        platformTokenAccount: treasuryAta,
        payer: payer.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .instruction();
    const res = svm.sendLegacyTransaction(buildTx([ix], []).serialize());
    expect(res.constructor.name).toBe("FailedTransactionMetadata");
  });

  test("fund_period moves cap_gross; second fund is rejected", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    expect(tokenBalance(ctx.vault)).toBe(BigInt(CAP_GROSS.toString()));
    const p = decodeHourly(ctx.period);
    expect(Object.keys(p.status)[0]).toBe("funded");
    expect(Number(p.fundedAmount.toString())).toBe(CAP_GROSS.toNumber());

    const ix = await program.methods
      .fundPeriod()
      .accountsPartial({
        config: configPda,
        hourlyPeriod: ctx.period,
        vaultTokenAccount: ctx.vault,
        employer: payer.publicKey,
        employerTokenAccount: payerAta,
        tokenMint: mint,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .instruction();
    expectFail([ix], [], "PeriodFullyFunded");
  });
});

describe("hourly: stage + finalize", () => {
  test("stage then finalize pays exactly tranche net + commission; siblings untouched", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);

    const a0 = new BN(400_000);
    const a1 = new BN(300_000);
    send([await stage(ctx, a0)], [ctx.platformAuthority]);
    send([await stage(ctx, a1)], [ctx.platformAuthority]);

    const treBefore = tokenBalance(treasuryAta);
    const vaultBefore = tokenBalance(ctx.vault);

    warpBy(7 * DAY + 1);
    send([await finalize(ctx, 0)]);

    expect(tokenBalance(ctx.employeeAta)).toBe(BigInt(a0.toString()));
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(a0).toString()),
    );
    const moved = a0.add(commission(a0));
    expect(vaultBefore - tokenBalance(ctx.vault)).toBe(
      BigInt(moved.toString()),
    );

    const p = decodeHourly(ctx.period);
    expect(trancheStatus(p.tranches[0])).toBe("finalized");
    expect(trancheStatus(p.tranches[1])).toBe("frozen");
  });

  test("finalize before window elapses is rejected", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    send([await stage(ctx, new BN(100_000))], [ctx.platformAuthority]);
    expectFail([await finalize(ctx, 0)], [], "TrancheWindowNotElapsed");
  });

  test("double-finalize of the same tranche is rejected", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    send([await stage(ctx, new BN(100_000))], [ctx.platformAuthority]);
    warpBy(7 * DAY + 1);
    send([await finalize(ctx, 0)]);
    expectFail([await finalize(ctx, 0)], [], "TrancheNotFrozen");
  });

  test("finalize on an unstaged index is out of bounds", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    expectFail([await finalize(ctx, 0)], [], "InvalidTrancheIndex");
  });
});

describe("hourly: cap enforcement", () => {
  test("staging past the weekly cap is rejected", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    send([await stage(ctx, new BN(900_000))], [ctx.platformAuthority]);
    expectFail(
      [await stage(ctx, new BN(200_000))],
      [ctx.platformAuthority],
      "WeeklyCapExceeded",
    );
  });

  test("an 8th tranche is rejected", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    for (let i = 0; i < 7; i++) {
      send([await stage(ctx, new BN(100_000))], [ctx.platformAuthority]);
    }
    expectFail(
      [await stage(ctx, new BN(100_000))],
      [ctx.platformAuthority],
      "TrancheLimitReached",
    );
  });
});

describe("hourly: disputes", () => {
  test("raise dispute freezes the tranche and moves no money", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    send([await stage(ctx, new BN(400_000))], [ctx.platformAuthority]);
    const vaultBefore = tokenBalance(ctx.vault);
    send([await raiseDispute(ctx, 0, now() + 5 * DAY)]);
    expect(tokenBalance(ctx.vault)).toBe(vaultBefore);
    const p = decodeHourly(ctx.period);
    expect(trancheStatus(p.tranches[0])).toBe("disputed");
    expectFail([await finalize(ctx, 0)], [], "TrancheNotFrozen");
  });

  test("resolve splits the tranche exactly: worker + treasury + employer == amount + commission", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    const amt = new BN(400_000);
    send([await stage(ctx, amt)], [ctx.platformAuthority]);
    send([await raiseDispute(ctx, 0, now() + 5 * DAY)]);

    const empBefore = tokenBalance(ctx.employeeAta);
    const treBefore = tokenBalance(treasuryAta);
    const employerBefore = tokenBalance(payerAta);
    const vaultBefore = tokenBalance(ctx.vault);

    const share = new BN(250_000);
    send([await resolve(ctx, 0, share)], [ctx.platformAuthority]);

    const empDelta = tokenBalance(ctx.employeeAta) - empBefore;
    const treDelta = tokenBalance(treasuryAta) - treBefore;
    const employerDelta = tokenBalance(payerAta) - employerBefore;
    const vaultDelta = vaultBefore - tokenBalance(ctx.vault);

    const trancheCommission = commission(amt);
    const total = amt.add(trancheCommission);
    expect(empDelta).toBe(BigInt(share.toString()));
    expect(empDelta + treDelta + employerDelta).toBe(BigInt(total.toString()));
    expect(vaultDelta).toBe(BigInt(total.toString()));

    const p = decodeHourly(ctx.period);
    expect(trancheStatus(p.tranches[0])).toBe("resolved");
  });

  test("auto-release after deadline pays the worker in full; before deadline is rejected", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    const amt = new BN(400_000);
    send([await stage(ctx, amt)], [ctx.platformAuthority]);
    const deadline = now() + 4 * DAY;
    send([await raiseDispute(ctx, 0, deadline)]);

    expectFail([await triggerAuto(ctx, 0)], [], "DisputeDeadlineNotReached");

    const empBefore = tokenBalance(ctx.employeeAta);
    const treBefore = tokenBalance(treasuryAta);
    warpBy(4 * DAY + 1);
    send([await triggerAuto(ctx, 0)]);
    expect(tokenBalance(ctx.employeeAta) - empBefore).toBe(
      BigInt(amt.toString()),
    );
    expect(tokenBalance(treasuryAta) - treBefore).toBe(
      BigInt(commission(amt).toString()),
    );
  });
});

describe("hourly: refund safety", () => {
  test("refund returns only vault-minus-liabilities; a frozen tranche still finalizes", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    const amt = new BN(400_000);
    send([await stage(ctx, amt)], [ctx.platformAuthority]);

    const liabilities = amt.add(commission(amt));
    const employerBefore = tokenBalance(payerAta);
    const vaultBefore = tokenBalance(ctx.vault);

    send([await refund(ctx)]);

    const refunded = vaultBefore - BigInt(liabilities.toString());
    expect(tokenBalance(payerAta) - employerBefore).toBe(refunded);
    expect(tokenBalance(ctx.vault)).toBe(BigInt(liabilities.toString()));

    warpBy(7 * DAY + 1);
    send([await finalize(ctx, 0)]);
    expect(tokenBalance(ctx.employeeAta)).toBe(BigInt(amt.toString()));
  });
});

describe("hourly: external delegate funding", () => {
  test("pull_fund_period funds the vault via the delegate; replay is rejected", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    send([
      createApproveInstruction(
        payerAta,
        delegateAuth,
        payer.publicKey,
        BigInt(CAP_GROSS.toString()) * 4n,
      ),
    ]);
    await pullFundPeriod(ctx);
    expect(tokenBalance(ctx.vault)).toBe(BigInt(CAP_GROSS.toString()));
    expectFail(
      [
        await program.methods
          .pullFundPeriod()
          .accountsPartial({
            config: configPda,
            hourlyPeriod: ctx.period,
            vaultTokenAccount: ctx.vault,
            employerTokenAccount: payerAta,
            delegateAuthority: delegateAuth,
            tokenMint: mint,
            caller: payer.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .instruction(),
      ],
      [],
      "PeriodFullyFunded",
    );
  });
});

describe("hourly: pause invariant", () => {
  test("pause blocks staging but never blocks finalize", async () => {
    const ctx = newHourly();
    await openPeriod(ctx);
    await fundPeriod(ctx);
    send([await stage(ctx, new BN(400_000))], [ctx.platformAuthority]);

    await setPaused(true);
    expectFail(
      [await stage(ctx, new BN(100_000))],
      [ctx.platformAuthority],
      "ProgramPaused",
    );

    warpBy(7 * DAY + 1);
    send([await finalize(ctx, 0)]);
    expect(tokenBalance(ctx.employeeAta)).toBe(BigInt("400000"));
    await setPaused(false);
  });
});

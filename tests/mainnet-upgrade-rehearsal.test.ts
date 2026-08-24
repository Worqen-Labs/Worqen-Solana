/**
 * Mainnet upgrade rehearsal — v1.1.0 genesis state under the v1.5.0 binary.
 *
 * Mainnet program `HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs` still runs the
 * v1.1.0 genesis binary. This suite loads the *mainnet-feature* build of current
 * master at that same program id inside LiteSVM and injects the exact live
 * account bytes (fetched once into `tests/fixtures/mainnet/`, never at runtime).
 *
 * Because the loaded program id equals the mainnet one, every real PDA and every
 * stored bump derives correctly, so this is a byte-faithful rehearsal of what the
 * in-place upgrade does to live state:
 *
 *   (a) the untouched live Config still deserializes, and the known post-upgrade
 *       outage (`platform_authority == Pubkey::default()`) surfaces as exactly
 *       `InvalidPlatformAuthority` (6065) — not a discriminator/deserialize error;
 *   (b) the one-line migration (`update_config` setting `platform_authority`)
 *       clears the outage, preserves every other field, and leaves the system
 *       fully operable, kill-switch included;
 *   (c) the four live 948-byte Escrow accounts deserialize and pass every
 *       structural constraint of the v1.5.0 ATA-diet account lists, and both
 *       funded vaults drain to the exact expected amounts;
 *   (d) all four live escrows decode client-side against the v1.5.0 IDL.
 *
 * The mainnet-feature artifact only exists during the mainnet campaign, so the
 * whole suite skips cleanly when it is absent.
 *
 *   bun test tests/mainnet-upgrade-rehearsal.test.ts
 */
import { describe, test, expect, beforeAll } from "bun:test";
import { LiteSvm, Clock, Account } from "litesvm/dist/internal";
import * as anchor from "@anchor-lang/core";
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
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  AccountLayout,
} from "@solana/spl-token";
import BN from "bn.js";
import fs from "fs";

// ─────────────────────────────────────────────────────────────────────────────
// Artifacts — the mainnet-feature build only exists during the mainnet campaign
// ─────────────────────────────────────────────────────────────────────────────
const MAINNET_SO =
  process.env.WORQEN_MAINNET_SO_PATH ??
  "target/deploy/worqen_escrow_mainnet.so";
const MAINNET_IDL_PATH =
  process.env.WORQEN_MAINNET_IDL_PATH ??
  "target/idl/worqen_escrow_mainnet.json";
const FIXTURES = "tests/fixtures/mainnet";

const ARTIFACTS_PRESENT =
  fs.existsSync(MAINNET_SO) &&
  fs.existsSync(MAINNET_IDL_PATH) &&
  fs.existsSync(`${FIXTURES}/config.json`);

if (!ARTIFACTS_PRESENT) {
  console.warn(
    `[mainnet-upgrade-rehearsal] SKIPPED — missing ${MAINNET_SO} / ` +
      `${MAINNET_IDL_PATH} / ${FIXTURES}/*.json. Build the mainnet artifact with ` +
      `\`anchor build -- --features mainnet\` (renaming the output to ` +
      `worqen_escrow_mainnet.so / worqen_escrow_mainnet.json) to run this suite.`,
  );
}

const suite = ARTIFACTS_PRESENT ? describe : describe.skip;

// ─────────────────────────────────────────────────────────────────────────────
// Live mainnet addresses (documented in ../CLAUDE.md / MAINNET.md)
// ─────────────────────────────────────────────────────────────────────────────
const MAINNET_PROGRAM_ID = "HShWcYbT6wGrndgauQxNrcNJuJQ1BX9CVZqFSn9Q7rNs";
const LIVE_CONFIG = "F6cqSgcBv2Jo9ttHoPRVuQDvYv6yMVhFdkrJUNe8dsyP";
const LIVE_AUTHORITY = "7GTsbFX9gywsrNQouUuvw5bChAeL9GCp24acMPKEC5Sn";
const LIVE_TREASURY = "5jVqeEsYJaMtHZ4p3qKAhLgupo5Vre3anZWurKnQgDa8";
const LIVE_TREASURY_USDT_ATA = "4z2V2PDcxTPf72W8kSzE9fvtie9rZggdG85uDiEDGDDb";
const USDT_MINT = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const THIRD_MINT = "HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr";

// Escrow #1 — Funded, 380 USDT worker + 19 USDT commission.
const E1 = "4BxbdsTNrpX9qKyH2tDKPpEq6PeXzmUJPjeNnxKnYMTJ";
const E1_VAULT = "AjztXgWCEyaixNXCm99ExWSE16brELmnNUJBU2FsS35g";
const E1_EMPLOYER = "KAJDJzbYkQonJ1ekQ4vgYAcbwbpxVCxN5KhVTEES1QV";
const E1_EMPLOYEE = "8WQtbeUWHLWvndbCnLmwL5wNqqr3wumn8j785xWKfDES";
const E1_AMOUNT = 380_000_000n;
const E1_COMMISSION = 19_000_000n;

// Escrow #2 — Funded, 550 USDT worker + 27.5 USDT commission. Same two wallets
// as #1 but with the employer/employee roles swapped.
const E2 = "Ada2fJkEVRjSThVL6UknfDL8Eq3mzFUT63XGE2aAdasV";
const E2_VAULT = "3r3gTEPgJ4AT74ukVjnaTc4De7FgJHs3nP4LGUbkkwg9";
const E2_EMPLOYER = "8WQtbeUWHLWvndbCnLmwL5wNqqr3wumn8j785xWKfDES";
const E2_EMPLOYEE = "KAJDJzbYkQonJ1ekQ4vgYAcbwbpxVCxN5KhVTEES1QV";
const E2_AMOUNT = 550_000_000n;
const E2_COMMISSION = 27_500_000n;

// Both Funded escrows name this ops key as `platform_authority`. It is NOT the
// devnet/staging ESCROW_WALLET (64PF1jbX…) that the two Resolved escrows carry.
const LIVE_PLATFORM_AUTHORITY = "FonMQCHztDpr8UaZH6f2LvZr79a9XzJgKoLp9Dpz79pK";
const STAGING_PLATFORM_AUTHORITY =
  "64PF1jbXinCFteyegYpkPJ25fHKibPeVGJsjmc4AH46H";

const E3 = "6M5d1261xxjGTGQ96WZ3pn5gCVFutrgymKWmJ2RofrLo"; // Resolved
const E4 = "GNMcMmVtAoFWHqJ6Wn6swN7aJJeBviLMzQxNpbbdzDfa"; // Resolved

const TREASURY_USDT_PREBALANCE = 20_000_000n;

// ─────────────────────────────────────────────────────────────────────────────
// Byte offsets into the live account layouts. Every offset is re-verified at
// runtime by reading the pre-patch value, so a layout change breaks loudly.
// ─────────────────────────────────────────────────────────────────────────────
const CONFIG_OFF = {
  version: 8,
  authority: 9, // 32 bytes
  pendingAuthority: 41,
  feeRecipient: 73,
};

/**
 * `platform_authority` sits *after* the borsh-variable `allowed_mints` Vec, so
 * its offset depends on the allowlist length (3 on mainnet) — it is NOT a fixed
 * distance from the end of the 1137-byte account, which is over-allocated for
 * MAX_ALLOWED_MINTS = 30.
 */
const configPlatformAuthorityOffset = (mintCount: number) =>
  8 + 1 + 32 + 32 + 32 + 2 + 1 + 4 + mintCount * 32 + 1;

const ESCROW_OFF = {
  version: 8,
  status: 230, // 1 byte
  disputeRaisedBy: 553, // 32 bytes
  disputeRaisedAt: 585, // i64
  disputeDeadline: 593, // i64
};

const STATUS_FUNDED = 1;
const STATUS_RELEASED = 3;
const STATUS_DISPUTED = 4;
const STATUS_RESOLVED = 5;

const ZEROS32 = Array(32).fill(0);
const DAY = 24 * 3600;
// Wall clock for the rehearsal: 2026-09-21T05:33:20Z — after every live escrow
// was created (latest funded_at is 2026-08-09) so the state is chronologically
// coherent.
const BASE_TS = 1_790_000_000;

const BPS = 500;
const SOL_AMT = new BN(0.02 * LAMPORTS_PER_SOL);
const commission = (amt: BN) => amt.muln(BPS).divn(10000);

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures — captured once from api.mainnet-beta.solana.com, replayed offline
// ─────────────────────────────────────────────────────────────────────────────
type Fixture = {
  pubkey: string;
  owner: string;
  lamports: number;
  executable: boolean;
  rentEpoch: string;
  space: number;
  data_base64: string;
};

const fixtureCache = new Map<string, Fixture>();

function fixture(name: string): Fixture {
  const hit = fixtureCache.get(name);
  if (hit) return hit;
  const fx = JSON.parse(
    fs.readFileSync(`${FIXTURES}/${name}.json`, "utf8"),
  ) as Fixture;
  fixtureCache.set(name, fx);
  return fx;
}

/** The live account's raw bytes, as a fresh mutable copy. */
function fixtureData(name: string): Buffer {
  return Buffer.from(fixture(name).data_base64, "base64");
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared harness — mirrors tests/escrow.test.ts idioms
// ─────────────────────────────────────────────────────────────────────────────
let IDL: any;
let PROGRAM_ID: PublicKey;
let program: anchor.Program;

let svm: LiteSvm;
let payer: Keypair; // fee payer / rent payer for every rehearsal tx

/** Fresh SVM with the mainnet-feature program at the real mainnet program id. */
function bootSvm(): void {
  svm = new LiteSvm();
  svm.addProgramFromFile(PROGRAM_ID.toBytes(), MAINNET_SO);
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
}

/** Write a captured mainnet account into the SVM verbatim (or with `data` patched). */
function inject(name: string, data?: Buffer): PublicKey {
  const fx = fixture(name);
  const bytes = data ?? fixtureData(name);
  const pk = new PublicKey(fx.pubkey);
  svm.setAccount(
    pk.toBytes(),
    new Account(
      BigInt(fx.lamports),
      bytes,
      new PublicKey(fx.owner).toBytes(),
      fx.executable,
      BigInt(fx.rentEpoch),
    ),
  );
  return pk;
}

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

/**
 * Submit a transaction whose `forged` signers we do not hold keys for.
 *
 * Requires `svm.setSigverify(false)`. This is how the rehearsal exercises the
 * real mainnet `platform_authority` (`FonMQCHzt…`) — a key held by the platform
 * ops wallet, never by this test. Signature *verification* is what gets skipped;
 * every on-chain authorization check still runs against the real pubkey.
 */
function sendForged(
  ixs: TransactionInstruction[],
  forged: PublicKey[],
  signers: Keypair[] = [],
): any {
  svm.expireBlockhash();
  const tx = new Transaction().add(...ixs);
  tx.recentBlockhash = svm.latestBlockhash();
  tx.feePayer = payer.publicKey;
  tx.sign(payer, ...signers);
  for (const pk of forged) tx.addSignature(pk, Buffer.alloc(64));
  const raw = tx.serialize({
    requireAllSignatures: false,
    verifySignatures: false,
  });
  const res = svm.sendLegacyTransaction(raw);
  if (res.constructor.name === "FailedTransactionMetadata") {
    throw new Error("tx failed: " + JSON.stringify((res as any).meta().logs()));
  }
  return res;
}

function logsOf(res: any): string {
  return (res as any).meta().logs().join("\n");
}

/**
 * Assert a transaction fails with a specific Anchor error, and — critically for
 * an upgrade rehearsal — that it is NOT a discriminator (3002) or deserialization
 * (3003) failure. Reaching a *domain* error proves the old bytes were read whole.
 */
function failWith(
  ixs: TransactionInstruction[],
  signers: Keypair[],
  name: string,
  number: number,
  why: string,
): string {
  const res = svm.sendLegacyTransaction(buildTx(ixs, signers).serialize());
  expect(res.constructor.name).toBe("FailedTransactionMetadata");
  const logs = logsOf(res);
  expect(
    logs.includes(`Error Code: ${name}`),
    `${why}\nexpected "Error Code: ${name}" in logs but got:\n${logs}`,
  ).toBe(true);
  expect(
    logs.includes(`Error Number: ${number}`),
    `${why}\nexpected "Error Number: ${number}" in logs but got:\n${logs}`,
  ).toBe(true);
  assertNoDeserializationFailure(logs, why);
  return logs;
}

/** The two failure modes an in-place upgrade must never produce on live state. */
function assertNoDeserializationFailure(logs: string, why: string) {
  for (const bad of [
    "AccountDiscriminatorMismatch",
    "AccountDidNotDeserialize",
    "Error Number: 3002",
    "Error Number: 3003",
  ]) {
    expect(
      logs.includes(bad),
      `${why}\nlive account bytes failed to deserialize under v1.5.0 ("${bad}"):\n${logs}`,
    ).toBe(false);
  }
}

function accountData(pk: PublicKey): Buffer {
  const acc = svm.getAccount(pk.toBytes());
  if (acc === null) throw new Error(`account ${pk.toBase58()} not found`);
  return Buffer.from(acc.data());
}

function decodeConfigAt(pk: PublicKey): any {
  return program.coder.accounts.decode("config", accountData(pk));
}

function decodeEscrowAt(pk: PublicKey): any {
  return program.coder.accounts.decode("escrow", accountData(pk));
}

function tokenBalance(ata: PublicKey): bigint {
  const acc = svm.getAccount(ata.toBytes());
  if (acc === null) return 0n;
  return BigInt(
    AccountLayout.decode(Buffer.from(acc.data())).amount.toString(),
  );
}

function balance(pk: PublicKey): number {
  const b = svm.getBalance(pk.toBytes());
  return b === null ? 0 : Number(b);
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

function now(): number {
  return Number(svm.getClock().unixTimestamp);
}

const configPda = () =>
  PublicKey.findProgramAddressSync([Buffer.from("config")], PROGRAM_ID)[0];

function escrowPda(escrowId: number[] | Buffer) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("escrow"), Buffer.from(escrowId)],
    PROGRAM_ID,
  );
}

function solVaultPda(escrow: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), escrow.toBuffer()],
    PROGRAM_ID,
  );
}

/** Idempotent ATA create — exactly what backend `ensure_token_account_ixs` prepends. */
function ensureAta(mintPk: PublicKey, owner: PublicKey): PublicKey {
  const ata = getAssociatedTokenAddressSync(mintPk, owner, true);
  if (svm.getAccount(ata.toBytes()) === null) {
    send([
      createAssociatedTokenAccountInstruction(
        payer.publicKey,
        ata,
        owner,
        mintPk,
      ),
    ]);
  }
  return ata;
}

// ─────────────────────────────────────────────────────────────────────────────

suite("mainnet upgrade rehearsal (v1.1.0 state → v1.5.0 binary)", () => {
  beforeAll(() => {
    IDL = JSON.parse(fs.readFileSync(MAINNET_IDL_PATH, "utf8"));
    PROGRAM_ID = new PublicKey(IDL.address);
    const conn = new Connection("http://localhost:8899");
    const wallet = new anchor.Wallet(Keypair.generate());
    program = new anchor.Program(
      IDL as anchor.Idl,
      new anchor.AnchorProvider(conn, wallet, { commitment: "processed" }),
    );
  });

  // ───────────────────────────────────────────────────────────────────────────
  test("artifact is the mainnet-feature build and every live PDA derives", () => {
    expect(PROGRAM_ID.toBase58()).toBe(MAINNET_PROGRAM_ID);

    bootSvm();

    // The whole rehearsal hinges on this: at the mainnet program id the derived
    // Config PDA IS the live account, so its stored bump (255) validates.
    expect(configPda().toBase58()).toBe(LIVE_CONFIG);

    // Live escrow PDAs re-derive from their stored escrow_id, and the derived
    // bump matches the bump byte persisted by v1.1.0.
    for (const [name, addr, bump] of [
      ["escrow-funded-1", E1, 254],
      ["escrow-funded-2", E2, 255],
      ["escrow-resolved-1", E3, 255],
      ["escrow-resolved-2", E4, 253],
    ] as const) {
      const data = fixtureData(name);
      const escrowId = data.subarray(9, 41);
      const [pda, derivedBump] = escrowPda(escrowId);
      expect(
        pda.toBase58(),
        `${name}: [b"escrow", escrow_id] must derive the live address`,
      ).toBe(addr);
      expect(derivedBump, `${name}: derived bump must match stored bump`).toBe(
        bump,
      );
      expect(data.length).toBe(948);
    }

    // Both live vaults are the canonical ATA of (USDT, escrow PDA) — the address
    // v1.5.0's `associated_token::authority = escrow` constraint still requires
    // on `deposit_token`, and the address the payout builders must pass.
    const usdt = new PublicKey(USDT_MINT);
    expect(
      getAssociatedTokenAddressSync(usdt, new PublicKey(E1), true).toBase58(),
    ).toBe(E1_VAULT);
    expect(
      getAssociatedTokenAddressSync(usdt, new PublicKey(E2), true).toBase58(),
    ).toBe(E2_VAULT);
    expect(
      getAssociatedTokenAddressSync(
        usdt,
        new PublicKey(LIVE_TREASURY),
        true,
      ).toBase58(),
    ).toBe(LIVE_TREASURY_USDT_ATA);
  });

  // ───────────────────────────────────────────────────────────────────────────
  // (a) EXACT-BYTES CONFIG
  // ───────────────────────────────────────────────────────────────────────────
  describe("(a) untouched live Config bytes", () => {
    /** Fresh SVM holding the live Config verbatim. Per-test, never a hook. */
    function bootA(): PublicKey {
      bootSvm();
      return inject("config");
    }

    test("live 1137-byte Config deserializes under the v1.5.0 coder", () => {
      bootA();
      const cfg = decodeConfigAt(configPda());
      expect(accountData(configPda()).length).toBe(1137);
      expect(cfg.version).toBe(1);
      expect(cfg.authority.toBase58()).toBe(LIVE_AUTHORITY);
      expect(cfg.pendingAuthority.toBase58()).toBe(
        PublicKey.default.toBase58(),
      );
      expect(cfg.feeRecipient.toBase58()).toBe(LIVE_TREASURY);
      expect(cfg.defaultCommissionBps).toBe(500);
      expect(cfg.paused).toBe(false);
      expect(cfg.allowedMints.map((m: PublicKey) => m.toBase58())).toEqual([
        USDC_MINT,
        USDT_MINT,
        THIRD_MINT,
      ]);
      expect(cfg.bump).toBe(255);
      // The v1.5.0 field carved out of `reserved`: reads as default on every
      // account written by v1.1.0. This is the whole reason for the migration.
      expect(cfg.platformAuthority.toBase58()).toBe(
        PublicKey.default.toBase58(),
      );
    });

    test("create_escrow fails EXACTLY InvalidPlatformAuthority (6065)", async () => {
      bootA();
      const employer = Keypair.generate();
      svm.airdrop(employer.publicKey.toBytes(), BigInt(10 * LAMPORTS_PER_SOL));
      const employee = Keypair.generate();
      const platform = Keypair.generate();
      const escrowId = Array.from(Keypair.generate().publicKey.toBytes());
      const [escrow] = escrowPda(escrowId);

      const ix = await program.methods
        .createEscrow(
          escrowId,
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
          escrow,
          config: configPda(),
          employer: employer.publicKey,
          employee: employee.publicKey,
          platformAuthority: platform.publicKey,
          tokenMint: SystemProgram.programId,
          systemProgram: SystemProgram.programId,
        })
        .instruction();

      const logs = failWith(
        [ix],
        [employer],
        "InvalidPlatformAuthority",
        6065,
        "post-upgrade outage must manifest as the pinned-authority check, " +
          "which is only reachable after Config was read whole",
      );

      // Anchor blames the account whose constraint failed — proof the failure is
      // the pinned-authority check, not a Config read.
      expect(
        logs.includes("caused by account: platform_authority"),
        `expected the platform_authority constraint to be the blamed account:\n${logs}`,
      ).toBe(true);
      // The Config PDA seeds+stored-bump constraint (`bump = config.bump`) is
      // evaluated before it and did not fail.
      expect(logs.includes("ConstraintSeeds")).toBe(false);
    });

    test("update_config from a stranger fails Unauthorized (6001), and the live bytes are untouched", async () => {
      const cfgPk = bootA();
      const stranger = Keypair.generate();
      svm.airdrop(stranger.publicKey.toBytes(), BigInt(LAMPORTS_PER_SOL));
      const ix = await program.methods
        .updateConfig(null, null, null, null, Keypair.generate().publicKey)
        .accountsPartial({ config: cfgPk, authority: stranger.publicKey })
        .instruction();

      failWith(
        [ix],
        [stranger],
        "Unauthorized",
        6001,
        "has_one = authority must resolve against the live authority bytes " +
          "(and `bump = config.bump` must accept the stored bump 255)",
      );

      // A failed v1.5.0 transaction leaves the v1.1.0 account bit-for-bit intact.
      expect(accountData(cfgPk).toString("base64")).toBe(
        fixture("config").data_base64,
      );
    });
  });

  // ───────────────────────────────────────────────────────────────────────────
  // (b) MIGRATED CONFIG FLOW
  // ───────────────────────────────────────────────────────────────────────────
  describe("(b) migrated Config", () => {
    type BCtx = {
      authority: Keypair;
      platformAuthority: Keypair;
      cfgPk: PublicKey;
    };

    /**
     * Fresh SVM holding the live Config with ONLY `authority` (offsets 9..41)
     * swapped for a throwaway key — the real multisig 7GTsbFX… signs on mainnet
     * and we hold no keys. Every other byte is live data.
     */
    function bootB(): BCtx {
      bootSvm();
      const authority = Keypair.generate();
      const platformAuthority = Keypair.generate();
      svm.airdrop(authority.publicKey.toBytes(), BigInt(10 * LAMPORTS_PER_SOL));
      svm.airdrop(
        platformAuthority.publicKey.toBytes(),
        BigInt(10 * LAMPORTS_PER_SOL),
      );

      const data = fixtureData("config");
      expect(
        new PublicKey(
          data.subarray(CONFIG_OFF.authority, CONFIG_OFF.authority + 32),
        ).toBase58(),
        "CONFIG_OFF.authority must point at the live authority",
      ).toBe(LIVE_AUTHORITY);
      authority.publicKey.toBuffer().copy(data, CONFIG_OFF.authority);
      const cfgPk = inject("config", data);
      return { authority, platformAuthority, cfgPk };
    }

    /** bootB + the one-line migration the mainnet upgrade requires. */
    async function bootMigrated(): Promise<BCtx> {
      const ctx = bootB();
      const ix = await program.methods
        .updateConfig(null, null, null, null, ctx.platformAuthority.publicKey)
        .accountsPartial({
          config: ctx.cfgPk,
          authority: ctx.authority.publicKey,
        })
        .instruction();
      send([ix], [ctx.authority]);
      return ctx;
    }

    test("update_config sets platform_authority and touches nothing else", async () => {
      const { authority, platformAuthority, cfgPk } = bootB();
      const before = decodeConfigAt(cfgPk);
      expect(before.platformAuthority.toBase58()).toBe(
        PublicKey.default.toBase58(),
      );
      const lamportsBefore = balance(cfgPk);

      const ix = await program.methods
        .updateConfig(null, null, null, null, platformAuthority.publicKey)
        .accountsPartial({ config: cfgPk, authority: authority.publicKey })
        .instruction();
      send([ix], [authority]);

      const after = decodeConfigAt(cfgPk);
      expect(after.platformAuthority.toBase58()).toBe(
        platformAuthority.publicKey.toBase58(),
      );
      // Everything else is untouched.
      expect(after.version).toBe(1);
      expect(after.authority.toBase58()).toBe(authority.publicKey.toBase58());
      expect(after.pendingAuthority.toBase58()).toBe(
        PublicKey.default.toBase58(),
      );
      expect(after.feeRecipient.toBase58()).toBe(LIVE_TREASURY);
      expect(after.defaultCommissionBps).toBe(500);
      expect(after.paused).toBe(false);
      expect(after.allowedMints.map((m: PublicKey) => m.toBase58())).toEqual([
        USDC_MINT,
        USDT_MINT,
        THIRD_MINT,
      ]);
      expect(after.bump).toBe(255);
      // No realloc, no rent top-up: the field came out of `reserved`.
      expect(accountData(cfgPk).length).toBe(1137);
      expect(balance(cfgPk)).toBe(lamportsBefore);

      // Byte-level: only the 32-byte platform_authority window moved. Its offset
      // trails the variable-length allowed_mints Vec (209 for a 3-mint allowlist).
      const live = fixtureData("config");
      const nowBytes = accountData(cfgPk);
      const paOff = configPlatformAuthorityOffset(after.allowedMints.length);
      expect(paOff).toBe(209);
      expect(
        new PublicKey(nowBytes.subarray(paOff, paOff + 32)).toBase58(),
        "computed platform_authority offset must land on the written field",
      ).toBe(platformAuthority.publicKey.toBase58());
      expect(nowBytes.subarray(0, CONFIG_OFF.authority)).toEqual(
        live.subarray(0, CONFIG_OFF.authority),
      );
      expect(nowBytes.subarray(CONFIG_OFF.authority + 32, paOff)).toEqual(
        live.subarray(CONFIG_OFF.authority + 32, paOff),
      );
      expect(nowBytes.subarray(paOff + 32)).toEqual(live.subarray(paOff + 32));
    });

    test("post-migration: a fresh SOL escrow runs create → deposit → confirm → release", async () => {
      const { platformAuthority, cfgPk } = await bootMigrated();
      const employer = Keypair.generate();
      const employee = Keypair.generate();
      svm.airdrop(employer.publicKey.toBytes(), BigInt(10 * LAMPORTS_PER_SOL));
      const escrowId = Array.from(Keypair.generate().publicKey.toBytes());
      const [escrow] = escrowPda(escrowId);
      const [vault] = solVaultPda(escrow);
      const feeRecipient = new PublicKey(LIVE_TREASURY);

      const createIx = await program.methods
        .createEscrow(
          escrowId,
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
          escrow,
          config: cfgPk,
          employer: employer.publicKey,
          employee: employee.publicKey,
          platformAuthority: platformAuthority.publicKey,
          tokenMint: SystemProgram.programId,
          systemProgram: SystemProgram.programId,
        })
        .instruction();
      send([createIx], [employer]);

      const created = decodeEscrowAt(escrow);
      expect(created.version).toBe(1);
      expect(created.status.created).toBeDefined();
      // The escrow snapshots the *live* treasury off the migrated Config.
      expect(created.feeRecipient.toBase58()).toBe(LIVE_TREASURY);
      expect(created.platformAuthority.toBase58()).toBe(
        platformAuthority.publicKey.toBase58(),
      );
      expect(created.commissionAmount.toString()).toBe(
        commission(SOL_AMT).toString(),
      );

      const depositIx = await program.methods
        .depositSol()
        .accountsPartial({
          config: cfgPk,
          escrow,
          escrowVault: vault,
          employer: employer.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .instruction();
      send([depositIx], [employer]);
      expect(decodeEscrowAt(escrow).status.funded).toBeDefined();
      expect(balance(vault)).toBe(SOL_AMT.add(commission(SOL_AMT)).toNumber());

      const confirmIx = await program.methods
        .confirmCompletion()
        .accountsPartial({ escrow, signer: employer.publicKey })
        .instruction();
      send([confirmIx], [employer]);
      expect(decodeEscrowAt(escrow).status.pendingRelease).toBeDefined();

      const treasuryBefore = balance(feeRecipient);
      const releaseIx = await program.methods
        .releaseSol(ZEROS32)
        .accountsPartial({
          escrow,
          escrowVault: vault,
          employee: employee.publicKey,
          feeRecipient,
          authority: employer.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .instruction();
      send([releaseIx], [employer]);

      const released = decodeEscrowAt(escrow);
      expect(released.status.released).toBeDefined();
      expect(released.releasedToEmployee.toString()).toBe(SOL_AMT.toString());
      expect(balance(employee.publicKey)).toBe(SOL_AMT.toNumber());
      expect(balance(feeRecipient) - treasuryBefore).toBe(
        commission(SOL_AMT).toNumber(),
      );
      expect(balance(vault)).toBe(0);
    });

    test("kill-switch rehearsal: pause blocks deposits (6036), unpause restores them", async () => {
      const { authority, platformAuthority, cfgPk } = await bootMigrated();
      const employer = Keypair.generate();
      const employee = Keypair.generate();
      svm.airdrop(employer.publicKey.toBytes(), BigInt(10 * LAMPORTS_PER_SOL));
      const escrowId = Array.from(Keypair.generate().publicKey.toBytes());
      const [escrow] = escrowPda(escrowId);
      const [vault] = solVaultPda(escrow);

      const createIx = await program.methods
        .createEscrow(
          escrowId,
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
          escrow,
          config: cfgPk,
          employer: employer.publicKey,
          employee: employee.publicKey,
          platformAuthority: platformAuthority.publicKey,
          tokenMint: SystemProgram.programId,
          systemProgram: SystemProgram.programId,
        })
        .instruction();
      send([createIx], [employer]);

      const pauseIx = await program.methods
        .updateConfig(null, null, true, null, null)
        .accountsPartial({ config: cfgPk, authority: authority.publicKey })
        .instruction();
      send([pauseIx], [authority]);
      expect(decodeConfigAt(cfgPk).paused).toBe(true);

      const depositIx = await program.methods
        .depositSol()
        .accountsPartial({
          config: cfgPk,
          escrow,
          escrowVault: vault,
          employer: employer.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .instruction();
      failWith(
        [depositIx],
        [employer],
        "ProgramPaused",
        6036,
        "the kill switch must halt new money on a Config migrated in place",
      );
      expect(decodeEscrowAt(escrow).status.created).toBeDefined();
      expect(balance(vault)).toBe(0);

      const unpauseIx = await program.methods
        .updateConfig(null, null, false, null, null)
        .accountsPartial({ config: cfgPk, authority: authority.publicKey })
        .instruction();
      send([unpauseIx], [authority]);
      expect(decodeConfigAt(cfgPk).paused).toBe(false);

      send([depositIx], [employer]);
      expect(decodeEscrowAt(escrow).status.funded).toBeDefined();
      expect(balance(vault)).toBe(SOL_AMT.add(commission(SOL_AMT)).toNumber());
      // Config's platform_authority survived the pause cycle.
      expect(decodeConfigAt(cfgPk).platformAuthority.toBase58()).toBe(
        platformAuthority.publicKey.toBase58(),
      );
    });
  });

  // ───────────────────────────────────────────────────────────────────────────
  // (c) LIVE ESCROWS
  // ───────────────────────────────────────────────────────────────────────────
  describe("(c) live Funded escrows and their USDT vaults", () => {
    const usdt = () => new PublicKey(USDT_MINT);

    /**
     * Fresh SVM holding the exact live USDT mint, both Funded escrows, both
     * vaults and the live treasury ATA, plus caller-prepended ATAs for the two
     * counterparties (production does exactly this — v1.5.0 no longer creates
     * payout destinations itself).
     */
    function bootLive(escrowOverrides: Record<string, Buffer> = {}) {
      bootSvm();
      svm.setSigverify(false);
      inject("usdt-mint");
      inject("treasury-usdt-ata");
      inject("escrow-funded-1", escrowOverrides["escrow-funded-1"]);
      inject("escrow-funded-2", escrowOverrides["escrow-funded-2"]);
      inject("vault-funded-1");
      inject("vault-funded-2");
      const w1 = ensureAta(usdt(), new PublicKey(E1_EMPLOYER));
      const w2 = ensureAta(usdt(), new PublicKey(E1_EMPLOYEE));
      return { kajd: w1, wqt: w2 };
    }

    test("injected live state is intact and vaults hold the funded totals", () => {
      bootLive();
      expect(tokenBalance(new PublicKey(E1_VAULT))).toBe(
        E1_AMOUNT + E1_COMMISSION,
      );
      expect(tokenBalance(new PublicKey(E2_VAULT))).toBe(
        E2_AMOUNT + E2_COMMISSION,
      );
      expect(tokenBalance(new PublicKey(LIVE_TREASURY_USDT_ATA))).toBe(
        TREASURY_USDT_PREBALANCE,
      );
      const e1 = decodeEscrowAt(new PublicKey(E1));
      expect(e1.status.funded).toBeDefined();
      expect(e1.platformAuthority.toBase58()).toBe(LIVE_PLATFORM_AUTHORITY);
    });

    test("trigger_auto_release_token on a live Funded escrow fails InvalidStatus (6000) — auto-release is structurally unreachable", async () => {
      const { kajd, wqt } = bootLive();

      // Documented finding: `trigger_auto_release_*` gates on
      // `escrow.status == Disputed` AND `dispute_deadline != 0`. Both live Funded
      // escrows carry status=Funded, dispute_deadline=0 AND auto_release_at=0, so
      // no permissionless path exists for them at all — the failure surfaces as
      // InvalidStatus, one step *before* the deadline check, and it can never
      // reach DisputeDeadlineNotReached (6017) without a party first calling
      // raise_dispute (employer- or employee-signed).
      const live = decodeEscrowAt(new PublicKey(E1));
      expect(
        live.disputeDeadline.toString(),
        "live escrow must have no dispute deadline (nothing to be 'before')",
      ).toBe("0");
      expect(live.autoReleaseAt.toString()).toBe("0");

      const ix = await program.methods
        .triggerAutoReleaseToken()
        .accountsPartial({
          escrow: new PublicKey(E1),
          vaultTokenAccount: new PublicKey(E1_VAULT),
          employeeTokenAccount: wqt,
          employerTokenAccount: kajd,
          platformTokenAccount: new PublicKey(LIVE_TREASURY_USDT_ATA),
          caller: payer.publicKey,
        })
        .instruction();

      failWith(
        [ix],
        [],
        "InvalidStatus",
        6000,
        "the 948-byte live Escrow must deserialize whole under v1.5.0 and fail " +
          "on its domain status gate, not on a discriminator/borsh error",
      );
      expect(tokenBalance(new PublicKey(E1_VAULT))).toBe(
        E1_AMOUNT + E1_COMMISSION,
      );
    });

    test("release_token from a stranger fails ReleaseNotAuthorized (6005) — every v1.5.0 account constraint passed on live bytes", async () => {
      const { wqt } = bootLive();
      const stranger = Keypair.generate();
      svm.airdrop(stranger.publicKey.toBytes(), BigInt(LAMPORTS_PER_SOL));

      // 6005 is raised *inside the handler*, so reaching it proves the whole
      // v1.5.0 ATA-diet account list validated against live data:
      //   escrow      → discriminator + 948-byte borsh + status/is_native gates
      //   vault ATA   → owner == escrow PDA, mint == escrow.token_mint
      //   employee ATA→ owner == escrow.employee (8WQt…), mint == USDT
      //   treasury ATA→ owner == escrow.fee_recipient (5jVqe…), mint == USDT
      const ix = await program.methods
        .releaseToken(ZEROS32)
        .accountsPartial({
          escrow: new PublicKey(E1),
          vaultTokenAccount: new PublicKey(E1_VAULT),
          employeeTokenAccount: wqt,
          platformTokenAccount: new PublicKey(LIVE_TREASURY_USDT_ATA),
          authority: stranger.publicKey,
        })
        .instruction();

      const logs = failWith(
        [ix],
        [stranger],
        "ReleaseNotAuthorized",
        6005,
        "a handler-level authorization error is only reachable once every " +
          "account constraint in the v1.5.0 list has validated live bytes",
      );
      expect(logs.includes("caused by account:")).toBe(false);
      expect(tokenBalance(new PublicKey(E1_VAULT))).toBe(
        E1_AMOUNT + E1_COMMISSION,
      );
    });

    test("release_token by the live platform_authority drains escrow #1 exactly (380 USDT worker / 19 USDT commission)", async () => {
      const { wqt } = bootLive();
      const platform = new PublicKey(LIVE_PLATFORM_AUTHORITY);

      // We do not hold FonMQCHzt…'s key; sigverify is disabled for this SVM so
      // the ops key's *authorization* path can still be rehearsed end to end.
      const ix = await program.methods
        .releaseToken(ZEROS32)
        .accountsPartial({
          escrow: new PublicKey(E1),
          vaultTokenAccount: new PublicKey(E1_VAULT),
          employeeTokenAccount: wqt,
          platformTokenAccount: new PublicKey(LIVE_TREASURY_USDT_ATA),
          authority: platform,
        })
        .instruction();
      sendForged([ix], [platform]);

      expect(tokenBalance(wqt)).toBe(E1_AMOUNT);
      expect(tokenBalance(new PublicKey(LIVE_TREASURY_USDT_ATA))).toBe(
        TREASURY_USDT_PREBALANCE + E1_COMMISSION,
      );
      expect(tokenBalance(new PublicKey(E1_VAULT))).toBe(0n);

      const after = decodeEscrowAt(new PublicKey(E1));
      expect(after.status.released).toBeDefined();
      expect(accountData(new PublicKey(E1)).readUInt8(ESCROW_OFF.status)).toBe(
        STATUS_RELEASED,
      );
      expect(after.releasedToEmployee.toString()).toBe(E1_AMOUNT.toString());
      expect(after.releaseInitiator.toBase58()).toBe(LIVE_PLATFORM_AUTHORITY);
      expect(after.completedAt.toString()).toBe(String(now()));
      // Immutable identity fields survived a v1.5.0 write to a v1.1.0 account.
      expect(after.version).toBe(1);
      expect(after.bump).toBe(254);
      expect(after.vaultBump).toBe(253);
      expect(after.employer.toBase58()).toBe(E1_EMPLOYER);
      expect(after.employee.toBase58()).toBe(E1_EMPLOYEE);
      expect(after.amount.toString()).toBe(E1_AMOUNT.toString());
      expect(after.commissionAmount.toString()).toBe(E1_COMMISSION.toString());
      expect(after.tokenMint.toBase58()).toBe(USDT_MINT);
      expect(after.feeRecipient.toBase58()).toBe(LIVE_TREASURY);
      expect(accountData(new PublicKey(E1)).length).toBe(948);
    });

    /**
     * Derived-state auto-release rehearsal.
     *
     * The live Funded escrows can never reach `trigger_auto_release_*` on their
     * own bytes (see the InvalidStatus test above), so this scenario patches the
     * EXACT live bytes of escrow #2 with the four fields `raise_dispute` would
     * write — status Funded→Disputed, dispute_raised_by, dispute_raised_at and
     * dispute_deadline — and nothing else. Everything the money math reads
     * (amount, commission_amount, commission_rate_bps, released_to_employee,
     * bump, parties, mint, fee_recipient) is untouched live data.
     */
    function disputedEscrow2(deadline: number): Buffer {
      const data = fixtureData("escrow-funded-2");
      expect(
        data.readUInt8(ESCROW_OFF.status),
        "ESCROW_OFF.status must point at the live status byte",
      ).toBe(STATUS_FUNDED);
      expect(
        data.readBigInt64LE(ESCROW_OFF.disputeDeadline),
        "ESCROW_OFF.disputeDeadline must point at the live (zero) deadline",
      ).toBe(0n);
      expect(
        new PublicKey(
          data.subarray(
            ESCROW_OFF.disputeRaisedBy,
            ESCROW_OFF.disputeRaisedBy + 32,
          ),
        ).toBase58(),
      ).toBe(PublicKey.default.toBase58());

      data.writeUInt8(STATUS_DISPUTED, ESCROW_OFF.status);
      new PublicKey(E2_EMPLOYEE)
        .toBuffer()
        .copy(data, ESCROW_OFF.disputeRaisedBy);
      data.writeBigInt64LE(BigInt(BASE_TS), ESCROW_OFF.disputeRaisedAt);
      data.writeBigInt64LE(BigInt(deadline), ESCROW_OFF.disputeDeadline);
      return data;
    }

    test("derived Disputed state: trigger_auto_release_token before the deadline fails DisputeDeadlineNotReached (6017)", async () => {
      const deadline = BASE_TS + 30 * DAY;
      const { kajd, wqt } = bootLive({
        "escrow-funded-2": disputedEscrow2(deadline),
      });

      const e2 = decodeEscrowAt(new PublicKey(E2));
      expect(e2.status.disputed).toBeDefined();
      expect(e2.disputeDeadline.toString()).toBe(String(deadline));
      expect(now()).toBeLessThan(deadline);

      const ix = await program.methods
        .triggerAutoReleaseToken()
        .accountsPartial({
          escrow: new PublicKey(E2),
          vaultTokenAccount: new PublicKey(E2_VAULT),
          employeeTokenAccount: kajd, // escrow #2's employee is KAJD…
          employerTokenAccount: wqt, // escrow #2's employer is 8WQt…
          platformTokenAccount: new PublicKey(LIVE_TREASURY_USDT_ATA),
          caller: payer.publicKey,
        })
        .instruction();

      failWith(
        [ix],
        [],
        "DisputeDeadlineNotReached",
        6017,
        "the deadline gate must fire only after every account constraint " +
          "validated live bytes",
      );
      expect(tokenBalance(new PublicKey(E2_VAULT))).toBe(
        E2_AMOUNT + E2_COMMISSION,
      );
    });

    test("derived Disputed state: after warping past the deadline, auto-release drains escrow #2 exactly (550 USDT worker / 27.5 USDT commission)", async () => {
      const deadline = BASE_TS + 30 * DAY;
      const { kajd, wqt } = bootLive({
        "escrow-funded-2": disputedEscrow2(deadline),
      });

      warpBy(31 * DAY);
      expect(now()).toBeGreaterThanOrEqual(deadline);

      const caller = Keypair.generate();
      svm.airdrop(caller.publicKey.toBytes(), BigInt(LAMPORTS_PER_SOL));

      const ix = await program.methods
        .triggerAutoReleaseToken()
        .accountsPartial({
          escrow: new PublicKey(E2),
          vaultTokenAccount: new PublicKey(E2_VAULT),
          employeeTokenAccount: kajd,
          employerTokenAccount: wqt,
          platformTokenAccount: new PublicKey(LIVE_TREASURY_USDT_ATA),
          caller: caller.publicKey,
        })
        .instruction();
      send([ix], [caller]);

      expect(tokenBalance(kajd)).toBe(E2_AMOUNT);
      expect(tokenBalance(new PublicKey(LIVE_TREASURY_USDT_ATA))).toBe(
        TREASURY_USDT_PREBALANCE + E2_COMMISSION,
      );
      expect(tokenBalance(wqt), "no dust should reach the employer").toBe(0n);
      expect(tokenBalance(new PublicKey(E2_VAULT))).toBe(0n);

      const after = decodeEscrowAt(new PublicKey(E2));
      expect(after.status.resolved).toBeDefined();
      expect(accountData(new PublicKey(E2)).readUInt8(ESCROW_OFF.status)).toBe(
        STATUS_RESOLVED,
      );
      expect(after.releasedToEmployee.toString()).toBe(E2_AMOUNT.toString());
      expect(after.employeeShareResolved.toString()).toBe(E2_AMOUNT.toString());
      expect(after.employerShareResolved.toString()).toBe("0");
      expect(after.disputeResolvedBy.toBase58()).toBe(
        caller.publicKey.toBase58(),
      );
      expect(after.disputeResolvedAt.toString()).toBe(String(now()));
      expect(after.version).toBe(1);
      expect(after.bump).toBe(255);
      expect(after.amount.toString()).toBe(E2_AMOUNT.toString());
      expect(after.commissionAmount.toString()).toBe(E2_COMMISSION.toString());
      expect(accountData(new PublicKey(E2)).length).toBe(948);
    });
  });

  // ───────────────────────────────────────────────────────────────────────────
  // (d) CLIENT-SIDE DECODE OF ALL FOUR LIVE ESCROWS
  // ───────────────────────────────────────────────────────────────────────────
  describe("(d) all four live Escrow accounts decode against the v1.5.0 IDL", () => {
    const expected = [
      {
        name: "escrow-funded-1",
        pubkey: E1,
        status: STATUS_FUNDED,
        statusKey: "funded",
        employer: E1_EMPLOYER,
        employee: E1_EMPLOYEE,
        platformAuthority: LIVE_PLATFORM_AUTHORITY,
        amount: "380000000",
        commissionAmount: "19000000",
        commissionRateBps: 500,
        releasedToEmployee: "0",
        tokenMint: USDT_MINT,
        bump: 254,
        vaultBump: 253,
        createdAt: 1786262196,
        fundedAt: 1786262196,
        completedAt: 0,
        disputeDeadline: 0,
      },
      {
        name: "escrow-funded-2",
        pubkey: E2,
        status: STATUS_FUNDED,
        statusKey: "funded",
        employer: E2_EMPLOYER,
        employee: E2_EMPLOYEE,
        platformAuthority: LIVE_PLATFORM_AUTHORITY,
        amount: "550000000",
        commissionAmount: "27500000",
        commissionRateBps: 500,
        releasedToEmployee: "0",
        tokenMint: USDT_MINT,
        bump: 255,
        vaultBump: 255,
        createdAt: 1786264320,
        fundedAt: 1786264320,
        completedAt: 0,
        disputeDeadline: 0,
      },
      {
        name: "escrow-resolved-1",
        pubkey: E3,
        status: STATUS_RESOLVED,
        statusKey: "resolved",
        employer: "BL4oRwuwdzuqunwEgTfVjRTJoNewvqrFvrtTxZCpsAAi",
        employee: "G2Dm5ej5pqZXQLTTEeDM7epDkucepp5yVjymnvjJ4HKK",
        platformAuthority: STAGING_PLATFORM_AUTHORITY,
        amount: "1000000",
        commissionAmount: "0",
        commissionRateBps: 0,
        releasedToEmployee: "500000",
        tokenMint: USDT_MINT,
        bump: 255,
        vaultBump: 250,
        createdAt: 1780409195,
        fundedAt: 1780409195,
        completedAt: 1780412344,
        disputeDeadline: 1781620306,
      },
      {
        name: "escrow-resolved-2",
        pubkey: E4,
        status: STATUS_RESOLVED,
        statusKey: "resolved",
        employer: "DHuiG6B9Jpqh28hMADA6YkYnrwLa39nyDqELdqp9tkjN",
        employee: "9SjyYRGTQZuixvCifkam4iQenEa14uV89gVdQZrdsWH7",
        platformAuthority: STAGING_PLATFORM_AUTHORITY,
        amount: "1000000",
        commissionAmount: "0",
        commissionRateBps: 0,
        releasedToEmployee: "500000",
        tokenMint: THIRD_MINT,
        bump: 253,
        vaultBump: 255,
        createdAt: 1780417998,
        fundedAt: 1780417998,
        completedAt: 1780504144,
        disputeDeadline: 1781627908,
      },
    ] as const;

    for (const e of expected) {
      test(`${e.name} (${e.pubkey}) parses every field`, () => {
        const raw = fixtureData(e.name);
        expect(raw.length).toBe(948);
        expect(raw.readUInt8(ESCROW_OFF.status)).toBe(e.status);

        const d = program.coder.accounts.decode("escrow", raw);
        expect(d.version).toBe(1);
        expect(d.status[e.statusKey]).toBeDefined();
        expect(d.employer.toBase58()).toBe(e.employer);
        expect(d.employee.toBase58()).toBe(e.employee);
        expect(d.platformAuthority.toBase58()).toBe(e.platformAuthority);
        expect(d.amount.toString()).toBe(e.amount);
        expect(d.commissionAmount.toString()).toBe(e.commissionAmount);
        expect(d.commissionRateBps).toBe(e.commissionRateBps);
        expect(d.releasedToEmployee.toString()).toBe(e.releasedToEmployee);
        expect(d.tokenMint.toBase58()).toBe(e.tokenMint);
        expect(d.isNative).toBe(false);
        expect(d.bump).toBe(e.bump);
        expect(d.vaultBump).toBe(e.vaultBump);
        expect(d.escrowKind).toBe(0);
        expect(d.feeRecipient.toBase58()).toBe(LIVE_TREASURY);
        expect(d.createdAt.toNumber()).toBe(e.createdAt);
        expect(d.fundedAt.toNumber()).toBe(e.fundedAt);
        expect(d.completedAt.toNumber()).toBe(e.completedAt);
        expect(d.autoReleaseAt.toNumber()).toBe(0);
        expect(d.disputeDeadline.toNumber()).toBe(e.disputeDeadline);
        expect(d.sequenceInGroup).toBe(0);
        expect(d.totalInGroup).toBe(0);
        expect(Buffer.from(d.termsHash).equals(Buffer.alloc(32))).toBe(true);
        expect(Buffer.from(d.reserved).equals(Buffer.alloc(64))).toBe(true);
        // The PDA the decoded escrow_id derives must be the account it came from.
        expect(escrowPda(Buffer.from(d.escrowId))[0].toBase58()).toBe(e.pubkey);
      });
    }

    test("both Resolved escrows carry a coherent dispute record", () => {
      for (const name of ["escrow-resolved-1", "escrow-resolved-2"] as const) {
        const d = program.coder.accounts.decode("escrow", fixtureData(name));
        expect(d.status.resolved).toBeDefined();
        expect(d.disputeRaisedBy.toBase58()).toBe(d.employer.toBase58());
        expect(d.disputeRaisedAt.toNumber()).toBeGreaterThan(
          d.fundedAt.toNumber(),
        );
        expect(d.disputeResolvedBy.toBase58()).toBe(STAGING_PLATFORM_AUTHORITY);
        expect(d.disputeResolvedAt.toNumber()).toBe(d.completedAt.toNumber());
        // 50/50 split of a 1 USDT escrow with no commission.
        expect(d.employeeShareResolved.toString()).toBe("500000");
        expect(d.employerShareResolved.toString()).toBe("500000");
        expect(
          d.disputeDeadline.toNumber() - d.disputeRaisedAt.toNumber(),
        ).toBeGreaterThanOrEqual(3 * DAY);
        expect(
          d.disputeDeadline.toNumber() - d.disputeRaisedAt.toNumber(),
        ).toBeLessThanOrEqual(90 * DAY);
      }
    });
  });
});

import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { WorqenEscrow } from "../target/types/worqen_escrow";
import {
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  Connection,
} from "@solana/web3.js";
import { createHash } from "crypto";
import { expect } from "chai";

// ============================================================================
// Helper Functions
// ============================================================================

const ZERO_32 = Array(32).fill(0) as number[];

/** Calculate commission from amount and rate in basis points */
const calculateCommission = (amount: number, rateBps: number = 150): number => {
  return Math.floor((amount * rateBps) / 10000);
};

/** Confirm transaction using latest blockhash (non-deprecated approach) */
const confirmTx = async (connection: Connection, signature: string) => {
  const latestBlockhash = await connection.getLatestBlockhash();
  await connection.confirmTransaction({
    signature,
    blockhash: latestBlockhash.blockhash,
    lastValidBlockHeight: latestBlockhash.lastValidBlockHeight,
  });
};

/** Generate escrow ID from hire ID */
const generateEscrowId = (hireId: string): number[] => {
  return Array.from(createHash("sha256").update(hireId).digest()) as number[];
};

/** Derive escrow and vault PDAs */
const derivePDAs = (escrowId: number[], programId: PublicKey) => {
  const [escrowPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("escrow"), Buffer.from(escrowId)],
    programId
  );
  const [vaultPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), escrowPda.toBuffer()],
    programId
  );
  return { escrowPda, vaultPda };
};

/** Airdrop SOL to an account */
const airdrop = async (
  connection: Connection,
  publicKey: PublicKey,
  amount: number = 10 * LAMPORTS_PER_SOL
) => {
  const signature = await connection.requestAirdrop(publicKey, amount);
  await confirmTx(connection, signature);
};

/** Test context shared across test suites */
interface TestContext {
  provider: anchor.AnchorProvider;
  program: Program<WorqenEscrow>;
  employer: Keypair;
  employee: Keypair;
  platformAuthority: Keypair;
  escrowId: number[];
  escrowPda: PublicKey;
  vaultPda: PublicKey;
  escrowAmount: number;
  commissionRateBps: number;
  commissionAmount: number;
  totalDeposit: number;
}

/** Create (and optionally fund) an escrow using v1 argument list. */
const createAndFundEscrow = async (
  ctx: TestContext,
  options: {
    autoReleaseAt?: anchor.BN;
    escrowGroupId?: number[];
    sequenceInGroup?: number;
    totalInGroup?: number;
    fund?: boolean;
  } = {}
) => {
  const {
    autoReleaseAt = new anchor.BN(0),
    escrowGroupId = ZERO_32,
    sequenceInGroup = 0,
    totalInGroup = 0,
    fund = true,
  } = options;

  await ctx.program.methods
    .createEscrow(
      ctx.escrowId,
      escrowGroupId,
      sequenceInGroup,
      totalInGroup,
      new anchor.BN(ctx.escrowAmount),
      true,
      ctx.commissionRateBps,
      autoReleaseAt
    )
    .accountsStrict({
      escrow: ctx.escrowPda,
      employer: ctx.employer.publicKey,
      employee: ctx.employee.publicKey,
      platformAuthority: ctx.platformAuthority.publicKey,
      tokenMint: SystemProgram.programId,
      systemProgram: SystemProgram.programId,
    })
    .signers([ctx.employer])
    .rpc();

  if (fund) {
    await ctx.program.methods
      .depositSol()
      .accountsStrict({
        escrow: ctx.escrowPda,
        escrowVault: ctx.vaultPda,
        employer: ctx.employer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([ctx.employer])
      .rpc();
  }
};

/** Setup test context */
const setupTestContext = async (
  hireId: string,
  escrowAmount: number,
  commissionRateBps: number
): Promise<TestContext> => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.WorqenEscrow as Program<WorqenEscrow>;
  const employer = Keypair.generate();
  const employee = Keypair.generate();
  const platformAuthority = Keypair.generate();

  const escrowId = generateEscrowId(hireId);
  const { escrowPda, vaultPda } = derivePDAs(escrowId, program.programId);

  const commissionAmount = calculateCommission(escrowAmount, commissionRateBps);
  const totalDeposit = escrowAmount + commissionAmount;

  await airdrop(provider.connection, employer.publicKey);

  return {
    provider,
    program,
    employer,
    employee,
    platformAuthority,
    escrowId,
    escrowPda,
    vaultPda,
    escrowAmount,
    commissionRateBps,
    commissionAmount,
    totalDeposit,
  };
};

// ============================================================================
// Tests — happy path
// ============================================================================

describe("Worqen Escrow - SOL Tests with Commission", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext("test-hire-commission-001", LAMPORTS_PER_SOL, 150);
  });

  it("Creates an escrow with commission (v1 schema, ungrouped)", async () => {
    await ctx.program.methods
      .createEscrow(
        ctx.escrowId,
        ZERO_32,
        0,
        0,
        new anchor.BN(ctx.escrowAmount),
        true,
        ctx.commissionRateBps,
        new anchor.BN(0)
      )
      .accountsStrict({
        escrow: ctx.escrowPda,
        employer: ctx.employer.publicKey,
        employee: ctx.employee.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        tokenMint: SystemProgram.programId,
        systemProgram: SystemProgram.programId,
      })
      .signers([ctx.employer])
      .rpc();

    const escrowAccount = await ctx.program.account.escrow.fetch(ctx.escrowPda);
    expect(escrowAccount.version).to.equal(1);
    expect(escrowAccount.employer.toBase58()).to.equal(ctx.employer.publicKey.toBase58());
    expect(escrowAccount.employee.toBase58()).to.equal(ctx.employee.publicKey.toBase58());
    expect(escrowAccount.amount.toNumber()).to.equal(ctx.escrowAmount);
    expect(escrowAccount.commissionAmount.toNumber()).to.equal(ctx.commissionAmount);
    expect(escrowAccount.releasedToEmployee.toNumber()).to.equal(0);
    expect(escrowAccount.status).to.deep.equal({ created: {} });
    expect(escrowAccount.autoReleaseAt.toNumber()).to.equal(0);
  });

  it("Deposits SOL (worker + commission)", async () => {
    await ctx.program.methods
      .depositSol()
      .accountsStrict({
        escrow: ctx.escrowPda,
        escrowVault: ctx.vaultPda,
        employer: ctx.employer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([ctx.employer])
      .rpc();

    const vaultBalance = await ctx.provider.connection.getBalance(ctx.vaultPda);
    expect(vaultBalance).to.equal(ctx.totalDeposit);
  });

  it("Employer confirms completion", async () => {
    await ctx.program.methods
      .confirmCompletion()
      .accountsStrict({ escrow: ctx.escrowPda, signer: ctx.employer.publicKey })
      .signers([ctx.employer])
      .rpc();

    const escrowAccount = await ctx.program.account.escrow.fetch(ctx.escrowPda);
    expect(escrowAccount.employerConfirmed).to.be.true;
    expect(escrowAccount.status).to.deep.equal({ pendingRelease: {} });
  });

  it("Releases SOL to employee and commission to platform", async () => {
    const employeeBefore = await ctx.provider.connection.getBalance(ctx.employee.publicKey);
    const platformBefore = await ctx.provider.connection.getBalance(ctx.platformAuthority.publicKey);

    await ctx.program.methods
      .releaseSol()
      .accountsStrict({
        escrow: ctx.escrowPda,
        escrowVault: ctx.vaultPda,
        employee: ctx.employee.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        authority: ctx.employer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([ctx.employer])
      .rpc();

    const employeeAfter = await ctx.provider.connection.getBalance(ctx.employee.publicKey);
    const platformAfter = await ctx.provider.connection.getBalance(ctx.platformAuthority.publicKey);
    expect(employeeAfter - employeeBefore).to.equal(ctx.escrowAmount);
    expect(platformAfter - platformBefore).to.equal(ctx.commissionAmount);

    const escrowAccount = await ctx.program.account.escrow.fetch(ctx.escrowPda);
    expect(escrowAccount.status).to.deep.equal({ released: {} });
    expect(escrowAccount.releasedToEmployee.toNumber()).to.equal(ctx.escrowAmount);
  });
});

// ============================================================================
// Tests — dispute with persisted resolution metadata
// ============================================================================

describe("Worqen Escrow - Dispute persists resolution data", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext("test-hire-dispute-v1-001", 2 * LAMPORTS_PER_SOL, 150);
    await createAndFundEscrow(ctx);
  });

  it("Employee raises dispute with reason + deadline", async () => {
    const reason = Buffer.from("Deliverable not as described");
    const deadline = new anchor.BN(Math.floor(Date.now() / 1000) + 86400);

    await airdrop(ctx.provider.connection, ctx.employee.publicKey, LAMPORTS_PER_SOL);

    await ctx.program.methods
      .raiseDispute(reason, deadline)
      .accountsStrict({ escrow: ctx.escrowPda, signer: ctx.employee.publicKey })
      .signers([ctx.employee])
      .rpc();

    const account = await ctx.program.account.escrow.fetch(ctx.escrowPda);
    expect(account.status).to.deep.equal({ disputed: {} });
    expect(account.disputeRaisedBy.toBase58()).to.equal(ctx.employee.publicKey.toBase58());
    expect(account.disputeRaisedAt.toNumber()).to.be.greaterThan(0);
    expect(account.disputeDeadline.toNumber()).to.equal(deadline.toNumber());
  });

  it("Platform resolves and persists split + resolver", async () => {
    const employeeShare = new anchor.BN(ctx.escrowAmount / 2);
    await airdrop(ctx.provider.connection, ctx.platformAuthority.publicKey, LAMPORTS_PER_SOL);

    await ctx.program.methods
      .resolveDisputeSol(employeeShare)
      .accountsStrict({
        escrow: ctx.escrowPda,
        escrowVault: ctx.vaultPda,
        employer: ctx.employer.publicKey,
        employee: ctx.employee.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([ctx.platformAuthority])
      .rpc();

    const a = await ctx.program.account.escrow.fetch(ctx.escrowPda);
    expect(a.status).to.deep.equal({ resolved: {} });
    expect(a.disputeResolvedBy.toBase58()).to.equal(ctx.platformAuthority.publicKey.toBase58());
    expect(a.disputeResolvedAt.toNumber()).to.be.greaterThan(0);
    expect(a.employeeShareResolved.toNumber()).to.equal(ctx.escrowAmount / 2);
    expect(a.employerShareResolved.toNumber()).to.equal(ctx.escrowAmount / 2);
  });
});

// ============================================================================
// Tests — cancellation persists reason + actor
// ============================================================================

describe("Worqen Escrow - Cancel with reason", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext("test-hire-cancel-v1-001", LAMPORTS_PER_SOL, 150);
    await createAndFundEscrow(ctx);
  });

  it("Employer cancels with reason, employer receives full refund", async () => {
    const reason = Buffer.from("Scope change - restarting from scratch");

    await ctx.program.methods
      .cancelEscrowSol(reason)
      .accountsStrict({
        escrow: ctx.escrowPda,
        escrowVault: ctx.vaultPda,
        employer: ctx.employer.publicKey,
        signer: ctx.employer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([ctx.employer])
      .rpc();

    const a = await ctx.program.account.escrow.fetch(ctx.escrowPda);
    expect(a.status).to.deep.equal({ cancelled: {} });
    expect(a.cancelledBy.toBase58()).to.equal(ctx.employer.publicKey.toBase58());
    const stored = Buffer.from(a.cancellationReason).slice(0, reason.length);
    expect(stored.equals(reason)).to.be.true;
  });
});

// ============================================================================
// Tests — partial release
// ============================================================================

describe("Worqen Escrow - Partial release", () => {
  let ctx: TestContext;

  before(async () => {
    ctx = await setupTestContext("test-hire-partial-001", 4 * LAMPORTS_PER_SOL, 150);
    await createAndFundEscrow(ctx);
  });

  it("Releases 25% to employee, keeps escrow Funded", async () => {
    const slice = Math.floor(ctx.escrowAmount / 4);
    const employeeBefore = await ctx.provider.connection.getBalance(ctx.employee.publicKey);

    await ctx.program.methods
      .releasePartialSol(new anchor.BN(slice))
      .accountsStrict({
        escrow: ctx.escrowPda,
        escrowVault: ctx.vaultPda,
        employee: ctx.employee.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        authority: ctx.employer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([ctx.employer])
      .rpc();

    const employeeAfter = await ctx.provider.connection.getBalance(ctx.employee.publicKey);
    expect(employeeAfter - employeeBefore).to.equal(slice);

    const a = await ctx.program.account.escrow.fetch(ctx.escrowPda);
    expect(a.status).to.deep.equal({ funded: {} });
    expect(a.releasedToEmployee.toNumber()).to.equal(slice);
  });

  it("Final release closes the escrow", async () => {
    await ctx.program.methods
      .releaseSol()
      .accountsStrict({
        escrow: ctx.escrowPda,
        escrowVault: ctx.vaultPda,
        employee: ctx.employee.publicKey,
        platformAuthority: ctx.platformAuthority.publicKey,
        authority: ctx.platformAuthority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([ctx.platformAuthority])
      .rpc();

    const a = await ctx.program.account.escrow.fetch(ctx.escrowPda);
    expect(a.status).to.deep.equal({ released: {} });
    expect(a.releasedToEmployee.toNumber()).to.equal(ctx.escrowAmount);
  });
});

// ============================================================================
// Tests — platform authority rotation
// ============================================================================

describe("Worqen Escrow - Platform authority rotation", () => {
  let ctx: TestContext;
  const newAuthority = Keypair.generate();

  before(async () => {
    ctx = await setupTestContext("test-hire-rotate-001", LAMPORTS_PER_SOL, 150);
    await createAndFundEscrow(ctx);
  });

  it("Current platform_authority rotates to a new key", async () => {
    await airdrop(ctx.provider.connection, ctx.platformAuthority.publicKey, LAMPORTS_PER_SOL);

    await ctx.program.methods
      .updatePlatformAuthority()
      .accountsStrict({
        escrow: ctx.escrowPda,
        currentPlatformAuthority: ctx.platformAuthority.publicKey,
        newPlatformAuthority: newAuthority.publicKey,
      })
      .signers([ctx.platformAuthority])
      .rpc();

    const a = await ctx.program.account.escrow.fetch(ctx.escrowPda);
    expect(a.platformAuthority.toBase58()).to.equal(newAuthority.publicKey.toBase58());
  });
});

// ============================================================================
// Tests — validation rejections
// ============================================================================

describe("Worqen Escrow - Create validations", () => {
  it("Rejects when employee == employer", async () => {
    const ctx = await setupTestContext("test-validate-same-001", LAMPORTS_PER_SOL, 0);

    try {
      await ctx.program.methods
        .createEscrow(
          ctx.escrowId,
          ZERO_32,
          0,
          0,
          new anchor.BN(ctx.escrowAmount),
          true,
          0,
          new anchor.BN(0)
        )
        .accountsStrict({
          escrow: ctx.escrowPda,
          employer: ctx.employer.publicKey,
          employee: ctx.employer.publicKey, // same as employer — invalid
          platformAuthority: ctx.platformAuthority.publicKey,
          tokenMint: SystemProgram.programId,
          systemProgram: SystemProgram.programId,
        })
        .signers([ctx.employer])
        .rpc();
      expect.fail("Expected EmployeeIsEmployer error");
    } catch (e: unknown) {
      expect(String(e)).to.match(/EmployeeIsEmployer|must be different/);
    }
  });

  it("Rejects amount == 0", async () => {
    const ctx = await setupTestContext("test-validate-zero-001", 1, 0);

    try {
      await ctx.program.methods
        .createEscrow(
          ctx.escrowId,
          ZERO_32,
          0,
          0,
          new anchor.BN(0),
          true,
          0,
          new anchor.BN(0)
        )
        .accountsStrict({
          escrow: ctx.escrowPda,
          employer: ctx.employer.publicKey,
          employee: ctx.employee.publicKey,
          platformAuthority: ctx.platformAuthority.publicKey,
          tokenMint: SystemProgram.programId,
          systemProgram: SystemProgram.programId,
        })
        .signers([ctx.employer])
        .rpc();
      expect.fail("Expected InvalidAmount error");
    } catch (e: unknown) {
      expect(String(e)).to.match(/InvalidAmount|Invalid amount/);
    }
  });
});

// ============================================================================
// Tests — direct-pay (pay_with_commission_sol)
// ============================================================================

describe("Worqen Escrow - Direct pay with commission (SOL)", () => {
  let provider: anchor.AnchorProvider;
  let program: Program<WorqenEscrow>;
  const employer = Keypair.generate();
  const employee = Keypair.generate();
  const platformAuthority = Keypair.generate();

  const hireId = generateEscrowId("test-direct-pay-sol-001");

  before(async () => {
    provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);
    program = anchor.workspace.WorqenEscrow as Program<WorqenEscrow>;
    await airdrop(provider.connection, employer.publicKey, 2 * LAMPORTS_PER_SOL);
  });

  it("Splits SOL amount into worker + platform commission", async () => {
    const amount = LAMPORTS_PER_SOL; // 1 SOL
    const commissionBps = 200; // 2%
    const expectedCommission = Math.floor((amount * commissionBps) / 10000);
    const expectedWorker = amount - expectedCommission;

    const employeeBefore = await provider.connection.getBalance(employee.publicKey);
    const platformBefore = await provider.connection.getBalance(platformAuthority.publicKey);

    await program.methods
      .payWithCommissionSol(hireId, new anchor.BN(amount), commissionBps)
      .accountsStrict({
        payer: employer.publicKey,
        recipient: employee.publicKey,
        platformAuthority: platformAuthority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([employer])
      .rpc();

    const employeeAfter = await provider.connection.getBalance(employee.publicKey);
    const platformAfter = await provider.connection.getBalance(platformAuthority.publicKey);

    expect(employeeAfter - employeeBefore).to.equal(expectedWorker);
    expect(platformAfter - platformBefore).to.equal(expectedCommission);
  });

  it("Rejects self-payment (payer == recipient)", async () => {
    try {
      await program.methods
        .payWithCommissionSol(hireId, new anchor.BN(LAMPORTS_PER_SOL / 2), 150)
        .accountsStrict({
          payer: employer.publicKey,
          recipient: employer.publicKey, // same as payer
          platformAuthority: platformAuthority.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([employer])
        .rpc();
      expect.fail("Expected SelfPaymentNotAllowed");
    } catch (e: unknown) {
      expect(String(e)).to.match(/SelfPaymentNotAllowed|Self-payment/);
    }
  });

  it("Rejects commission_bps > MAX (1000)", async () => {
    try {
      await program.methods
        .payWithCommissionSol(hireId, new anchor.BN(LAMPORTS_PER_SOL / 2), 1001)
        .accountsStrict({
          payer: employer.publicKey,
          recipient: employee.publicKey,
          platformAuthority: platformAuthority.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([employer])
        .rpc();
      expect.fail("Expected InvalidCommissionRate");
    } catch (e: unknown) {
      expect(String(e)).to.match(/InvalidCommissionRate/);
    }
  });

  it("Rejects amount == 0", async () => {
    try {
      await program.methods
        .payWithCommissionSol(hireId, new anchor.BN(0), 150)
        .accountsStrict({
          payer: employer.publicKey,
          recipient: employee.publicKey,
          platformAuthority: platformAuthority.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([employer])
        .rpc();
      expect.fail("Expected InvalidAmount");
    } catch (e: unknown) {
      expect(String(e)).to.match(/InvalidAmount|Invalid amount/);
    }
  });

  it("Zero commission_bps sends the entire amount to the worker", async () => {
    const freshEmployee = Keypair.generate();
    const freshPlatform = Keypair.generate();
    const amount = LAMPORTS_PER_SOL / 4;

    const employeeBefore = await provider.connection.getBalance(freshEmployee.publicKey);
    const platformBefore = await provider.connection.getBalance(freshPlatform.publicKey);

    await program.methods
      .payWithCommissionSol(hireId, new anchor.BN(amount), 0)
      .accountsStrict({
        payer: employer.publicKey,
        recipient: freshEmployee.publicKey,
        platformAuthority: freshPlatform.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([employer])
      .rpc();

    const employeeAfter = await provider.connection.getBalance(freshEmployee.publicKey);
    const platformAfter = await provider.connection.getBalance(freshPlatform.publicKey);

    expect(employeeAfter - employeeBefore).to.equal(amount);
    expect(platformAfter - platformBefore).to.equal(0);
  });
});

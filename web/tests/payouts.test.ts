// Payout flow through the real route handlers, the real payroll-api and
// PostgreSQL, against the MOCK payout wallet (tests/support/mock-wallet.ts).
// No keys, chain or funds are involved here; see docs/PHASE3_PAYOUTS.md for
// the live testnet run.
import { beforeAll, describe, expect, it } from "vitest";
import { payroll, type ClaimView } from "@/lib/payroll";
import * as payoutRoute from "@/app/api/claims/[claimId]/payout/route";
import * as quoteRoute from "@/app/api/claims/[claimId]/payout/quote/route";
import * as releaseRoute from "@/app/api/claims/[claimId]/release/route";
import { call, claim, createOrg, fastStream, join, lifecycle, newUser, sleep, type TestUser } from "./support/helpers";

const ADDR = "utest1mockcontributoraddressaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER = "utest1mockotheraddressbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

let owner: TestUser, worker: TestUser, outsider: TestUser;
let org: string;

const mock = async (path: string, body: unknown = {}) =>
  (await fetch(`${process.env.MOCK_WALLET_URL}/__mock/${path}`, { method: "POST", body: JSON.stringify(body) })).json() as Promise<{
    txs_built: number;
    submits: number;
    tip: number;
  }>;

beforeAll(async () => {
  [owner, worker, outsider] = await Promise.all([newUser("pOwner", "Payout Owner"), newUser("pWorker", "Payout Worker"), newUser("pOut", "Outsider")]);
  org = await createOrg(owner, "Payout Co");
  await join(owner, org, worker);
  await createOrg(outsider, "Other Co");
});

async function reservedClaim(zec = "0.1"): Promise<{ streamId: string; claimId: string }> {
  const s = await fastStream(owner, org, worker);
  await sleep(1100);
  await lifecycle(owner, s.id, "pause");
  const r = await claim(worker, s.id, zec);
  expect(r.status).toBe(201);
  return { streamId: s.id, claimId: (r.body.claim as { id: string }).id };
}

const pay = (u: TestUser, claimId: string, address = ADDR, opts: { origin?: string | null } = {}) =>
  call(payoutRoute.POST, "POST", { claimId }, { cookie: u.cookie, body: { address }, ...opts });

async function claimOf(streamId: string, claimId: string): Promise<ClaimView> {
  const d = await payroll.stream(worker.id, streamId);
  return d.claims.find((c) => c.id === claimId)!;
}

async function waitFor(streamId: string, claimId: string, ok: (c: ClaimView) => boolean, ms = 15_000): Promise<ClaimView> {
  const until = Date.now() + ms;
  for (;;) {
    const c = await claimOf(streamId, claimId);
    if (ok(c)) return c;
    if (Date.now() > until) throw new Error(`timed out; claim=${JSON.stringify(c)}`);
    await sleep(250);
  }
}

describe("paying out a claim", () => {
  it("quote → send → submitted (still unpaid) → confirmations → paid", async () => {
    const { streamId, claimId } = await reservedClaim("0.1");
    const q = await call(quoteRoute.POST, "POST", { claimId }, { cookie: worker.cookie, body: { address: ADDR } });
    expect(q.status).toBe(200);
    expect(q.body).toMatchObject({ amount_zats: 10_000_000, fee_zats: 10_000, valid_recipient: true, sufficient: true, network: "testnet", required_confirmations: 3 });

    const before = (await mock("stats")).txs_built;
    const r = await pay(worker, claimId);
    expect(r.status).toBe(201);
    const p = r.body.payout as { state: string; txid: string; amount_zats: number };
    expect(p).toMatchObject({ state: "submitted", amount_zats: 10_000_000 });
    expect(p.txid).toMatch(/^[0-9a-f]{64}$/);
    expect(JSON.stringify(r.body)).not.toContain(ADDR); // the address is not echoed or stored

    // Broadcast is not payment.
    let c = await claimOf(streamId, claimId);
    expect(c.status).toBe("reserved");
    let b = (await payroll.stream(worker.id, streamId)).stream.balance;
    expect(b.claimed_zats).toBe(0);

    await mock("mine");
    await mock("advance", { blocks: 1 }); // 2 confirmations < 3
    c = await waitFor(streamId, claimId, (c) => c.payout!.confirmations === 2);
    expect([c.status, c.payout!.state]).toEqual(["reserved", "submitted"]);
    await mock("advance", { blocks: 1 });
    c = await waitFor(streamId, claimId, (c) => c.status === "settled");
    expect(c.payout).toMatchObject({ state: "confirmed", confirmations: 3, txid: p.txid });
    expect(c.settlement_ref).toBe(p.txid);
    b = (await payroll.stream(worker.id, streamId)).stream.balance;
    expect([b.reserved_zats, b.claimed_zats]).toEqual([0, 10_000_000]);
    expect((await mock("stats")).txs_built - before).toBe(1);
  });

  it("the employer sees the payout too, and the payout wallet summary", async () => {
    const { streamId, claimId } = await reservedClaim("0.01");
    await pay(worker, claimId);
    const seen = (await payroll.stream(owner.id, streamId)).claims.find((c) => c.id === claimId)!;
    expect(seen.payout?.state).toBe("submitted");
    const w = await payroll.payoutWallet(owner.id, org);
    expect(w).toMatchObject({ network: "testnet", funding_address: "utest1mockfundingaddress", required_confirmations: 3 });
    await expect(payroll.payoutWallet(worker.id, org)).rejects.toMatchObject({ status: 403 });
    await expect(payroll.payoutWallet(outsider.id, org)).rejects.toMatchObject({ status: 404 });
  });
});

describe("authorization", () => {
  it("only the claimant can quote or pay; others are refused", async () => {
    const { claimId } = await reservedClaim("0.01");
    expect((await pay(owner, claimId)).status).toBe(403); // employer can't redirect wages
    expect((await pay(outsider, claimId)).status).toBe(404); // outsiders can't tell it exists
    expect((await call(quoteRoute.POST, "POST", { claimId }, { cookie: owner.cookie, body: { address: ADDR } })).status).toBe(403);
    expect((await call(payoutRoute.POST, "POST", { claimId }, { body: { address: ADDR } })).status).toBe(401);
    expect((await pay(worker, claimId, ADDR, { origin: "https://evil.example" })).status).toBe(403);
    expect((await pay(worker, "00000000-0000-0000-0000-000000000000")).status).toBe(404);
  });
});

describe("recipient validation", () => {
  it("rejects malformed and non-payable addresses without creating a payout", async () => {
    const { streamId, claimId } = await reservedClaim("0.01");
    for (const a of ["", "short", "UTEST1UPPERCASE00000000", "utest1 with spaces 000000", "t1".padEnd(40, "x") + "!"]) {
      expect((await pay(worker, claimId, a)).status, a).toBe(422);
    }
    const q = await call(quoteRoute.POST, "POST", { claimId }, { cookie: worker.cookie, body: { address: "utest1saplingonlyaddressxxxxxxxxxx" } });
    expect(q.body).toMatchObject({ valid_recipient: false });
    const r = await pay(worker, claimId, "utest1saplingonlyaddressxxxxxxxxxx");
    expect([r.status, r.body.error]).toEqual([422, "invalid_recipient"]);
    expect((await claimOf(streamId, claimId)).payout).toBeNull();
  });
});

describe("fee and balance checks", () => {
  it("refuses when the payout wallet can't cover amount + fee; the claim stays reserved", async () => {
    const { streamId, claimId } = await reservedClaim("0.01");
    await mock("spendable", { zats: 1_000_005 }); // 0.01 ZEC + fee needs 1,010,000
    const q = await call(quoteRoute.POST, "POST", { claimId }, { cookie: worker.cookie, body: { address: ADDR } });
    expect(q.body).toMatchObject({ sufficient: false, valid_recipient: true });
    const r = await pay(worker, claimId);
    expect([r.status, r.body.error]).toEqual([409, "insufficient_wallet_balance"]);
    const c = await claimOf(streamId, claimId);
    expect([c.status, c.payout]).toEqual(["reserved", null]);
    await mock("spendable", { zats: 5_000_000_000 });
    expect((await pay(worker, claimId)).status).toBe(201);
  });
});

describe("duplicate and concurrent requests", () => {
  it("repeats return the same payout; a different address is refused", async () => {
    const { claimId } = await reservedClaim("0.01");
    const a = await pay(worker, claimId);
    const b = await pay(worker, claimId);
    expect([a.status, b.status]).toEqual([201, 200]);
    expect(b.body.duplicate).toBe(true);
    expect((b.body.payout as { id: string }).id).toBe((a.body.payout as { id: string }).id);
    const c = await pay(worker, claimId, OTHER);
    expect([c.status, c.body.error]).toEqual([409, "payout_conflict"]);
  });

  it("only the claimant can repeat a request (no address probing by managers)", async () => {
    const { claimId } = await reservedClaim("0.01");
    await pay(worker, claimId);
    // Same answer whether or not the owner's guess is the real address.
    const right = await pay(owner, claimId, ADDR);
    const wrong = await pay(owner, claimId, OTHER);
    expect([right.status, wrong.status]).toEqual([403, 403]);
    expect(right.body).toEqual(wrong.body);
    expect((await pay(outsider, claimId, ADDR)).status).toBe(404);
  });

  it("10 concurrent pay requests for one claim build exactly one transaction", async () => {
    const { streamId, claimId } = await reservedClaim("0.02");
    const before = await mock("stats");
    const rs = await Promise.all(Array.from({ length: 10 }, () => pay(worker, claimId)));
    expect(rs.filter((r) => r.status === 201)).toHaveLength(1);
    expect(rs.filter((r) => r.status === 200 && r.body.duplicate === true)).toHaveLength(9);
    expect(new Set(rs.map((r) => (r.body.payout as { id: string }).id)).size).toBe(1);
    const after = await mock("stats");
    expect(after.txs_built - before.txs_built).toBe(1);
    expect(after.submits - before.submits).toBe(1);
    expect((await payroll.stream(worker.id, streamId)).claims).toHaveLength(1);
  });
});

describe("failures and recovery", () => {
  it("a lost wallet response needs reconciliation, then resolves without paying twice", async () => {
    const { streamId, claimId } = await reservedClaim("0.01");
    const before = await mock("stats");
    await mock("next", { mode: "lost_response" });
    const r = await pay(worker, claimId);
    expect(r.status).toBe(201);
    expect((r.body.payout as { state: string }).state).toBe("requires_reconciliation");
    let c = await waitFor(streamId, claimId, (c) => c.payout?.state === "submitted");
    expect(c.status).toBe("reserved");
    await mock("mine");
    await mock("advance", { blocks: 2 });
    c = await waitFor(streamId, claimId, (c) => c.status === "settled");
    expect(c.payout?.state).toBe("confirmed");
    expect((await mock("stats")).txs_built - before.txs_built).toBe(1);
  });

  it("a definite failure releases the claim so the amount can be claimed again", async () => {
    const { streamId, claimId } = await reservedClaim("0.01");
    const avail = (await payroll.stream(worker.id, streamId)).stream.balance.available_zats;
    await mock("next", { mode: "fail" });
    const r = await pay(worker, claimId);
    const p = r.body.payout as { state: string; detail: string };
    expect(p.state).toBe("failed");
    expect(p.detail).toContain("Insufficient");
    const c = await claimOf(streamId, claimId);
    expect(c.status).toBe("released");
    expect((await payroll.stream(worker.id, streamId)).stream.balance.available_zats).toBe(avail + 1_000_000);
    // Retrying the same claim returns the failed payout; it can't pay.
    expect(((await pay(worker, claimId)).body.payout as { state: string }).state).toBe("failed");
  });

  it("a claim with a payout can't be cancelled", async () => {
    const { claimId } = await reservedClaim("0.01");
    await pay(worker, claimId);
    const rel = await call(releaseRoute.POST, "POST", { claimId }, { cookie: worker.cookie });
    expect(rel.status).toBe(409);
    const relOwner = await call(releaseRoute.POST, "POST", { claimId }, { cookie: owner.cookie });
    expect(relOwner.status).toBe(409);
  });
});

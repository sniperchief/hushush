import { randomUUID } from "node:crypto";
import { beforeAll, describe, expect, it } from "vitest";
import { payroll } from "@/lib/payroll";
import * as releaseRoute from "@/app/api/claims/[claimId]/release/route";
import { call, claim, createOrg, fastStream, join, lifecycle, newUser, sleep, type TestUser } from "./support/helpers";

let owner: TestUser, worker: TestUser;
let org: string;

beforeAll(async () => {
  [owner, worker] = await Promise.all([newUser("cOwner", "Claims Owner"), newUser("cWorker", "Claims Worker")]);
  org = await createOrg(owner, "Claims Co");
  await join(owner, org, worker);
});

/** A paused stream with a fixed, known earned amount (so balances are exact). */
async function frozenStream() {
  const s = await fastStream(owner, org, worker);
  await sleep(1100);
  await lifecycle(owner, s.id, "pause");
  const b = (await payroll.stream(worker.id, s.id)).stream.balance;
  return { id: s.id, earned: b.earned_zats, earnedZec: b.earned_zec };
}

const balance = async (id: string) => (await payroll.stream(worker.id, id)).stream.balance;

describe("claim reservations", () => {
  it("partial claims reserve funds and are labelled pending/unpaid", async () => {
    const s = await frozenStream();
    const r = await claim(worker, s.id, "0.25");
    expect(r.status).toBe(201);
    const c = r.body.claim as { status: string; amount_zats: number; settlement_ref: string | null };
    expect(c).toMatchObject({ status: "reserved", amount_zats: 25_000_000, settlement_ref: null });
    const b = await balance(s.id);
    expect(b).toMatchObject({ earned_zats: s.earned, reserved_zats: 25_000_000, claimed_zats: 0, available_zats: s.earned - 25_000_000 });
    expect(b.claimed_zec).toBe("0.00000000"); // nothing is ever "paid" without a payout
  });

  it("a full claim reserves exactly the available amount", async () => {
    const s = await frozenStream();
    const b0 = await balance(s.id);
    expect((await claim(worker, s.id, b0.available_zec)).status).toBe(201);
    const b = await balance(s.id);
    expect(b.available_zats).toBe(0);
    expect(b.reserved_zats).toBe(s.earned);
  });

  it("rejects claims above the available balance", async () => {
    const s = await frozenStream();
    const r = await claim(worker, s.id, "999999");
    expect(r.status).toBe(409);
    expect(r.body.error).toBe("insufficient_available");
    expect((await balance(s.id)).reserved_zats).toBe(0);
  });

  it("rejects malformed amounts", async () => {
    const s = await frozenStream();
    for (const a of ["0", "-1", "0.000000001", "abc", ""]) {
      expect((await claim(worker, s.id, a)).status, a).toBe(422);
    }
  });

  it("releasing a reservation makes the funds available again, once", async () => {
    const s = await frozenStream();
    const r = await claim(worker, s.id, "0.5");
    const id = (r.body.claim as { id: string }).id;
    const rel = await call(releaseRoute.POST, "POST", { claimId: id }, { cookie: worker.cookie });
    expect(rel.status).toBe(200);
    expect(rel.body.status).toBe("released");
    expect((await balance(s.id)).available_zats).toBe(s.earned);
    expect((await call(releaseRoute.POST, "POST", { claimId: id }, { cookie: worker.cookie })).status).toBe(409);
  });

  it("claims stay possible on paused and ended streams", async () => {
    const s = await frozenStream();
    await lifecycle(owner, s.id, "end");
    expect((await claim(worker, s.id, "0.1")).status).toBe(201);
  });
});

describe("duplicate requests", () => {
  it("the same idempotency key returns the original claim", async () => {
    const s = await frozenStream();
    const key = randomUUID();
    const a = await claim(worker, s.id, "0.2", key);
    const b = await claim(worker, s.id, "0.2", key);
    expect([a.status, b.status]).toEqual([201, 200]);
    expect(b.body.duplicate).toBe(true);
    expect((b.body.claim as { id: string }).id).toBe((a.body.claim as { id: string }).id);
    expect((await balance(s.id)).reserved_zats).toBe(20_000_000);
  });

  it("reusing a key for a different amount is rejected", async () => {
    const s = await frozenStream();
    const key = randomUUID();
    await claim(worker, s.id, "0.2", key);
    const r = await claim(worker, s.id, "0.3", key);
    expect(r.status).toBe(409);
    expect(r.body.error).toBe("idempotency_conflict");
  });

  it("concurrent retries of one request create exactly one claim", async () => {
    const s = await frozenStream();
    const key = randomUUID();
    const results = await Promise.all(Array.from({ length: 12 }, () => claim(worker, s.id, "0.3", key)));
    expect(results.filter((r) => r.status === 201)).toHaveLength(1);
    expect(results.filter((r) => r.status === 200 && r.body.duplicate === true)).toHaveLength(11);
    const ids = new Set(results.map((r) => (r.body.claim as { id: string }).id));
    expect(ids.size).toBe(1);
    expect((await payroll.stream(worker.id, s.id)).claims).toHaveLength(1);
  });
});

describe("concurrent claims", () => {
  it("parallel distinct claims can never reserve more than earned", async () => {
    const s = await frozenStream();
    // Each claim is ~40% of earned: at most two can fit.
    const portion = Math.floor(s.earned * 0.4);
    const zec = `${Math.floor(portion / 1e8)}.${String(portion % 1e8).padStart(8, "0")}`;
    const results = await Promise.all(Array.from({ length: 10 }, () => claim(worker, s.id, zec)));
    expect(results.filter((r) => r.status === 201)).toHaveLength(2);
    expect(results.filter((r) => r.status === 409 && r.body.error === "insufficient_available")).toHaveLength(8);
    const b = await balance(s.id);
    expect(b.reserved_zats).toBe(2 * portion);
    expect(b.reserved_zats).toBeLessThanOrEqual(b.earned_zats);
  });
});

describe("no payouts in this phase", () => {
  it("payroll-api exposes no way to settle (mark paid) a claim", async () => {
    const s = await frozenStream();
    const r = await claim(worker, s.id, "0.1");
    const id = (r.body.claim as { id: string }).id;
    const res = await fetch(`${process.env.PAYROLL_API_URL}/v1/claims/${id}/settle`, {
      method: "POST",
      headers: { authorization: `Bearer ${process.env.PAYROLL_API_TOKEN}`, "x-actor-id": owner.id, "content-type": "application/json" },
      body: JSON.stringify({ settlement_ref: "fake-txid" }),
    });
    expect(res.status).toBe(404);
    expect((await balance(s.id)).claimed_zats).toBe(0);
  });

  it("payroll-api rejects calls without the service token", async () => {
    const res = await fetch(`${process.env.PAYROLL_API_URL}/v1/me/streams`, { headers: { "x-actor-id": worker.id } });
    expect(res.status).toBe(401);
  });
});

// The Phase 5 flow through the real route handlers, the real payroll-api and
// PostgreSQL, against the MOCK payout wallet (tests/support/mock-wallet.ts):
// no keys, chain or funds are involved here. Accounts use a SOFTWARE passkey.
import { randomUUID } from "node:crypto";
import { beforeAll, describe, expect, it } from "vitest";
import * as streamsRoute from "@/app/api/streams/route";
import * as relinkRoute from "@/app/api/offers/[offerId]/link/route";
import * as cancelRoute from "@/app/api/offers/[offerId]/cancel/route";
import * as endRoute from "@/app/api/streams/[streamId]/end/route";
import * as streamRoute from "@/app/api/streams/[streamId]/route";
import { hashToken } from "@/lib/crypto";
import { db } from "@/lib/db";
import { PayrollApiError, payroll, type FlowStream } from "@/lib/payroll";
import {
  boundStream,
  call,
  createStream,
  flow,
  join,
  mock,
  newEmployer,
  newWorker,
  setAddress,
  signUp,
  sleep,
  waitFor,
  withdraw,
  type TestUser,
} from "./support/helpers";

const ADDR = "utest1mockworkeraddressaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER = "utest1mockotheraddressbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SAPLING = "utest1saplingonlyccccccccccccccccccccccccccccccccccccccccccc"; // refused by the mock's rule

let employer: TestUser & { companyId: string };
let rival: TestUser & { companyId: string };
let worker: TestUser;
let other: TestUser;

beforeAll(async () => {
  [employer, rival, worker, other] = await Promise.all([
    newEmployer("Erin", "Acme Testnet"),
    newEmployer("Rex", "Rival Co"),
    newWorker("Wendy"),
    newWorker("Otto"),
  ]);
});

const now = () => Math.floor(Date.now() / 1000);

/** A bound stream with its address locked. */
async function readyStream(totalZec = "0.001", seconds = 10): Promise<string> {
  const id = await boundStream(employer, worker, totalZec, seconds);
  expect((await setAddress(worker, id, ADDR)).status).toBe(200);
  return id;
}

const withdrawable = (id: string) => waitFor(() => flow(worker, id), (s) => s.withdrawable_zats !== null);

async function confirm(id: string, payoutId: string): Promise<FlowStream> {
  await mock("mine");
  await mock("advance", { blocks: 5 });
  return waitFor(
    () => flow(worker, id),
    (s) => s.payouts.find((p) => p.id === payoutId)?.state === "confirmed",
  );
}

describe("worker link", () => {
  it("an employer creates a stream and gets a single-use link; only its hash is stored", async () => {
    const c = await createStream(employer, "1.5", 3600, { workerName: "Wendy" });
    expect(c.link).toMatch(/^http:\/\/localhost:3000\/join\/[A-Za-z0-9_-]{43}$/);
    const { rows } = await db().query("SELECT link_hash FROM stream_offers WHERE id = $1", [c.offerId]);
    expect(rows[0].link_hash).toBe(hashToken(c.token));
    const home = await payroll.employerHome(employer.id, employer.companyId);
    expect(home.offers.find((o) => o.id === c.offerId)).toMatchObject({ status: "open", worker_name: "Wendy", total_zats: 150_000_000 });
    expect(JSON.stringify(home)).not.toContain(c.token);
  });

  it("the preview before sign-in shows only the terms", async () => {
    const c = await createStream(employer, "0.25", 600, { workerName: "Preview" });
    const p = await payroll.linkPreview(hashToken(c.token));
    expect(Object.keys(p).sort()).toEqual(["end_at", "organization_name", "start_at", "status", "total_zats", "total_zec", "worker_name"]);
    expect(p).toMatchObject({ organization_name: "Acme Testnet", worker_name: "Preview", total_zec: "0.25000000", status: "open" });
  });

  it("the worker who joins gets the stream; the link then works for nobody else", async () => {
    const c = await createStream(employer, "0.001", 3600);
    const j = await join(worker, c.token);
    expect(j.status).toBe(201);
    const id = j.body.streamId as string;
    // Repeating is harmless; anyone else is refused.
    const again = await join(worker, c.token);
    expect(again.status).toBe(200);
    expect(again.body).toMatchObject({ streamId: id, duplicate: true });
    expect((await join(other, c.token)).status).toBe(409);
    const asEmployer = await join(employer, c.token);
    expect(asEmployer.status).toBe(403);
    expect(asEmployer.body.error).toBe("wrong_account");
    expect((await join(worker, "not-a-token")).status).toBe(404);
    expect((await join(worker, "A".repeat(43))).status).toBe(404);

    const s = await flow(worker, id);
    expect(s).toMatchObject({ organization_name: "Acme Testnet", worker_name: "Wendy", total_zats: 100_000, status: "active", address_locked: false });
    expect((await payroll.workerStreams(worker.id)).some((x) => x.id === id)).toBe(true);
    expect((await payroll.workerStreams(other.id)).some((x) => x.id === id)).toBe(false);
  });

  it("before anyone joins, the employer can issue a new link or cancel", async () => {
    const c = await createStream(employer, "0.001", 3600);
    const r = await call(relinkRoute.POST, "POST", { offerId: c.offerId }, { cookie: employer.cookie });
    expect(r.status).toBe(200);
    const fresh = (r.body.link as string).split("/join/")[1]!;
    expect((await join(worker, c.token)).status).toBe(404); // old link is dead
    expect((await call(relinkRoute.POST, "POST", { offerId: c.offerId }, { cookie: rival.cookie })).status).toBe(404);

    const x = await call(cancelRoute.POST, "POST", { offerId: c.offerId }, { cookie: employer.cookie });
    expect(x.status).toBe(200);
    expect(x.body.status).toBe("cancelled");
    expect((await join(worker, fresh)).status).toBe(409);

    // A joined stream can't be cancelled (only ended).
    const d = await createStream(employer, "0.001", 3600);
    await join(worker, d.token);
    expect((await call(cancelRoute.POST, "POST", { offerId: d.offerId }, { cookie: employer.cookie })).status).toBe(409);
  });

  it("validates the terms", async () => {
    const bad = async (body: Record<string, unknown>) =>
      (await call(streamsRoute.POST, "POST", {}, { cookie: employer.cookie, body: { workerName: "W", totalZec: "1", startAt: null, endAt: now() + 60, ...body } })).status;
    expect(await bad({ startAt: now() - 3600 })).toBe(422); // backdated
    expect(await bad({ endAt: now() - 1 })).toBe(422); // end before start
    expect(await bad({ totalZec: "0" })).toBe(422);
    expect(await bad({ totalZec: "1.123456789" })).toBe(422); // > 8 decimals
    expect(await bad({ totalZec: "-1" })).toBe(422);
    expect(await bad({ workerName: " " })).toBe(422);
    expect(await bad({ endAt: now() + 101 * 365 * 86_400 })).toBe(422);
    const noCompany = await signUp("employer", "No Company Yet");
    const r = await call(streamsRoute.POST, "POST", {}, { cookie: noCompany.cookie, body: { workerName: "W", totalZec: "1", startAt: null, endAt: now() + 60 } });
    expect(r.status).toBe(409);
  });

  it("the total is reached exactly at the end date", async () => {
    const id = await boundStream(employer, worker, "0.00100003", 3); // not divisible by 3 s
    const s = await waitFor(() => flow(worker, id), (x) => x.status === "ended", 8_000);
    expect(s.balance.earned_zats).toBe(100_003);
  });

  it("a future start accrues nothing until it starts", async () => {
    const c = await createStream(employer, "0.001", 60, { startAt: now() + 3600 });
    const id = (await join(worker, c.token)).body.streamId as string;
    await sleep(1100);
    expect((await flow(worker, id)).balance.earned_zats).toBe(0);
  });
});

describe("payout address", () => {
  it("only the worker sets it, it is validated, and then it is locked for good", async () => {
    const id = await boundStream(employer, worker, "0.001", 3600);
    const byEmployer = await setAddress(employer, id, ADDR);
    expect(byEmployer.status).toBe(403); // also: no address probing by the employer
    expect((await setAddress(other, id, ADDR)).status).toBe(404);
    expect((await setAddress(worker, id, "not an address")).status).toBe(422);
    const sapling = await setAddress(worker, id, SAPLING);
    expect(sapling.status).toBe(422);
    expect(sapling.body.error).toBe("invalid_recipient");
    expect((await flow(worker, id)).address_locked).toBe(false);

    const ok = await setAddress(worker, id, ADDR);
    expect(ok.status).toBe(200);
    expect(ok.body).toMatchObject({ address_locked: true, address: ADDR });
    expect((await setAddress(worker, id, ADDR)).status).toBe(200); // same again: no-op
    const change = await setAddress(worker, id, OTHER);
    expect(change.status).toBe(409);
    expect((await flow(worker, id)).address).toBe(ADDR);
  });

  it("is stored sealed, and never shown to the employer", async () => {
    const id = await readyStream("0.001", 3600);
    const { rows } = await db().query("SELECT address_sealed, address_fingerprint FROM stream_payout_addresses WHERE stream_id = $1", [id]);
    expect(rows[0].address_sealed).toMatch(/^v1\.[0-9a-f]+$/);
    expect(rows[0].address_sealed).not.toContain(ADDR);
    expect(rows[0].address_fingerprint).toMatch(/^[0-9a-f]{64}$/);
    const asEmployer = await payroll.flowStream(employer.id, id);
    expect(asEmployer.address_locked).toBe(true);
    expect(asEmployer.address).toBeNull();
    expect(JSON.stringify(await payroll.employerHome(employer.id, employer.companyId))).not.toContain(ADDR);
  });
});

describe("withdraw", () => {
  it("unlocks at the 20% milestone, pays everything available, and is paid only once confirmed", async () => {
    const id = await readyStream("0.001", 20); // milestone: 20,000 zats = 4 s
    const early = await withdraw(worker, id);
    expect(early.status).toBe(409);
    expect((await flow(worker, id)).balance.reserved_zats).toBe(0);

    const before = (await mock("stats")).txs_built;
    const ready = await withdrawable(id);
    expect(ready.withdrawable_zats).toBeGreaterThanOrEqual(20_000);
    const r = await withdraw(worker, id);
    expect(r.status).toBe(201);
    const p = r.body.payout as { id: string; state: string; amount_zats: number; txid: string };
    expect(p.state).toBe("submitted");
    expect(p.amount_zats).toBeGreaterThanOrEqual(20_000);
    expect(p.txid).toMatch(/^[0-9a-f]{64}$/);
    expect(JSON.stringify(r.body)).not.toContain(ADDR);
    expect((await mock("stats")).txs_built).toBe(before + 1);

    // Broadcast is not payment.
    let s = await flow(worker, id);
    expect(s.balance.claimed_zats).toBe(0);
    expect(s.balance.reserved_zats).toBe(p.amount_zats);
    expect(s.withdrawable_zats).toBeNull(); // one withdrawal at a time

    s = await confirm(id, p.id);
    expect(s.balance.claimed_zats).toBe(p.amount_zats);
    expect(s.payouts[0]!.state).toBe("confirmed");
    expect(s.payouts[0]!.confirmations).toBeGreaterThanOrEqual(s.payouts[0]!.required_confirmations);
  });

  it("after the stream ends, any remainder can be withdrawn, even below 20%", async () => {
    const id = await readyStream("0.001", 30); // ~3,333 zats/s; milestone 20,000 at 6 s
    await withdrawable(id);
    const first = (await withdraw(worker, id)).body.payout as { id: string; amount_zats: number };
    await confirm(id, first.id);
    await sleep(1100); // earn a little more, well under a milestone
    const ended = await call(endRoute.POST, "POST", { streamId: id }, { cookie: employer.cookie });
    expect(ended.status).toBe(200);
    const s = await flow(worker, id);
    expect(s.status).toBe("ended");
    expect(s.balance.available_zats).toBeGreaterThan(0);
    expect(s.balance.available_zats).toBeLessThan(20_000);
    expect(s.withdrawable_zats).toBe(s.balance.available_zats);
    const r = await withdraw(worker, id);
    expect(r.status).toBe(201);
    expect((r.body.payout as { amount_zats: number }).amount_zats).toBe(s.balance.available_zats);
  });

  it("double taps, retries and refreshes pay once", async () => {
    const id = await readyStream("0.001", 10);
    await withdrawable(id);
    const before = (await mock("stats")).txs_built;
    const key = randomUUID();
    // 6 taps with one key + 6 "refreshes" with new keys, all at once.
    const all = await Promise.all([
      ...Array.from({ length: 6 }, () => withdraw(worker, id, key)),
      ...Array.from({ length: 6 }, () => withdraw(worker, id)),
    ]);
    expect(all.every((r) => r.status === 200 || r.status === 201)).toBe(true);
    expect(all.filter((r) => r.status === 201)).toHaveLength(1);
    const ids = new Set(all.map((r) => (r.body.payout as { id: string }).id));
    expect(ids.size).toBe(1);
    expect((await mock("stats")).txs_built).toBe(before + 1);
    const s = await flow(worker, id);
    expect(s.payouts).toHaveLength(1);
  });

  it("a definite failure sends nothing and returns the amount; the worker can withdraw again", async () => {
    const id = await readyStream("0.001", 10);
    await withdrawable(id);
    await mock("next", { mode: "fail" });
    const r = await withdraw(worker, id);
    expect((r.body.payout as { state: string }).state).toBe("failed");
    const s = await flow(worker, id);
    expect(s.balance.reserved_zats).toBe(0);
    expect(s.withdrawable_zats).not.toBeNull();
    const again = await withdraw(worker, id);
    expect(again.status).toBe(201);
    expect((again.body.payout as { state: string }).state).toBe("submitted");
  });

  it("a lost wallet response is reconciled without paying twice", async () => {
    const id = await readyStream("0.001", 10);
    await withdrawable(id);
    const before = (await mock("stats")).txs_built;
    await mock("next", { mode: "lost_response" });
    const r = await withdraw(worker, id);
    expect((r.body.payout as { state: string }).state).toBe("requires_reconciliation");
    // A retry while it is unresolved returns the same withdrawal.
    const retry = await withdraw(worker, id);
    expect(retry.status).toBe(200);
    expect((retry.body.payout as { id: string }).id).toBe((r.body.payout as { id: string }).id);
    const s = await waitFor(() => flow(worker, id), (x) => x.payouts[0]?.state === "submitted");
    expect(s.payouts).toHaveLength(1);
    expect((await mock("stats")).txs_built).toBe(before + 1);
  });

  it("refuses when the treasury can't cover amount + fee; nothing is reserved", async () => {
    const id = await readyStream("0.001", 10);
    await withdrawable(id);
    await mock("spendable", { zats: 1_000 });
    try {
      const r = await withdraw(worker, id);
      expect(r.status).toBe(409);
      expect(r.body.error).toBe("insufficient_wallet_balance");
      expect((await flow(worker, id)).balance.reserved_zats).toBe(0);
    } finally {
      await mock("spendable", { zats: 5_000_000_000 });
    }
  });

  it("only the worker can withdraw, and only with a locked address", async () => {
    const id = await boundStream(employer, worker, "0.001", 10);
    const noAddress = await withdraw(worker, id);
    expect(noAddress.status).toBe(409);
    expect(String(noAddress.body.message)).toMatch(/payout address/i);
    expect((await setAddress(worker, id, ADDR)).status).toBe(200);
    await withdrawable(id);
    expect((await withdraw(employer, id)).status).toBe(403);
    expect((await withdraw(other, id)).status).toBe(404);
    expect((await withdraw(worker, id, randomUUID(), { origin: "https://evil.example" })).status).toBe(403);
    expect((await flow(worker, id)).payouts).toHaveLength(0);
  });
});

describe("employer view", () => {
  it("home lists streams with earned and withdrawn, payments with txids, and the shared treasury", async () => {
    const id = await readyStream("0.001", 10);
    await withdrawable(id);
    const p = (await withdraw(worker, id)).body.payout as { id: string; txid: string };
    const home = await payroll.employerHome(employer.id, employer.companyId);
    const s = home.streams.find((x) => x.id === id)!;
    expect(s).toMatchObject({ worker_name: "Wendy", total_zats: 100_000, address_locked: true, address: null });
    expect(s.payouts[0]).toMatchObject({ id: p.id, txid: p.txid });
    expect(home.payouts_enabled).toBe(true);
    const w = await payroll.payoutWallet(employer.id, employer.companyId);
    expect(w).toMatchObject({ network: "testnet", funding_address: "utest1mockfundingaddress" });
  });

  it("ending stops accrual and keeps what was earned; fixed-total streams can't be paused", async () => {
    const id = await boundStream(employer, worker, "0.001", 3600);
    await sleep(1100);
    const r = await call(endRoute.POST, "POST", { streamId: id }, { cookie: employer.cookie });
    expect(r.status).toBe(200);
    const earned = (r.body as unknown as FlowStream).balance.earned_zats;
    expect(earned).toBeGreaterThan(0);
    await sleep(1100);
    expect((await flow(worker, id)).balance.earned_zats).toBe(earned);
    expect((await call(endRoute.POST, "POST", { streamId: id }, { cookie: employer.cookie })).status).toBe(409);
    expect((await call(endRoute.POST, "POST", { streamId: id }, { cookie: worker.cookie })).status).toBe(403);

    const live = await boundStream(employer, worker, "0.001", 3600);
    const pause = await fetch(`${process.env.PAYROLL_API_URL}/v1/streams/${live}/pause`, {
      method: "POST",
      headers: { authorization: `Bearer ${process.env.PAYROLL_API_TOKEN}`, "x-actor-id": employer.id },
    });
    expect(pause.status).toBe(409);
  });
});

describe("isolation", () => {
  it("another company sees and touches nothing", async () => {
    const id = await readyStream("0.001", 3600);
    await expect(payroll.flowStream(rival.id, id)).rejects.toMatchObject({ status: 404 });
    await expect(payroll.employerHome(rival.id, employer.companyId)).rejects.toBeInstanceOf(PayrollApiError);
    await expect(payroll.payoutWallet(rival.id, employer.companyId)).rejects.toBeInstanceOf(PayrollApiError);
    expect((await call(endRoute.POST, "POST", { streamId: id }, { cookie: rival.cookie })).status).toBe(404);
    const home = await payroll.employerHome(rival.id, rival.companyId);
    expect(home.streams.some((s) => s.id === id)).toBe(false);
    await expect(payroll.flowStream(other.id, id)).rejects.toMatchObject({ status: 404 });
  });

  it("the read-only stream route answers its worker and employer only", async () => {
    const id = await readyStream("0.001", 3600);
    const get = (cookie?: string) => call(streamRoute.GET, "GET", { streamId: id }, { cookie, origin: null });
    const mine = await get(worker.cookie);
    expect(mine.status).toBe(200);
    expect(mine.body.address).toBe(ADDR);
    const boss = await get(employer.cookie);
    expect(boss.status).toBe(200);
    expect(boss.body.address).toBeNull();
    expect((await get(other.cookie)).status).toBe(404);
    expect((await get(rival.cookie)).status).toBe(404);
    expect((await get()).status).toBe(401);
  });

  it("malformed ids are plain 404s", async () => {
    await expect(payroll.flowStream(worker.id, "nope")).rejects.toMatchObject({ status: 404 });
    expect((await withdraw(worker, "../../etc")).status).toBe(404);
    expect((await call(cancelRoute.POST, "POST", { offerId: "x" }, { cookie: employer.cookie })).status).toBe(404);
  });
});

describe("payroll-api service boundary", () => {
  const api = (path: string, init: RequestInit = {}) => fetch(`${process.env.PAYROLL_API_URL}${path}`, init);

  it("rejects calls without the service token", async () => {
    expect((await api("/v1/me/flow-streams", { headers: { "x-actor-id": worker.id } })).status).toBe(401);
    expect((await api("/v1/links/" + "0".repeat(64))).status).toBe(401);
  });

  it("has no way to mark a withdrawal paid, and the old claim path refuses fixed-total streams", async () => {
    const id = await readyStream("0.001", 10);
    await withdrawable(id);
    const h = { authorization: `Bearer ${process.env.PAYROLL_API_TOKEN}`, "x-actor-id": worker.id, "content-type": "application/json" };
    const claim = await api(`/v1/streams/${id}/claims`, { method: "POST", headers: h, body: JSON.stringify({ amount_zec: "0.00001", idempotency_key: "legacy" }) });
    expect(claim.status).toBe(409);
    for (const path of [`/v1/claims/${randomUUID()}/settle`, `/v1/flow-streams/${id}/settle`]) {
      expect([404, 405]).toContain((await api(path, { method: "POST", headers: h, body: "{}" })).status);
    }
  });
});

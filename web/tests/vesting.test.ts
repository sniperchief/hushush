// Vesting (Phase 2) through the real route handlers, the real payroll-api
// and PostgreSQL, against the MOCK payout wallet (tests/support/mock-wallet.ts):
// no keys, chain or funds are involved here. Accounts use a SOFTWARE passkey.
import { randomUUID } from "node:crypto";
import { beforeAll, describe, expect, it } from "vitest";
import * as createRoute from "@/app/api/vesting/route";
import * as activateRoute from "@/app/api/vesting/[agreementId]/activate/route";
import * as cancelRoute from "@/app/api/vesting/[agreementId]/cancel/route";
import * as addressRoute from "@/app/api/vesting/[agreementId]/address/route";
import * as quoteRoute from "@/app/api/vesting/[agreementId]/quote/route";
import * as withdrawRoute from "@/app/api/vesting/[agreementId]/withdraw/route";
import { db } from "@/lib/db";
import { PayrollApiError, payroll, type VestingDetail } from "@/lib/payroll";
import { zecOf } from "@/lib/vesting";
import { boundStream, call, mock, newEmployer, newWorker, signUp, sleep, waitFor, type TestUser } from "./support/helpers";

const ADDR = "utest1mockvestingaddressaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER = "utest1mockotheraddressbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

let employer: TestUser & { companyId: string };
let rival: TestUser & { companyId: string };
let worker: TestUser;
let colleague: TestUser;
let stranger: TestUser;

beforeAll(async () => {
  [employer, rival, worker, colleague, stranger] = await Promise.all([
    newEmployer("Vera", "Vesting Co"),
    newEmployer("Rex", "Rival Vesting Co"),
    newWorker("Wes"),
    newWorker("Cole"),
    newWorker("Stan"),
  ]);
  // People join a company through a stream's worker link.
  await boundStream(employer, worker, "0.001", 3600);
  await boundStream(employer, colleague, "0.001", 3600);
});

const now = () => Math.floor(Date.now() / 1000);

const create = (u: TestUser, body: Record<string, unknown>) =>
  call(createRoute.POST, "POST", {}, {
    cookie: u.cookie,
    body: { employeeUserId: worker.id, totalZec: "1", startAt: now() + 3600, durationDays: 365, cliffDays: null, ...body },
  });
const activate = (u: TestUser, id: string, opts: { origin?: string | null } = {}) =>
  call(activateRoute.POST, "POST", { agreementId: id }, { cookie: u.cookie, ...opts });
const cancel = (u: TestUser, id: string) => call(cancelRoute.POST, "POST", { agreementId: id }, { cookie: u.cookie });
const setAddress = (u: TestUser, id: string, address = ADDR) =>
  call(addressRoute.POST, "POST", { agreementId: id }, { cookie: u.cookie, body: { address } });
const quote = (u: TestUser, id: string, amountZec: string) =>
  call(quoteRoute.POST, "POST", { agreementId: id }, { cookie: u.cookie, body: { amountZec } });
const withdraw = (u: TestUser, id: string, amountZec: string, idempotencyKey: string = randomUUID()) =>
  call(withdrawRoute.POST, "POST", { agreementId: id }, { cookie: u.cookie, body: { amountZec, idempotencyKey } });
const detail = (u: TestUser, id: string): Promise<VestingDetail> => payroll.vesting(u.id, id);

/**
 * A short schedule for real-time tests: payroll-api takes seconds (the web
 * form only offers whole days). Created and activated by the employer, with
 * the worker's address locked.
 */
async function shortGrant(totalZec: string, durationSecs: number, cliffSecs: number | null = null): Promise<string> {
  const v = await payroll.createVesting(employer.id, employer.companyId, {
    employee_user_id: worker.id,
    total_zec: totalZec,
    start_at: now() + 2,
    duration_secs: durationSecs,
    cliff_secs: cliffSecs,
  });
  expect((await activate(employer, v.id)).status).toBe(200);
  expect((await setAddress(worker, v.id)).status).toBe(200);
  return v.id;
}

async function confirmed(id: string, payoutId: string): Promise<VestingDetail> {
  await mock("mine");
  await mock("advance", { blocks: 5 });
  return waitFor(
    () => detail(worker, id),
    (d) => d.payouts.find((p) => p.id === payoutId)?.state === "confirmed",
  );
}

describe("agreements", () => {
  it("an employer creates a draft for a member; the employee can't see it until it is activated", async () => {
    const r = await create(employer, { totalZec: "12.5", durationDays: 400, cliffDays: 100 });
    expect(r.status, JSON.stringify(r.body)).toBe(201);
    const v = r.body.agreement as Record<string, unknown>;
    expect(v).toMatchObject({
      status: "draft",
      phase: "draft",
      total_zats: 1_250_000_000,
      duration_secs: 400 * 86_400,
      cliff_secs: 100 * 86_400,
      // The cliff vests the linear share of its period: 100/400 of 12.5 ZEC.
      cliff_vests_zats: 312_500_000,
      employee_display_name: "Wes",
      organization_name: "Vesting Co",
      activatable: true,
      cancellable: true,
    });
    const id = v.id as string;
    await expect(detail(worker, id)).rejects.toMatchObject({ status: 404 });
    expect((await payroll.myVesting(worker.id)).some((x) => x.id === id)).toBe(false);
    expect((await payroll.orgVesting(employer.id, employer.companyId)).some((x) => x.id === id)).toBe(true);

    expect((await activate(employer, id)).status).toBe(200);
    const d = await detail(worker, id);
    expect(d).toMatchObject({ viewer: "employee", address_locked: false, withdrawable_zats: null });
    expect(d.agreement).toMatchObject({ status: "active", phase: "not_started" });
    expect(d.events.map((e) => e.kind)).toEqual(["created", "activated"]);
    expect((await payroll.myVesting(worker.id)).some((x) => x.id === id)).toBe(true);
  });

  it("validates the terms and who the employee may be", async () => {
    const status = async (u: TestUser, body: Record<string, unknown>) => (await create(u, body)).status;
    expect(await status(employer, { employeeUserId: stranger.id })).toBe(422); // not a member
    expect(await status(employer, { employeeUserId: employer.id })).toBe(403); // not yourself
    expect(await status(employer, { employeeUserId: "nope" })).toBe(422);
    expect(await status(employer, { startAt: now() - 3600 })).toBe(422); // backdated
    expect(await status(employer, { durationDays: 0 })).toBe(422);
    expect(await status(employer, { durationDays: 10, cliffDays: 11 })).toBe(422);
    expect(await status(employer, { totalZec: "0" })).toBe(422);
    expect(await status(employer, { totalZec: "1.123456789" })).toBe(422);
    expect(await status(employer, { totalZec: "-1" })).toBe(422);
    // A rival company can't grant to someone who isn't its member.
    expect(await status(rival, {})).toBe(422);
    // An account without a company can't create agreements at all.
    const noCompany = await signUp("employer", "No Company");
    expect(await status(noCompany, {})).toBe(409);
    expect(await status(worker, {})).toBe(409);
  });

  it("activates only before the start; a draft whose start passed can only be discarded", async () => {
    const v = await payroll.createVesting(employer.id, employer.companyId, {
      employee_user_id: worker.id,
      total_zec: "1",
      start_at: now() + 1,
      duration_secs: 3600,
      cliff_secs: null,
    });
    await sleep(2100);
    const late = await activate(employer, v.id);
    expect(late.status).toBe(409);
    expect(late.body.message).toMatch(/start date has passed/);
    const d = await detail(employer, v.id);
    expect(d.agreement).toMatchObject({ status: "draft", activatable: false, cancellable: true });
    const x = await cancel(employer, v.id);
    expect(x.status).toBe(200);
    expect(x.body).toMatchObject({ status: "cancelled", activated_at: null });
    await expect(detail(worker, v.id)).rejects.toMatchObject({ status: 404 });
  });
});

describe("authorization", () => {
  it("only the company's managers manage; only the employee sees the address and withdraws", async () => {
    const id = (await create(employer, {})).body.agreement as { id: string };
    // Rival and colleagues can't see a draft at all.
    for (const u of [rival, colleague, worker, stranger]) {
      expect((await activate(u, id.id)).status).toBe(404);
      expect((await cancel(u, id.id)).status).toBe(404);
    }
    expect((await activate(employer, id.id)).status).toBe(200);
    // Once active, the employee sees it but can't manage it.
    expect((await cancel(worker, id.id)).status).toBe(403);
    for (const u of [rival, colleague, stranger]) {
      await expect(detail(u, id.id)).rejects.toMatchObject({ status: 404 });
      expect((await setAddress(u, id.id)).status).toBe(404);
      expect((await quote(u, id.id, "0.1")).status).toBe(404);
      expect((await withdraw(u, id.id, "0.1")).status).toBe(404);
    }
    // The employer can't set the address or withdraw (no address probing either).
    expect((await setAddress(employer, id.id)).status).toBe(403);
    expect((await withdraw(employer, id.id, "0.1")).status).toBe(403);
    expect((await quote(employer, id.id, "0.1")).status).toBe(403);
    // Listings stay inside each company.
    await expect(payroll.orgVesting(rival.id, employer.companyId)).rejects.toMatchObject({ status: 404 });
    expect((await payroll.myVesting(colleague.id)).length).toBe(0);
    // Session and CSRF are required.
    const anon = await call(activateRoute.POST, "POST", { agreementId: id.id }, {});
    expect(anon.status).toBe(401);
    expect((await activate(employer, id.id, { origin: "https://evil.example" })).status).toBe(403);
  });

  it("payroll-api rejects missing or invalid service authentication", async () => {
    const url = process.env.PAYROLL_API_URL!;
    const get = (headers: Record<string, string>) => fetch(`${url}/v1/me/vesting`, { headers });
    expect((await get({})).status).toBe(401);
    expect((await get({ authorization: "Bearer wrong", "x-actor-id": worker.id })).status).toBe(401);
    // With the token but no actor, user endpoints refuse too.
    expect((await get({ authorization: `Bearer ${process.env.PAYROLL_API_TOKEN}` })).status).toBe(401);
    const withdrawNoToken = await fetch(`${url}/v1/vesting/${randomUUID()}/withdraw`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-actor-id": worker.id },
      body: JSON.stringify({ amount_zec: "1", idempotency_key: "k" }),
    });
    expect(withdrawNoToken.status).toBe(401);
  });
});

describe("withdraw", () => {
  it("nothing before the cliff; then quote, withdraw, and paid only once confirmed", async () => {
    // 0.001 ZEC over 10 s with a 4 s cliff: 40,000 zats vest at the cliff.
    const id = await shortGrant("0.001", 10, 4);
    const before = await detail(worker, id);
    expect(before.agreement.phase === "not_started" || before.agreement.phase === "in_cliff").toBe(true);
    expect(before.withdrawable_zats).toBeNull();
    expect((await withdraw(worker, id, "0.00000001")).status).toBe(409); // unvested: not claimable

    const d = await waitFor(() => detail(worker, id), (x) => x.agreement.balance.vested_zats >= 40_000, 10_000);
    expect(d.address).toBe(ADDR);
    expect(d.withdrawable_zats).toBe(d.agreement.balance.available_zats);

    // More than available is refused by the server, whatever the browser says.
    const tooMuch = await quote(worker, id, "0.002");
    expect(tooMuch.status).toBe(409);
    expect(tooMuch.body.error).toBe("insufficient_available");
    const q = await quote(worker, id, "0.0003");
    expect(q.status).toBe(200);
    expect(q.body).toMatchObject({ amount_zats: 30_000, fee_zats: 10_000, valid_recipient: true, sufficient: true, required_confirmations: 3 });
    expect((await detail(worker, id)).agreement.balance.reserved_zats).toBe(0); // quoting reserves nothing

    const built = (await mock("advance", { blocks: 0 })).txs_built;
    const key = randomUUID();
    const w = await withdraw(worker, id, "0.0003", key);
    expect(w.status, JSON.stringify(w.body)).toBe(201);
    const p = w.body.payout as { id: string; state: string; compensation_type: string; claim_id: string };
    expect(p.compensation_type).toBe("vesting");
    expect(["submitted", "reserved"]).toContain(p.state);
    // Retries (same key) and other taps while it is unfinished return it.
    const again = await withdraw(worker, id, "0.0003", key);
    expect(again.status).toBe(200);
    expect(again.body).toMatchObject({ duplicate: true, payout: { id: p.id } });
    expect((await withdraw(worker, id, "0.0001")).body).toMatchObject({ duplicate: true, payout: { id: p.id } });
    expect((await quote(worker, id, "0.0001")).status).toBe(409); // in progress
    const mid = await detail(worker, id);
    expect(mid.agreement.balance).toMatchObject({ reserved_zats: 30_000, paid_zats: 0 });
    expect(mid.claims[0]).toMatchObject({ status: "reserved", amount_zats: 30_000 });

    const done = await confirmed(id, p.id);
    expect(done.agreement.balance).toMatchObject({ reserved_zats: 0, paid_zats: 30_000 });
    expect(done.claims[0]).toMatchObject({ status: "settled", payout: { id: p.id, state: "confirmed" } });
    expect((await mock("advance", { blocks: 0 })).txs_built).toBe(built + 1);

    // The address is sealed in the database and invisible to the employer.
    const { rows } = await db().query("SELECT address_sealed FROM vesting_payout_addresses WHERE agreement_id = $1", [id]);
    expect(rows[0].address_sealed).toMatch(/^v1\.[0-9a-f]+$/);
    expect(rows[0].address_sealed).not.toContain(ADDR);
    const asEmployer = await detail(employer, id);
    expect(asEmployer).toMatchObject({ viewer: "manager", address: null, address_locked: true });
    expect(asEmployer.payouts[0]!.state).toBe("confirmed");
    // The address is locked.
    expect((await setAddress(worker, id, OTHER)).status).toBe(409);
  });

  it("concurrent withdrawals pay once", async () => {
    const id = await shortGrant("0.001", 4);
    await waitFor(() => detail(worker, id), (x) => x.agreement.phase === "fully_vested", 10_000);
    const built = (await mock("advance", { blocks: 0 })).txs_built;
    const rs = await Promise.all([0, 1, 2, 3].map(() => withdraw(worker, id, "0.0005")));
    const created = rs.filter((r) => r.status === 201);
    expect(created.length, JSON.stringify(rs.map((r) => r.body))).toBe(1);
    const pid = (created[0]!.body.payout as { id: string }).id;
    for (const r of rs) expect((r.body.payout as { id: string }).id).toBe(pid);
    await confirmed(id, pid);
    expect((await mock("advance", { blocks: 0 })).txs_built).toBe(built + 1);
    const d = await detail(worker, id);
    expect(d.claims.length).toBe(1);
    expect(d.agreement.balance).toMatchObject({ paid_zats: 50_000, available_zats: 50_000 });
  });

  it("a failed withdrawal releases its amount; nothing is sent", async () => {
    const id = await shortGrant("0.001", 4);
    await waitFor(() => detail(worker, id), (x) => x.agreement.phase === "fully_vested", 10_000);
    const built = (await mock("advance", { blocks: 0 })).txs_built;
    await mock("next", { mode: "fail" });
    const w = await withdraw(worker, id, "0.001");
    expect((w.body.payout as { state: string }).state).toBe("failed");
    const d = await detail(worker, id);
    expect(d.agreement.balance).toMatchObject({ reserved_zats: 0, paid_zats: 0, available_zats: 100_000 });
    expect(d.claims[0]!.status).toBe("released");
    expect((await mock("advance", { blocks: 0 })).txs_built).toBe(built);
  });
});

describe("cancellation", () => {
  it("keeps an in-flight withdrawal and what has vested; stops the rest", async () => {
    const id = await shortGrant("0.001", 20); // 5,000 zats a second
    await waitFor(() => detail(worker, id), (x) => x.agreement.balance.vested_zats >= 30_000, 10_000);
    const w = await withdraw(worker, id, "0.0002");
    expect(w.status).toBe(201);
    const pid = (w.body.payout as { id: string }).id;

    const x = await cancel(employer, id);
    expect(x.status).toBe(200);
    const frozen = (x.body.balance as { vested_zats: number }).vested_zats;
    expect(frozen).toBeGreaterThanOrEqual(30_000);
    expect(frozen).toBeLessThan(100_000);
    expect(x.body).toMatchObject({ status: "cancelled", phase: "cancelled", cancellable: false });

    // The in-flight withdrawal still completes and settles.
    const done = await confirmed(id, pid);
    expect(done.agreement.balance).toMatchObject({ vested_zats: frozen, paid_zats: 20_000, unvested_zats: 100_000 - frozen });
    await sleep(1500);
    const later = await detail(worker, id);
    expect(later.agreement.balance.vested_zats).toBe(frozen); // vesting stopped
    expect(later.agreement.balance.available_zats).toBe(frozen - 20_000);
    expect(later.events.map((e) => e.kind)).toEqual(["created", "activated", "cancelled"]);
    // The rest of what vested is still withdrawable — and nothing more.
    const tooMuch = await withdraw(worker, id, zecOf(BigInt(frozen - 20_000 + 1)));
    expect(tooMuch.status).toBe(409);
    expect((await cancel(employer, id)).status).toBe(409); // final
  });
});

it("payroll-api errors keep their status through the web client", async () => {
  const e = await payroll.vesting(worker.id, randomUUID()).catch((x: unknown) => x);
  expect(e).toBeInstanceOf(PayrollApiError);
  expect((e as PayrollApiError).status).toBe(404);
});

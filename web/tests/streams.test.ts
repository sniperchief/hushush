import { beforeAll, describe, expect, it } from "vitest";
import { payroll } from "@/lib/payroll";
import * as streamsRoute from "@/app/api/orgs/[orgId]/streams/route";
import { call, createOrg, fastStream, join, lifecycle, newUser, sleep, type TestUser } from "./support/helpers";

let owner: TestUser, worker: TestUser;
let org: string;

beforeAll(async () => {
  [owner, worker] = await Promise.all([newUser("sOwner", "Stream Owner"), newUser("sWorker", "Stream Worker")]);
  org = await createOrg(owner, "Streams Inc");
  await join(owner, org, worker);
});

const earned = async (id: string) => (await payroll.stream(worker.id, id)).stream.balance.earned_zats;

function createBody(extra: Record<string, unknown>) {
  return { recipientUserId: worker.id, rateZec: "1", ratePeriodSecs: 3600, startAt: null, endAt: null, ...extra };
}

describe("stream creation", () => {
  it("creates a fixed-rate ZEC stream with exact zatoshi rate", async () => {
    const r = await call(streamsRoute.POST, "POST", { orgId: org }, { cookie: owner.cookie, body: createBody({ rateZec: "1.23456789" }) });
    expect(r.status).toBe(201);
    expect(r.body.rate_zats).toBe(123_456_789);
    expect(r.body.rate_zec).toBe("1.23456789");
    expect(r.body.status).toBe("active");
  });

  it("rejects backdated starts, bad end dates, bad amounts and bad periods", async () => {
    const now = Math.floor(Date.now() / 1000);
    const cases = [
      createBody({ startAt: now - 3600 }),
      createBody({ startAt: now + 3600, endAt: now + 60 }),
      createBody({ rateZec: "0" }),
      createBody({ rateZec: "1.123456789" }),
      createBody({ rateZec: "-1" }),
      createBody({ rateZec: "1e3" }),
      createBody({ ratePeriodSecs: 7 }),
    ];
    for (const body of cases) {
      const r = await call(streamsRoute.POST, "POST", { orgId: org }, { cookie: owner.cookie, body });
      expect(r.status, JSON.stringify(body)).toBe(422);
    }
  });

  it("a future-dated stream accrues nothing before it starts", async () => {
    const now = Math.floor(Date.now() / 1000);
    const s = await fastStream(owner, org, worker, { startAt: now + 3600 });
    await sleep(1100);
    expect(await earned(s.id)).toBe(0);
  });

  it("an end date caps accrual", async () => {
    const now = Math.floor(Date.now() / 1000);
    const s = await fastStream(owner, org, worker, { endAt: now + 2 });
    await sleep(3200);
    const b = (await payroll.stream(worker.id, s.id)).stream;
    expect(b.status).toBe("ended");
    expect(b.balance.earned_zats).toBeLessThanOrEqual(3 * 100_000_000);
    const again = b.balance.earned_zats;
    await sleep(1100);
    expect(await earned(s.id)).toBe(again);
  });
});

describe("pause, resume and end", () => {
  it("pausing stops accrual without losing earned wages; resuming continues", async () => {
    const s = await fastStream(owner, org, worker);
    await sleep(1100);
    const p = await lifecycle(owner, s.id, "pause");
    expect(p.status).toBe(200);
    const frozen = await earned(s.id);
    expect(frozen).toBeGreaterThanOrEqual(100_000_000);
    await sleep(1200);
    expect(await earned(s.id)).toBe(frozen);
    expect((await lifecycle(owner, s.id, "pause")).status).toBe(409); // already paused

    expect((await lifecycle(owner, s.id, "resume")).status).toBe(200);
    await sleep(1100);
    expect(await earned(s.id)).toBeGreaterThan(frozen);
  });

  it("ending is permanent and preserves earned wages", async () => {
    const s = await fastStream(owner, org, worker);
    await sleep(1100);
    expect((await lifecycle(owner, s.id, "end")).status).toBe(200);
    const final = await earned(s.id);
    expect(final).toBeGreaterThan(0);
    await sleep(1100);
    expect(await earned(s.id)).toBe(final);
    for (const a of ["pause", "resume", "end"] as const) {
      expect((await lifecycle(owner, s.id, a)).status).toBe(409);
    }
    const detail = await payroll.stream(owner.id, s.id);
    expect(detail.events.map((e) => e.kind)).toEqual(["created", "ended"]);
  });

  it("records full history in order", async () => {
    const s = await fastStream(owner, org, worker);
    await lifecycle(owner, s.id, "pause");
    await lifecycle(owner, s.id, "resume");
    await lifecycle(owner, s.id, "end");
    const { events } = await payroll.stream(worker.id, s.id);
    expect(events.map((e) => e.kind)).toEqual(["created", "paused", "resumed", "ended"]);
    expect(events.every((e) => e.actor_user_id === owner.id)).toBe(true);
  });

  it("a rate change is an end plus a new stream; the old stream keeps its earnings", async () => {
    const old = await fastStream(owner, org, worker);
    await sleep(1100);
    await lifecycle(owner, old.id, "end");
    const kept = await earned(old.id);
    const next = await fastStream(owner, org, worker, { rateZec: "7200" });
    expect(next.id).not.toBe(old.id);
    expect(await earned(old.id)).toBe(kept);
    const mine = await payroll.myStreams(worker.id);
    expect(mine.find((x) => x.id === old.id)?.status).toBe("ended");
    expect(mine.find((x) => x.id === next.id)?.rate_zec).toBe("7200.00000000");
  });
});

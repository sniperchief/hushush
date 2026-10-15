// Dashboard chart maths: pure functions, plus agreement with the engine.
import { describe, expect, it } from "vitest";
import * as endRoute from "@/app/api/streams/[streamId]/end/route";
import {
  DAY,
  dailySeries,
  earnedAt,
  formatZats,
  mondayOf,
  niceMax,
  statusCounts,
  weeklyWithdrawals,
  withdrawnAt,
  type ChartStream,
} from "@/lib/chart-data";
import { boundStream, call, flow, newEmployer, newWorker, sleep } from "./support/helpers";

const U = (y: number, m: number, d: number, h = 0, mi = 0) => Date.UTC(y, m, d, h, mi) / 1000;

const stream = (over: Partial<ChartStream> = {}): ChartStream => ({
  total_zats: 150_000_000,
  start_at: U(2026, 9, 1),
  end_at: U(2026, 9, 31),
  ended_at: null,
  payouts: [],
  ...over,
});

describe("earnedAt", () => {
  it("is 0 before the start, linear in between, and the total from the end on", () => {
    const s = stream();
    expect(earnedAt(s, s.start_at - 1)).toBe(0n);
    expect(earnedAt(s, s.start_at)).toBe(0n);
    expect(earnedAt(s, U(2026, 9, 16))).toBe(75_000_000n); // half way
    expect(earnedAt(s, s.end_at)).toBe(150_000_000n);
    expect(earnedAt(s, s.end_at + 10 * DAY)).toBe(150_000_000n);
  });

  it("rounds down to whole zatoshis", () => {
    const s = stream({ total_zats: 100, start_at: 0, end_at: 3 });
    expect(earnedAt(s, 1)).toBe(33n);
    expect(earnedAt(s, 2)).toBe(66n);
  });

  it("stops at ended_at when the stream ended early", () => {
    const s = stream({ ended_at: U(2026, 9, 16) });
    expect(earnedAt(s, U(2026, 9, 20))).toBe(75_000_000n);
    expect(earnedAt(stream({ ended_at: U(2026, 8, 1) }), U(2026, 9, 20))).toBe(0n);
  });

  it("equals the engine's balance.earned_zats at as_of, active and ended", async () => {
    const boss = await newEmployer("Chart Boss");
    const worker = await newWorker("Chart Worker");
    const id = await boundStream(boss, worker, "0.00123457", 3600);
    await sleep(1500);
    const live = await flow(worker, id);
    expect(earnedAt(live, live.balance.as_of)).toBe(BigInt(live.balance.earned_zats));
    expect(live.balance.earned_zats).toBeGreaterThan(0);

    expect((await call(endRoute.POST, "POST", { streamId: id }, { cookie: boss.cookie })).status).toBe(200);
    await sleep(1100);
    const ended = await flow(worker, id);
    expect(ended.ended_at).not.toBeNull();
    expect(earnedAt(ended, ended.balance.as_of)).toBe(BigInt(ended.balance.earned_zats));
    // Nothing more accrues after the end.
    expect(earnedAt(ended, ended.balance.as_of + 3600)).toBe(BigInt(ended.balance.earned_zats));
  });
});

describe("withdrawnAt", () => {
  it("counts only confirmed payouts, from the moment they resolved", () => {
    const s = stream({
      payouts: [
        { amount_zats: 30_000_000, state: "confirmed", created_at: 100, resolved_at: 200 },
        { amount_zats: 5_000_000, state: "submitted", created_at: 300, resolved_at: null },
        { amount_zats: 7_000_000, state: "failed", created_at: 300, resolved_at: 310 },
        { amount_zats: 10_000_000, state: "confirmed", created_at: 400, resolved_at: 500 },
      ],
    });
    expect(withdrawnAt(s, 199)).toBe(0n);
    expect(withdrawnAt(s, 200)).toBe(30_000_000n);
    expect(withdrawnAt(s, 499)).toBe(30_000_000n);
    expect(withdrawnAt(s, 500)).toBe(40_000_000n);
  });
});

describe("dailySeries", () => {
  it("has a point per day at the same clock time, then one at `to`", () => {
    const from = U(2026, 9, 1, 14);
    const pts = dailySeries([stream()], from, from + 2 * DAY + 3600);
    expect(pts.map((p) => p.t)).toEqual([from, from + DAY, from + 2 * DAY, from + 2 * DAY + 3600]);
    expect(pts[1]!.earned).toBe(earnedAt(stream(), from + DAY));
  });

  it("sums several streams", () => {
    const a = stream();
    const b = stream({ total_zats: 200_000_000, start_at: U(2026, 9, 10), end_at: U(2026, 11, 9) });
    const t = U(2026, 9, 20);
    expect(dailySeries([a, b], t, t)[0]!.earned).toBe(earnedAt(a, t) + earnedAt(b, t));
  });
});

describe("weeklyWithdrawals", () => {
  it("groups by the Monday of created_at (UTC), splits paid from in flight, drops failed", () => {
    const now = U(2026, 9, 15, 14); // Thursday 15 Oct 2026
    expect(new Date(mondayOf(now) * 1000).toISOString()).toBe("2026-10-12T00:00:00.000Z");
    // Sunday 23:59 belongs to the week before; Monday 00:00 starts a new one.
    expect(mondayOf(U(2026, 9, 11, 23, 59))).toBe(U(2026, 9, 5));
    expect(mondayOf(U(2026, 9, 12))).toBe(U(2026, 9, 12));

    const lena = { ...stream(), worker_name: "Lena", payouts: [{ amount_zats: 250_000_000, state: "confirmed", created_at: U(2026, 8, 30, 8), resolved_at: U(2026, 8, 30, 9) }] };
    const marco = {
      ...stream(),
      worker_name: "Marco",
      payouts: [
        { amount_zats: 160_000_000, state: "confirmed", created_at: U(2026, 9, 2), resolved_at: U(2026, 9, 2, 1) },
        { amount_zats: 135_833_000, state: "submitted", created_at: U(2026, 9, 15, 13), resolved_at: null },
        { amount_zats: 99_000_000, state: "failed", created_at: U(2026, 9, 14), resolved_at: U(2026, 9, 14, 1) },
        { amount_zats: 1, state: "confirmed", created_at: U(2026, 7, 1), resolved_at: U(2026, 7, 1) }, // too old
      ],
    };
    const weeks = weeklyWithdrawals([lena, marco], now);
    expect(weeks.map((w) => new Date(w.weekStart * 1000).toISOString().slice(0, 10))).toEqual([
      "2026-09-14",
      "2026-09-21",
      "2026-09-28",
      "2026-10-05",
      "2026-10-12",
    ]);
    expect(weeks.map((w) => [formatZats(w.paid), formatZats(w.inFlight)])).toEqual([
      ["0", "0"],
      ["0", "0"],
      ["4.1", "0"],
      ["0", "0"],
      ["0", "1.35833"],
    ]);
    expect(weeks[2]!.byWorker).toEqual([
      { name: "Lena", paid: 250_000_000n, inFlight: 0n },
      { name: "Marco", paid: 160_000_000n, inFlight: 0n },
    ]);
  });
});

describe("helpers", () => {
  it("statusCounts follows the Streams table's rule", () => {
    const c = statusCounts(
      [
        { status: "active", payouts: [] },
        { status: "active", payouts: [{ state: "confirmed" }, { state: "failed" }] },
        { status: "active", payouts: [{ state: "submitted" }] },
        { status: "ended", payouts: [{ state: "reserved" }] },
      ],
      2,
    );
    expect(c).toEqual({ active: 2, withdrawing: 1, ended: 1, waiting: 2 });
  });

  it("formats zatoshis exactly and picks round axis maxima", () => {
    expect(formatZats(150_000_000n)).toBe("1.5");
    expect(formatZats(1n)).toBe("0.00000001");
    expect(formatZats(0n)).toBe("0");
    expect(niceMax(9.17)).toBe(10);
    expect(niceMax(4.1)).toBe(5);
    expect(niceMax(2.2)).toBe(2.5);
    expect(niceMax(0)).toBe(1);
  });
});

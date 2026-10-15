// Dashboard chart maths. Pure functions over data the pages already load, in
// whole zatoshis (BigInt) like the engine, so no float rounding creeps in.
//
// A stream earns in a straight line, so any past (or future) moment's value is
// exact: earned(t) = floor(total × (min(t, cap) − start) / (end − start)),
// 0 before start, where cap is end, or ended_at if the stream ended early.

export const DAY = 86_400;
const COIN = 100_000_000n;

export interface ChartPayout {
  amount_zats: number;
  state: string;
  created_at: number;
  resolved_at: number | null;
}

export interface ChartStream {
  total_zats: number;
  start_at: number;
  end_at: number;
  ended_at: number | null;
  payouts: ChartPayout[];
}

/** Payout states that are neither paid nor failed: money on its way. */
export const IN_FLIGHT = new Set(["reserved", "submitted", "requires_reconciliation"]);

export function earnedAt(s: ChartStream, t: number): bigint {
  if (t <= s.start_at || s.end_at <= s.start_at) return 0n;
  const cap = s.ended_at != null ? Math.min(s.end_at, s.ended_at) : s.end_at;
  const elapsed = Math.max(0, Math.min(t, cap) - s.start_at);
  return (BigInt(s.total_zats) * BigInt(elapsed)) / BigInt(s.end_at - s.start_at);
}

/** Confirmed payouts paid out by `t`. */
export function withdrawnAt(s: ChartStream, t: number): bigint {
  let sum = 0n;
  for (const p of s.payouts) if (p.state === "confirmed" && p.resolved_at != null && p.resolved_at <= t) sum += BigInt(p.amount_zats);
  return sum;
}

export interface SeriesPoint {
  t: number;
  earned: bigint;
  withdrawn: bigint;
}

/** Daily points from `from` (same clock time each day) up to and including `to`. */
export function dailySeries(streams: ChartStream[], from: number, to: number): SeriesPoint[] {
  const times: number[] = [];
  for (let t = from; t < to; t += DAY) times.push(t);
  times.push(to);
  return times.map((t) => ({
    t,
    earned: streams.reduce((a, s) => a + earnedAt(s, t), 0n),
    withdrawn: streams.reduce((a, s) => a + withdrawnAt(s, t), 0n),
  }));
}

/** Monday 00:00 UTC of the week containing `t`. */
export function mondayOf(t: number): number {
  const d = new Date(t * 1000);
  const back = (d.getUTCDay() + 6) % 7;
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() - back) / 1000;
}

export interface WeekBucket {
  weekStart: number;
  paid: bigint;
  inFlight: bigint;
  /** Per worker, in order of first appearance. */
  byWorker: { name: string; paid: bigint; inFlight: bigint }[];
}

/**
 * Payouts grouped by the Monday of their `created_at` week (UTC), for the
 * `weeks` weeks ending with the current one. Failed payouts are left out.
 */
export function weeklyWithdrawals(streams: (ChartStream & { worker_name: string })[], now: number, weeks = 5): WeekBucket[] {
  const current = mondayOf(now);
  const buckets: WeekBucket[] = [];
  for (let i = weeks - 1; i >= 0; i--) {
    // Step back whole weeks by calendar date (no DST in UTC, but stay exact).
    const d = new Date(current * 1000);
    buckets.push({ weekStart: Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() - 7 * i) / 1000, paid: 0n, inFlight: 0n, byWorker: [] });
  }
  for (const s of streams) {
    for (const p of s.payouts) {
      const paid = p.state === "confirmed";
      if (!paid && !IN_FLIGHT.has(p.state)) continue;
      const b = buckets.find((x) => x.weekStart === mondayOf(p.created_at));
      if (!b) continue;
      const amount = BigInt(p.amount_zats);
      let w = b.byWorker.find((x) => x.name === s.worker_name);
      if (!w) b.byWorker.push((w = { name: s.worker_name, paid: 0n, inFlight: 0n }));
      if (paid) {
        b.paid += amount;
        w.paid += amount;
      } else {
        b.inFlight += amount;
        w.inFlight += amount;
      }
    }
  }
  return buckets;
}

export interface StatusCounts {
  active: number;
  withdrawing: number;
  ended: number;
  waiting: number;
}

/** Same rule as the Streams table's status badge. */
export function streamState(s: { status: string; payouts: { state: string }[] }): "active" | "withdrawing" | "ended" {
  if (s.status === "ended") return "ended";
  if (s.payouts.some((p) => p.state !== "confirmed" && p.state !== "failed")) return "withdrawing";
  return "active";
}

export function statusCounts(streams: { status: string; payouts: { state: string }[] }[], openOffers: number): StatusCounts {
  const c: StatusCounts = { active: 0, withdrawing: 0, ended: 0, waiting: openOffers };
  for (const s of streams) c[streamState(s)]++;
  return c;
}

// ---- formatting --------------------------------------------------------------

/** Zatoshis → ZEC with trailing zeros trimmed ("150000000" → "1.5"). Exact. */
export function formatZats(z: bigint): string {
  const neg = z < 0n;
  const a = neg ? -z : z;
  const frac = (a % COIN).toString().padStart(8, "0").replace(/0+$/, "");
  return `${neg ? "-" : ""}${a / COIN}${frac ? `.${frac}` : ""}`;
}

/** For plotting only. */
export const zecNumber = (z: bigint): number => Number(z) / 1e8;

/** A round axis maximum at or above `v` (1, 2, 2.5, 5 × 10^k). */
export function niceMax(v: number): number {
  if (!(v > 0)) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= v) return m * p;
  return 10 * p;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "15 Oct" (UTC). */
export function dayLabel(t: number): string {
  const d = new Date(t * 1000);
  return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]}`;
}

/** "15 Oct, 14:00 UTC". */
export function dayTimeLabel(t: number): string {
  const d = new Date(t * 1000);
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${dayLabel(t)}, ${hh}:${mm} UTC`;
}

/** "1 Oct → 31 Oct 2026" (UTC; the year is shown once when both share it). */
export function rangeLabel(a: number, b: number): string {
  const ya = new Date(a * 1000).getUTCFullYear();
  const yb = new Date(b * 1000).getUTCFullYear();
  return ya === yb ? `${dayLabel(a)} → ${dayLabel(b)} ${yb}` : `${dayLabel(a)} ${ya} → ${dayLabel(b)} ${yb}`;
}

/** First instant (UTC) of the month containing `t`, and of the month after. */
export function monthBounds(t: number): { from: number; to: number } {
  const d = new Date(t * 1000);
  return {
    from: Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), 1) / 1000,
    to: Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1) / 1000,
  };
}

/** Initials for an avatar: "Marco Lindqvist" → "ML". */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const a = parts[0]?.[0] ?? "?";
  const b = parts.length > 1 ? (parts[parts.length - 1]?.[0] ?? "") : (parts[0]?.[1] ?? "");
  return (a + b).toUpperCase();
}

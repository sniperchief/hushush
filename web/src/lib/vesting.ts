// Vesting display helpers, safe for server and client components. Balances
// always come from payroll (the engine is authoritative); the only arithmetic
// here is the creation form's preview of the terms being entered, in exact
// BigInt zatoshis, using the engine's rule.

import { compactZec, formatTime } from "./format";
import type { VestingPhase } from "./payroll";

export const DAY = 86_400;
const COIN = BigInt(100_000_000);

/** "1.5" → 150000000n; null unless a positive ZEC amount with at most 8 decimals. */
export function parseZats(s: string): bigint | null {
  const m = /^(\d+)(?:\.(\d{1,8}))?$/.exec(s.trim());
  if (!m) return null;
  const z = BigInt(m[1]!) * COIN + BigInt((m[2] ?? "").padEnd(8, "0"));
  return z > BigInt(0) ? z : null;
}

/** 150000000n → "1.5". */
export function zecOf(zats: bigint): string {
  return compactZec(`${zats / COIN}.${(zats % COIN).toString().padStart(8, "0")}`);
}

/**
 * The engine's rule at the cliff: the linear share of the cliff period,
 * floor(total × cliff / duration) — everything at the end when the cliff is
 * the whole duration. Preview only.
 */
export function cliffShare(total: bigint, cliffSecs: number, durationSecs: number): bigint {
  if (cliffSecs <= 0) return BigInt(0);
  if (cliffSecs >= durationSecs) return total;
  return (total * BigInt(cliffSecs)) / BigInt(durationSecs);
}

export function daysLabel(secs: number): string {
  const d = secs / DAY;
  const n = Number.isInteger(d) ? d : Math.round(d * 10) / 10;
  return `${n} ${n === 1 ? "day" : "days"}`;
}

export interface ScheduleTerms {
  total_zec: string;
  start_at: number;
  end_at: number;
  duration_secs: number;
  cliff_secs: number;
  cliff_at: number;
  cliff_vests_zec: string;
}

/** The vesting schedule in plain words, matching the engine's rule exactly. */
export function scheduleSentences(t: ScheduleTerms): string[] {
  const total = compactZec(t.total_zec);
  const start = formatTime(t.start_at);
  const end = formatTime(t.end_at);
  if (t.cliff_secs <= 0) {
    return [`${total} ZEC vests evenly, second by second, from ${start} to ${end}.`];
  }
  if (t.cliff_secs >= t.duration_secs) {
    return [`Nothing vests until ${end}. Then all ${total} ZEC vests at once.`];
  }
  return [
    `Nothing vests during the first ${daysLabel(t.cliff_secs)} (the cliff), from ${start} until ${formatTime(t.cliff_at)}.`,
    `At the cliff, the share that would have vested over that period — ${compactZec(t.cliff_vests_zec)} ZEC — vests at once.`,
    `After that, the rest vests evenly, second by second, until ${end}, when all ${total} ZEC has vested.`,
  ];
}

export const CANCELLATION_RULE =
  "If the agreement is cancelled, vesting stops at that moment. Everything vested by then stays owed and can still be withdrawn; the rest does not vest.";

export function phaseLabel(p: VestingPhase): string {
  switch (p) {
    case "draft":
      return "Draft";
    case "not_started":
      return "Starts soon";
    case "in_cliff":
      return "In cliff";
    case "vesting":
      return "Vesting";
    case "fully_vested":
      return "Fully vested";
    case "cancelled":
      return "Cancelled";
  }
}

export function phaseTone(p: VestingPhase): "neutral" | "green" | "amber" | "accent" | "paid" {
  switch (p) {
    case "vesting":
      return "green";
    case "in_cliff":
    case "not_started":
      return "accent";
    case "fully_vested":
      return "paid";
    case "draft":
      return "amber";
    case "cancelled":
      return "neutral";
  }
}

/** Percent (0–100, two decimals) of `part` in `total`, for bar widths only. */
export function pct(part: number, total: number): number {
  if (total <= 0) return 0;
  return Number((BigInt(part) * BigInt(10_000)) / BigInt(total)) / 100;
}

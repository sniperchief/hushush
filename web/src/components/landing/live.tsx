"use client";

import { useSyncExternalStore } from "react";

const REDUCED = "(prefers-reduced-motion: reduce)";

/**
 * One shared interval per ticker, however many counters are on the page, so
 * they all stay in step. It never starts (and stops if the setting changes)
 * under prefers-reduced-motion.
 */
function createTicker(ms: number) {
  let n = 0;
  let id: ReturnType<typeof setInterval> | undefined;
  const subs = new Set<() => void>();
  const stop = () => {
    clearInterval(id);
    id = undefined;
  };
  const sync = () => {
    const reduced = window.matchMedia(REDUCED).matches;
    if (subs.size > 0 && !reduced && id === undefined) {
      id = setInterval(() => {
        n += 1;
        subs.forEach((f) => f());
      }, ms);
    } else if ((subs.size === 0 || reduced) && id !== undefined) {
      stop();
    }
  };
  const subscribe = (cb: () => void) => {
    subs.add(cb);
    const mq = window.matchMedia(REDUCED);
    mq.addEventListener("change", sync);
    sync();
    return () => {
      subs.delete(cb);
      mq.removeEventListener("change", sync);
      sync();
    };
  };
  return () =>
    useSyncExternalStore(
      subscribe,
      () => n,
      () => 0,
    );
}

const useSeconds = createTicker(1000);
const useBeat = createTicker(1700);

// Demo stream: 1.5 ZEC per 30 days from 1.25 ZEC, in whole zatoshi like the engine.
const START_ZAT = 125_000_000;
const RATE_ZAT = 150_000_000;
const PERIOD_SECS = 2_592_000;

/** Demo earnings counter, ticking once a second. Not a real balance. */
export function Earned() {
  const secs = useSeconds();
  const zat = Math.floor(START_ZAT + (secs * RATE_ZAT) / PERIOD_SECS);
  return <>{(zat / 1e8).toFixed(8)}</>;
}

// Labels follow the claim states in the product; 10 is payroll-api's default PAYOUT_CONFIRMATIONS.
const PHASES = [
  { label: "Pending · unpaid", tone: "bg-honey text-espresso", pct: 8, note: (who: string) => `Reserved for ${who}. No ZEC sent yet.` },
  { label: "Sent · 2/10 confirmations", tone: "bg-wisteria text-espresso", pct: 20, note: () => "On the network. Not paid yet." },
  { label: "Sent · 6/10 confirmations", tone: "bg-wisteria text-espresso", pct: 60, note: () => "On the network. Not paid yet." },
  { label: "Sent · 9/10 confirmations", tone: "bg-wisteria text-espresso", pct: 90, note: () => "Almost there. Still not paid." },
  { label: "Paid · payout confirmed", tone: "bg-espresso text-cream", pct: 100, note: () => "Confirmed on chain. Paid." },
] as const;

// "Paid" is held for two beats before the loop restarts.
const SEQUENCE = [0, 1, 2, 3, 4, 4];

/**
 * Demo payout moving from pending to paid: badge, progress bar and note.
 * Renders as flex children; the caller supplies the column container.
 */
export function PayoutStatus({
  who = "her",
  track = "bg-mist",
  noteClass = "text-[13px]",
  note = true,
}: {
  who?: "her" | "you";
  track?: string;
  noteClass?: string;
  note?: boolean;
}) {
  const beat = useBeat();
  const p = PHASES[SEQUENCE[beat % SEQUENCE.length] ?? 0] ?? PHASES[0];
  return (
    <>
      <span className={`tag self-start ${p.tone}`}>{p.label}</span>
      <div className={`h-1.5 overflow-hidden rounded-[3px] ${track}`}>
        <div className="h-1.5 bg-amber transition-[width] duration-500 ease-out motion-reduce:transition-none" style={{ width: `${p.pct}%` }} />
      </div>
      {note && <span className={`text-bark ${noteClass}`}>{p.note(who)}</span>}
    </>
  );
}

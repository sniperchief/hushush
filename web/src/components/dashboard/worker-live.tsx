"use client";

import { useEffect, useState } from "react";
import { earnedAt, formatZats } from "@/lib/chart-data";
import type { FlowStream } from "@/lib/payroll";

// Live worker figures, ticking every second with the same maths as the stream
// cards (LiveFigures): earned(t) from the server's as_of, never below what the
// server reported. The parent re-keys these on every refresh.

/** Seconds since mount, ticking once a second. */
function useElapsed(): number {
  const [anchor] = useState(() => Date.now());
  const [now, setNow] = useState(anchor);
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  return Math.floor((now - anchor) / 1000);
}

function liveEarned(s: FlowStream, elapsed: number): bigint {
  const server = BigInt(s.balance.earned_zats);
  if (s.status !== "active") return server;
  const e = earnedAt(s, s.balance.as_of + elapsed);
  return e > server ? e : server;
}

function figures(s: FlowStream, elapsed: number) {
  const total = BigInt(s.total_zats);
  const earned = liveEarned(s, elapsed);
  const claimed = BigInt(s.balance.claimed_zats);
  const reserved = BigInt(s.balance.reserved_zats);
  const available = earned - claimed - reserved;
  return { total, earned, claimed, reserved, available: available > 0n ? available : 0n };
}

function Kpi({ label, swatch, value, sub, tone }: { label: string; swatch?: string; value: string; sub: string; tone?: "honey" }) {
  return (
    <div className={`flex min-w-0 flex-col gap-2 rounded-xl border px-5 py-[18px] ${tone === "honey" ? "border-honey bg-honey-soft" : "border-linen bg-paper"}`}>
      <span className={`flex items-center gap-2 text-[13px] ${tone === "honey" ? "font-semi text-espresso" : "text-bark"}`}>
        {swatch && <i aria-hidden="true" className="inline-block size-2.5 rounded-[3px]" style={{ background: swatch }} />}
        {label}
      </span>
      <span className="mono truncate text-[26px] leading-[1.1] font-medium tracking-[-0.6px]">
        {value} <span className="font-landing text-sm font-normal tracking-normal text-bark">ZEC</span>
      </span>
      <span className="truncate text-[12.5px] text-bark">{sub}</span>
    </div>
  );
}

export function WorkerMetrics({ streams, lastPaid }: { streams: FlowStream[]; lastPaid: string | null }) {
  const elapsed = useElapsed();
  const f = streams.map((s) => ({ s, ...figures(s, elapsed) }));
  const ready = f.reduce((a, b) => (b.available > a.available ? b : a), f[0]!);
  const sum = (k: "total" | "earned" | "claimed" | "reserved") => f.reduce((a, x) => a + x[k], 0n);
  const reserved = sum("reserved");
  const n = streams.length;
  return (
    <div className="grid grid-cols-2 gap-4 min-[1180px]:grid-cols-4">
      <Kpi label="Ready to withdraw" tone="honey" value={formatZats(ready.available)} sub={`From ${ready.s.organization_name}`} />
      <Kpi label="Earned so far" swatch="#d08a00" value={formatZats(sum("earned"))} sub={`Across ${n} ${n === 1 ? "stream" : "streams"}`} />
      <Kpi
        label="Withdrawn"
        swatch="#7a5cc8"
        value={formatZats(sum("claimed"))}
        sub={reserved > 0n ? `${formatZats(reserved)} on its way` : lastPaid ? `Last: ${lastPaid}` : "Nothing withdrawn yet"}
      />
      <Kpi label="Still to earn" value={formatZats(sum("total") - sum("earned"))} sub={`Of ${formatZats(sum("total"))} ZEC in total`} />
    </div>
  );
}

/** One stream as a ring: earned (amber), withdrawn + on its way (purple), ticks every 20%. */
export function ProgressRing({ s }: { s: FlowStream }) {
  const elapsed = useElapsed();
  const { total, earned, claimed, reserved, available } = figures(s, elapsed);
  const C = 2 * Math.PI * 66;
  const frac = (v: bigint) => (total > 0n ? Number((v * 1_000_000n) / total) / 1_000_000 : 0);
  const pct = Math.floor(frac(earned) * 100);
  const row = (label: string, color: string, v: bigint) => (
    <div className="flex justify-between">
      <span className="inline-flex items-center gap-[7px] text-[13px] text-bark">
        <i aria-hidden="true" className="inline-block size-2.5 rounded-[3px]" style={{ background: color }} />
        {label}
      </span>
      <span className="mono">{formatZats(v)}</span>
    </div>
  );
  return (
    <>
      <div className="flex justify-center py-1">
        <svg viewBox="0 0 180 180" width="188" height="188" role="img" aria-label={`${s.organization_name} stream: ${pct}% earned`}>
          <g transform="rotate(-90 90 90)" fill="none" strokeWidth="18">
            <circle cx="90" cy="90" r="66" stroke="#efece5" />
            <circle cx="90" cy="90" r="66" stroke="#d08a00" strokeDasharray={`${(frac(earned) * C).toFixed(1)} ${C}`} />
            <circle cx="90" cy="90" r="66" stroke="#7a5cc8" strokeDasharray={`${(frac(claimed + reserved) * C).toFixed(1)} ${C}`} />
          </g>
          <g stroke="#ffffff" strokeWidth="3">
            {[0, 72, 144, 216, 288].map((deg) => (
              <line key={deg} x1="90" y1="12" x2="90" y2="36" transform={`rotate(${deg} 90 90)`} />
            ))}
          </g>
          <text x="90" y="92" textAnchor="middle" fontSize="32" fontWeight="500" fill="#261b07" style={{ fontFamily: "var(--font-plex)" }}>
            {pct}%
          </text>
          <text x="90" y="113" textAnchor="middle" fontSize="12" fill="#61594a">
            earned
          </text>
        </svg>
      </div>
      <div className="flex flex-col gap-2 text-[13.5px]">
        {row("Withdrawn", "#7a5cc8", claimed)}
        {row("Available", "#d08a00", available)}
        {row("Still to earn", "#efece5", total - earned)}
      </div>
      <p className="m-0 border-t border-mist pt-3 text-[12.5px] leading-[1.45] text-bark">
        White ticks mark every 20%. Withdraw unlocks each time 20% of the total is available.
      </p>
    </>
  );
}

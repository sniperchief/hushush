"use client";

import { useState } from "react";
import { dayLabel, formatZats, initials, type StatusCounts } from "@/lib/chart-data";
import { avatarTone } from "./avatar";

// Employer charts: streams by status (donut), each worker's stream (bars) and
// withdrawals per week. Amounts are zatoshi numbers (below 2^53).

const zec = (z: number) => formatZats(BigInt(Math.round(z)));

export function Legend({ items }: { items: { label: string; color?: string; stripe?: boolean }[] }) {
  return (
    <div className="flex flex-wrap gap-x-4 gap-y-2">
      {items.map((i) => (
        <span key={i.label} className="inline-flex items-center gap-[7px] text-[13px] text-bark">
          <i aria-hidden="true" className={`inline-block size-2.5 flex-none rounded-[3px] ${i.stripe ? "stripe-held" : ""}`} style={i.color ? { background: i.color } : undefined} />
          {i.label}
        </span>
      ))}
    </div>
  );
}

const STATUS = [
  { key: "active", label: "Active", color: "#d08a00" },
  { key: "withdrawing", label: "Withdrawing", color: "#7a5cc8" },
  { key: "ended", label: "Ended", color: "#d5d2cd" },
  { key: "waiting", label: "Waiting to join", color: "#f8da9d" },
] as const;

export function StatusDonut({ counts }: { counts: StatusCounts }) {
  const total = counts.active + counts.withdrawing + counts.ended + counts.waiting;
  const segs = STATUS.filter((s) => counts[s.key] > 0);
  const C = 2 * Math.PI * 64;
  const gap = segs.length > 1 ? 2 : 0;
  const arcs = segs.map((s, i) => {
    const before = segs.slice(0, i).reduce((a, x) => a + counts[x.key], 0);
    return { ...s, len: (counts[s.key] / total) * C, offset: (before / total) * C };
  });
  const label = `${total} streams: ${segs.map((s) => `${counts[s.key]} ${s.label.toLowerCase()}`).join(", ")}`;
  return (
    <>
      <div className="flex justify-center py-2">
        <svg viewBox="0 0 180 180" width="190" height="190" role="img" aria-label={label}>
          <g transform="rotate(-90 90 90)" fill="none" strokeWidth="22">
            {total === 0 && <circle cx="90" cy="90" r="64" stroke="#efece5" />}
            {arcs.map((s) => (
              <circle key={s.key} cx="90" cy="90" r="64" stroke={s.color} strokeDasharray={`${Math.max(0, s.len - gap)} ${C}`} strokeDashoffset={-s.offset} />
            ))}
          </g>
          <text x="90" y="88" textAnchor="middle" fontSize="34" fontWeight="500" fill="#261b07" style={{ fontFamily: "var(--font-plex)" }}>
            {total}
          </text>
          <text x="90" y="110" textAnchor="middle" fontSize="12.5" fill="#61594a">
            {total === 1 ? "stream" : "streams"}
          </text>
        </svg>
      </div>
      <div className="grid grid-cols-2 gap-x-4 gap-y-2.5">
        {segs.map((s) => (
          <span key={s.key} className="inline-flex items-center gap-[7px] text-[13px] text-espresso">
            <i aria-hidden="true" className="inline-block size-2.5 flex-none rounded-[3px]" style={{ background: s.color }} />
            {s.label} ({counts[s.key]})
          </span>
        ))}
      </div>
    </>
  );
}

export interface WorkerBar {
  id: string;
  name: string;
  total: number;
  earned: number;
  claimed: number;
  reserved: number;
}

/** One row per stream; bar length = stream total relative to the largest. */
export function WorkerBars({ rows }: { rows: WorkerBar[] }) {
  const max = Math.max(1, ...rows.map((r) => r.total));
  const pct = (v: number, of: number) => `${of > 0 ? Math.max(0, (v / of) * 100) : 0}%`;
  return (
    <div className="flex flex-1 flex-col justify-center gap-[22px]">
      {rows.map((r) => {
        const notWithdrawn = Math.max(0, r.earned - r.claimed - r.reserved);
        const title = `${r.name}: earned ${zec(r.earned)} of ${zec(r.total)} ZEC, withdrawn ${zec(r.claimed)}${r.reserved > 0 ? `, ${zec(r.reserved)} on its way` : ""}`;
        return (
          <div key={r.id} title={title} className="grid grid-cols-[minmax(0,130px)_1fr_auto] items-center gap-3.5">
            <span className="flex min-w-0 items-center gap-2 text-sm font-semi">
              <span aria-hidden="true" className={`flex size-[26px] flex-none items-center justify-center rounded-full text-[10px] font-semi ${avatarTone(r.name)}`}>
                {initials(r.name)}
              </span>
              <span className="truncate">{r.name}</span>
            </span>
            <div style={{ width: pct(r.total, max) }}>
              <div className="relative h-3 overflow-hidden rounded bg-mist" role="img" aria-label={title}>
                <span className="absolute inset-y-0 left-0 bg-withdrawn" style={{ width: pct(r.claimed, r.total) }} />
                <span className="stripe-held absolute inset-y-0" style={{ left: pct(r.claimed, r.total), width: pct(r.reserved, r.total) }} />
                <span className="absolute inset-y-0 bg-earned" style={{ left: pct(r.claimed + r.reserved, r.total), width: pct(notWithdrawn, r.total) }} />
              </div>
            </div>
            <span className="mono text-right text-[12.5px] whitespace-nowrap text-bark">
              <b className="font-medium text-espresso">{zec(r.earned)}</b> / {zec(r.total)}
            </span>
          </div>
        );
      })}
    </div>
  );
}

export interface WeekBar {
  weekStart: number;
  paid: number;
  inFlight: number;
  byWorker: { name: string; paid: number; inFlight: number }[];
}

/** A round step so that 3 steps cover `v`. */
function niceStep(v: number): number {
  if (!(v > 0)) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= v) return m * p;
  return 10 * p;
}

const firstName = (n: string) => n.trim().split(/\s+/)[0] ?? n;

export function WeeklyBars({ weeks }: { weeks: WeekBar[] }) {
  const [hover, setHover] = useState<number | null>(null);
  const step = niceStep(Math.max(...weeks.map((w) => (w.paid + w.inFlight) / 1e8)) / 3);
  const max = step * 3;
  const left = 40, right = 510, top = 20, base = 170;
  const Y = (z: number) => base - (z / 1e8 / max) * (base - top);
  const slot = (right - left) / weeks.length;
  const cx = (i: number) => left + slot * (i + 0.5);
  const tip = (w: WeekBar) => {
    const who = (key: "paid" | "inFlight") =>
      w.byWorker
        .filter((b) => b[key] > 0)
        .map((b) => `${firstName(b.name)}${w.byWorker.filter((x) => x[key] > 0).length > 1 ? ` ${zec(b[key])}` : ""}`)
        .join(", ");
    const parts = [];
    if (w.paid > 0) parts.push(`${zec(w.paid)} ZEC paid (${who("paid")})`);
    if (w.inFlight > 0) parts.push(`${zec(w.inFlight)} ZEC on its way (${who("inFlight")})`);
    return `Week of ${dayLabel(w.weekStart)}: ${parts.length ? parts.join("; ") : "no withdrawals"}`;
  };
  const bar = (x: number, yTop: number, y2: number, rounded: boolean) => {
    const w = 56, r = 4, x0 = x - w / 2;
    if (!rounded || y2 - yTop < r) return `M${x0} ${y2}V${yTop}h${w}V${y2}z`;
    return `M${x0} ${y2}V${yTop + r}a${r} ${r} 0 0 1 ${r}-${r}h${w - 2 * r}a${r} ${r} 0 0 1 ${r} ${r}V${y2}z`;
  };
  return (
    <>
      <div className="relative">
        <svg viewBox="0 0 520 220" role="img" aria-label={`Withdrawals per week: ${weeks.map(tip).join(". ")}`} className="block h-auto w-full">
          <defs>
            <pattern id="held-stripes" width="8" height="8" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
              <rect width="8" height="8" fill="#c9b8ef" />
              <rect width="4" height="8" fill="#7a5cc8" />
            </pattern>
          </defs>
          <g stroke="#efece5">
            {[1, 2, 3].map((k) => (
              <line key={k} x1={left} y1={Y(step * k * 1e8)} x2={right} y2={Y(step * k * 1e8)} />
            ))}
          </g>
          <line x1={left} y1={base} x2={right} y2={base} stroke="#d5d2cd" />
          <g fontSize="11" fill="#8f897e" textAnchor="end" style={{ fontFamily: "var(--font-plex)" }}>
            {[0, 1, 2, 3].map((k) => (
              <text key={k} x={left - 8} y={Y(step * k * 1e8) + 4}>
                {Number((step * k).toFixed(6))}
              </text>
            ))}
          </g>
          {weeks.map((w, i) => {
            const total = w.paid + w.inFlight;
            const x = cx(i);
            const yPaid = Y(w.paid);
            const yTop = Y(total);
            return (
              <g key={w.weekStart}>
                {total === 0 ? (
                  <rect x={x - 28} y={base - 2} width="56" height="2" rx="1" fill="#d5d2cd" />
                ) : (
                  <>
                    {w.paid > 0 && <path d={bar(x, yPaid, base, w.inFlight === 0)} fill="#7a5cc8" />}
                    {w.inFlight > 0 && <path d={bar(x, yTop, yPaid, true)} fill="url(#held-stripes)" />}
                  </>
                )}
                <text x={x} y={total === 0 ? base - 10 : yTop - 7} textAnchor="middle" fontSize="12" fill="#261b07" style={{ fontFamily: "var(--font-plex)" }}>
                  {zec(total)}
                </text>
                <text x={x} y="192" textAnchor="middle" fontSize="11" fill="#8f897e">
                  {dayLabel(w.weekStart)}
                </text>
                <rect
                  x={x - slot / 2 + 4}
                  y="10"
                  width={slot - 8}
                  height="190"
                  fill="transparent"
                  className="cursor-pointer"
                  onPointerEnter={() => setHover(i)}
                  onPointerLeave={() => setHover(null)}
                />
              </g>
            );
          })}
        </svg>
        {hover !== null && (
          <div
            role="tooltip"
            className="pointer-events-none absolute top-1.5 -translate-x-1/2 rounded-lg bg-espresso px-2.5 py-2 text-[12.5px] leading-normal whitespace-nowrap text-cream shadow-[0_8px_16px_rgba(38,27,7,.18)]"
            style={{ left: `${Math.min(80, Math.max(20, (cx(hover) / 520) * 100))}%` }}
          >
            {tip(weeks[hover]!)}
          </div>
        )}
      </div>
      <Legend
        items={[
          { label: "Paid", color: "#7a5cc8" },
          { label: "On its way", stripe: true },
        ]}
      />
    </>
  );
}

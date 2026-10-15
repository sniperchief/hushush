"use client";

import { useState } from "react";
import { dayLabel, dayTimeLabel, formatZats, niceMax } from "@/lib/chart-data";

// Earned (solid line + soft fill) and withdrawn (stepped line) over time, with
// an optional dashed projection. Amounts arrive as zatoshi numbers (always
// below 2^53) because BigInt can't cross from server to client components.

export interface ChartPoint {
  t: number;
  earned: number;
  withdrawn: number;
}

interface Geometry {
  w: number;
  h: number;
  left: number;
  right: number;
  top: number;
  base: number;
  labelY: number;
  font: number;
  grid: number;
  compact: boolean;
}

const DESKTOP: Geometry = { w: 680, h: 230, left: 44, right: 672, top: 20, base: 188, labelY: 210, font: 11, grid: 4, compact: false };
const PHONE: Geometry = { w: 340, h: 150, left: 30, right: 336, top: 10, base: 118, labelY: 140, font: 10, grid: 2, compact: true };

const zec = (z: number) => formatZats(BigInt(Math.round(z)));
const tick = (v: number) => String(Number(v.toFixed(6)));

interface Props {
  points: ChartPoint[];
  projection?: { t: number; earned: number }[];
  from: number;
  to: number;
  now: number;
  /** employer: "Not yet withdrawn" readout + line-end labels. worker: projection + Today marker. */
  mode: "employer" | "worker";
  ariaLabel: string;
}

function Plot({ g, points, projection, from, to, now, mode, ariaLabel, hover, setHover }: Props & { g: Geometry; hover: number | null; setHover: (i: number | null) => void }) {
  const last = points.length - 1;
  const maxZec = niceMax(Math.max(...points.map((p) => p.earned), ...(projection ?? []).map((p) => p.earned), 1) / 1e8);
  const X = (t: number) => g.left + ((t - from) / (to - from)) * (g.right - g.left);
  const Y = (z: number) => g.base - (z / 1e8 / maxZec) * (g.base - g.top);
  const f = (n: number) => n.toFixed(1);

  const earnD = points.map((p, i) => `${i ? "L" : "M"}${f(X(p.t))} ${f(Y(p.earned))}`).join(" ");
  const areaD = `${earnD} L${f(X(points[last]!.t))} ${g.base} L${f(X(points[0]!.t))} ${g.base} Z`;
  let wdD = `M${f(X(points[0]!.t))} ${f(Y(points[0]!.withdrawn))}`;
  for (let i = 1; i <= last; i++) wdD += ` L${f(X(points[i]!.t))} ${f(Y(points[i - 1]!.withdrawn))} L${f(X(points[i]!.t))} ${f(Y(points[i]!.withdrawn))}`;
  const projD = projection?.length ? projection.map((p, i) => `${i ? "L" : "M"}${f(X(p.t))} ${f(Y(p.earned))}`).join(" ") : null;
  const projEnd = projection?.[projection.length - 1];

  // X labels: the first day, then weekly, then the last day (and "Today" for workers).
  const nowX = X(now);
  const labels: { x: number; text: string; anchor: "start" | "middle" | "end"; bold?: boolean }[] = [{ x: X(from), text: dayLabel(from), anchor: "start" }];
  const endText = mode === "worker" ? dayLabel(to - 1) : g.compact ? "Today" : dayLabel(to);
  const step = g.compact ? (to - from) / 2 : 7 * 86_400;
  for (let t = from + step; t < to; t += step) {
    const x = X(t);
    const clear = (other: number) => Math.abs(x - other) > (g.right - g.left) * 0.09;
    if (clear(X(to)) && (mode !== "worker" || clear(nowX))) labels.push({ x, text: dayLabel(t), anchor: "middle" });
  }
  labels.push({ x: X(to), text: endText, anchor: "end" });
  if (mode === "worker" && now > from && now < to) labels.push({ x: nowX, text: "Today", anchor: "middle", bold: true });

  const h = hover ?? last;
  const hp = points[h]!;
  const grid = Array.from({ length: g.grid }, (_, k) => ((k + 1) / g.grid) * maxZec);

  return (
    <div className="relative">
      <svg viewBox={`0 0 ${g.w} ${g.h}`} role="img" aria-label={ariaLabel} className="block h-auto w-full">
        <defs>
          <linearGradient id={`earn-fill-${g.w}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#f9a600" stopOpacity=".28" />
            <stop offset="1" stopColor="#f9a600" stopOpacity="0" />
          </linearGradient>
        </defs>
        {mode === "worker" && now < to && <rect x={f(nowX)} y={g.top - 6} width={f(g.right - nowX)} height={g.base - g.top + 6} fill="#fbfaf8" />}
        <g stroke="#efece5">
          {grid.map((v) => (
            <line key={v} x1={g.left} y1={f(Y(v * 1e8))} x2={g.right} y2={f(Y(v * 1e8))} />
          ))}
        </g>
        <line x1={g.left} y1={g.base} x2={g.right} y2={g.base} stroke="#d5d2cd" />
        <g fontSize={g.font} fill="#8f897e" textAnchor="end" style={{ fontFamily: "var(--font-plex)" }}>
          {[0, ...grid].map((v) => (
            <text key={v} x={g.left - 8} y={f(Y(v * 1e8) + 4)}>
              {tick(v)}
            </text>
          ))}
        </g>
        <g fontSize={g.font} fill="#8f897e">
          {labels.map((l) => (
            <text key={`${l.text}-${l.x}`} x={f(l.x)} y={g.labelY} textAnchor={l.anchor} fill={l.bold ? "#261b07" : undefined} fontWeight={l.bold ? 600 : undefined}>
              {l.text}
            </text>
          ))}
        </g>
        <path d={areaD} fill={`url(#earn-fill-${g.w})`} />
        <path d={earnD} fill="none" stroke="#d08a00" strokeWidth="2" strokeLinejoin="round" />
        {projD && <path d={projD} fill="none" stroke="#d08a00" strokeWidth="2" strokeDasharray="5 5" />}
        <path d={wdD} fill="none" stroke="#7a5cc8" strokeWidth="2" strokeLinejoin="round" />
        {projEnd && (
          <text x={g.right - 4} y={f(Math.max(g.top + g.font, Y(projEnd.earned) - 10))} fontSize={g.font} fill="#61594a" textAnchor="end">
            Projected {zec(projEnd.earned)} by {dayLabel(to - 1)}
          </text>
        )}
        {hover !== null && <line x1={f(X(hp.t))} y1={g.top - 6} x2={f(X(hp.t))} y2={g.base} stroke="#261b07" strokeDasharray="3 3" opacity=".5" />}
        {(hover !== null || g.compact || mode === "worker") && (
          <>
            <circle cx={f(X(hp.t))} cy={f(Y(hp.earned))} r={g.compact ? 4 : 5} fill="#d08a00" stroke="#fff" strokeWidth="2" />
            <circle cx={f(X(hp.t))} cy={f(Y(hp.withdrawn))} r={g.compact ? 4 : 5} fill="#7a5cc8" stroke="#fff" strokeWidth="2" />
          </>
        )}
        {mode === "employer" && hover === null && !g.compact && (
          <g fontSize={g.font} fill="#61594a" textAnchor="end">
            <text x={g.right + 4} y={f(Y(points[last]!.earned) - 10)}>
              Earned
            </text>
            <text x={g.right + 4} y={f(Math.min(g.base - 4, Y(points[last]!.withdrawn) + 18))}>
              Withdrawn
            </text>
          </g>
        )}
      </svg>
      {/* Maps the pointer to the nearest day (past days only). */}
      <div
        aria-hidden="true"
        className="absolute top-0 cursor-crosshair"
        style={{ left: `${(g.left / g.w) * 100}%`, right: `${((g.w - g.right) / g.w) * 100}%`, bottom: `${((g.h - g.base) / g.h) * 100}%` }}
        onPointerMove={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          const t = from + Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * (to - from);
          let best = 0;
          points.forEach((p, i) => {
            if (Math.abs(p.t - t) < Math.abs(points[best]!.t - t)) best = i;
          });
          setHover(best === last ? null : best);
        }}
        onPointerLeave={() => setHover(null)}
      />
    </div>
  );
}

function Swatch({ color }: { color: string }) {
  return <i aria-hidden="true" className="inline-block size-2.5 flex-none rounded-[3px]" style={{ background: color }} />;
}

export function EarningsChart(props: Props) {
  const [hover, setHover] = useState<number | null>(null);
  const last = props.points.length - 1;
  const p = props.points[hover ?? last]!;
  const readout = (label: string, value: number, color?: string) => (
    <div className="flex flex-col gap-0.5">
      <span className="inline-flex items-center gap-[7px] text-[12.5px] text-bark sm:text-[13px]">
        {color && <Swatch color={color} />}
        {label}
      </span>
      <span className="mono text-lg font-medium sm:text-xl">{zec(value)}</span>
    </div>
  );
  return (
    <>
      <div className="flex flex-wrap items-end gap-x-7 gap-y-2">
        {readout("Earned", p.earned, "#d08a00")}
        {readout("Withdrawn", p.withdrawn, "#7a5cc8")}
        {props.mode === "employer" && (
          <>
            <div aria-hidden="true" className="w-px self-stretch bg-linen max-sm:hidden" />
            <div className="max-sm:hidden">{readout("Not yet withdrawn", p.earned - p.withdrawn)}</div>
          </>
        )}
        <span className="ml-auto text-[12.5px] text-bark">
          {hover === null ? "Today" : dayTimeLabel(p.t)}
        </span>
      </div>
      <div className="max-sm:hidden">
        <Plot {...props} g={DESKTOP} hover={hover} setHover={setHover} />
      </div>
      <div className="sm:hidden">
        <Plot {...props} g={PHONE} hover={hover} setHover={setHover} />
      </div>
    </>
  );
}

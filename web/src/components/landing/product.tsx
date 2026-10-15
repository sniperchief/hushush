import type { ReactNode } from "react";
import { Dot, Mark, Wordmark } from "./ui";

// Everything in this mockup is demo data, not a real organization's balances.

const BALANCES = [
  { label: "Earned", value: "39.2635", note: "Accrued over active time" },
  { label: "Reserved", value: "2.0000", note: "Pending claims, not yet paid" },
  { label: "Claimed", value: "29.3500", note: "Confirmed payouts only" },
  { label: "Available", value: "7.9135", note: "Earned − reserved − claimed", strong: true },
];

const STREAMS = [
  { name: "Marco Lindqvist", rate: "9", per: "ZEC per 30 days", paused: false, earned: "8.66550000", reserved: "1.50000000", claimed: "3.00000000", available: "4.16550000" },
  { name: "Ava Nolan", rate: "1.5", per: "ZEC per 30 days", paused: false, earned: "3.74800000", reserved: "0.50000000", claimed: "2.00000000", available: "1.24800000" },
  { name: "Kyle Mercer", rate: "1.2", per: "ZEC per week", paused: true, earned: "4.20000000", reserved: "0.00000000", claimed: "3.60000000", available: "0.60000000" },
];

const icon = { width: 16, height: 16, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.5, strokeLinecap: "round", strokeLinejoin: "round", "aria-hidden": true } as const;

const SIDE: { label: string; current?: boolean; svg: ReactNode }[] = [
  { label: "Dashboard", svg: <svg {...icon}><rect x="3" y="3" width="7" height="7" rx="1.5" /><rect x="14" y="3" width="7" height="7" rx="1.5" /><rect x="3" y="14" width="7" height="7" rx="1.5" /><rect x="14" y="14" width="7" height="7" rx="1.5" /></svg> },
  { label: "Payroll streams", current: true, svg: <svg {...icon}><path d="M3 8c3-3 6 3 9 0s6 3 9 0" /><path d="M3 16c3-3 6 3 9 0s6 3 9 0" /></svg> },
  { label: "Claims", svg: <svg {...icon}><path d="M12 3v12" /><path d="m7 10 5 5 5-5" /><path d="M4 19h16" /></svg> },
  { label: "Members", svg: <svg {...icon}><circle cx="9" cy="8" r="3.5" /><path d="M2.5 20c.8-3.5 3.4-5.5 6.5-5.5s5.7 2 6.5 5.5" /></svg> },
  { label: "Payout wallet", svg: <svg {...icon}><rect x="3" y="6" width="18" height="13" rx="2" /><path d="M3 10h18M16 14.5h2" /></svg> },
];

const th = "eyebrow border-b border-linen px-3.5 py-2.5 text-left font-mid whitespace-nowrap text-bark";
const td = "border-b border-mist px-3.5 py-[13px] whitespace-nowrap group-last:border-b-0";

function PausedTag({ small = false }: { small?: boolean }) {
  return <span className={`tag bg-linen text-bark ${small ? "px-1.5 py-0.5 text-[10.5px]" : ""}`}>{small ? "Paused" : "❚❚ Paused"}</span>;
}

function BrowserChrome() {
  const chev = { width: 15, height: 15, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 2, strokeLinecap: "round", strokeLinejoin: "round" } as const;
  return (
    <div className="border-b border-linen bg-mist">
      <div className="flex items-end gap-4 px-3.5 pt-2.5">
        <div className="flex gap-[7px] pb-3">
          <span className="size-[11px] rounded-full bg-terracotta" />
          <span className="size-[11px] rounded-full bg-amber" />
          <span className="size-[11px] rounded-full bg-stone" />
        </div>
        <div className="flex max-w-[260px] min-w-0 items-center gap-2 overflow-hidden rounded-t-lg border border-b-0 border-linen bg-paper px-3.5 py-2 text-[12.5px] whitespace-nowrap">
          <Mark className="h-3.5" />
          <span className="truncate">Kestrel Collective · hushush</span>
        </div>
      </div>
      <div className="flex items-center gap-2.5 border-t border-linen bg-paper px-3.5 py-2">
        <div className="flex gap-2.5 text-bark">
          <svg {...chev}><path d="M15 6l-6 6 6 6" /></svg>
          <svg {...chev}><path d="m9 6 6 6-6 6" /></svg>
          <svg {...chev}><path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7" /></svg>
        </div>
        <div className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden rounded-lg bg-cream px-3 py-1.5 text-[12.5px] whitespace-nowrap text-bark">
          <LockIcon size={12} />
          {/* Swap in the real domain once there is one. */}
          <span className="truncate">
            <span className="text-espresso">[your-domain]</span>/app/orgs/kestrel-collective
          </span>
        </div>
      </div>
    </div>
  );
}

function LockIcon({ size }: { size: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" className="flex-none">
      <rect x="5" y="11" width="14" height="9" rx="2" />
      <path d="M8 11V8a4 4 0 0 1 8 0v3" />
    </svg>
  );
}

function DesktopShot() {
  return (
    <div
      role="img"
      aria-label="Employer dashboard preview with demo data: balances, an earnings chart, the payout wallet and three payroll streams"
      className="l-card overflow-hidden text-left text-sm shadow-shot max-md:hidden"
    >
      <BrowserChrome />
      <div className="flex flex-wrap">
        <div className="flex max-w-full flex-[1_1_210px] flex-col gap-5 border-r border-linen bg-chalk px-3 py-5">
          <Wordmark className="mx-2 mt-0.5 h-[22px] self-start" alt="" />
          <div className="flex items-center gap-2.5 rounded-lg border border-linen bg-paper p-2.5">
            <span className="flex size-7 items-center justify-center rounded-md bg-espresso text-[11px] font-semi text-cream">KC</span>
            <span className="flex min-w-0 flex-col">
              <span className="text-[13.5px] font-mid">Kestrel Collective</span>
              <span className="text-xs text-bark">Owner</span>
            </span>
          </div>
          <div className="flex flex-col gap-0.5">
            {SIDE.map((s) => (
              <span key={s.label} className={`flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm ${s.current ? "bg-linen font-mid" : ""}`}>
                {s.svg}
                {s.label}
              </span>
            ))}
          </div>
          <div className="mt-auto flex items-center gap-2 rounded-lg bg-honey px-2.5 py-2 text-xs font-mid">
            <span className="size-1.5 rounded-xs bg-espresso" />
            Zcash testnet
          </div>
        </div>

        <div className="flex min-w-0 flex-[999_1_620px] flex-col gap-[18px] px-[clamp(16px,2.4vw,32px)] pt-6 pb-7">
          <div className="flex flex-wrap items-end justify-between gap-3">
            <span className="text-2xl leading-[1.13] font-semi tracking-[-0.31px]">Payroll streams</span>
            <div className="flex flex-wrap items-center gap-2">
              <span className="mono text-xs text-bark">Balances as of 14:05:12 UTC</span>
              <span className="tag bg-linen text-bark">Demo data</span>
              <span className="btn-amber rounded-lg px-3 py-[7px] text-[13.5px] font-mid">New stream</span>
            </div>
          </div>
          <div className="grid grid-cols-[repeat(auto-fit,minmax(min(160px,100%),1fr))] gap-3">
            {BALANCES.map((b) => (
              <div key={b.label} className={`flex flex-col gap-1.5 rounded-lg border p-4 ${b.strong ? "border-espresso bg-ivory" : "border-linen"}`}>
                <span className={`text-xs ${b.strong ? "font-mid" : "text-bark"}`}>{b.label}</span>
                <span className="mono text-xl font-medium">{b.value}</span>
                <span className="text-xs text-bark">{b.note}</span>
              </div>
            ))}
          </div>
          <div className="flex flex-wrap gap-3">
            <div className="flex min-w-0 flex-[3_1_360px] flex-col gap-2 rounded-lg border border-linen px-4 pt-4 pb-2.5">
              <div className="flex flex-wrap justify-between gap-2">
                <span className="text-sm font-mid">Marco Lindqvist · earned over time</span>
                <span className="text-xs text-bark">Flat while paused</span>
              </div>
              <svg viewBox="0 0 520 150" className="block h-auto w-full">
                <defs>
                  <linearGradient id="shot-fill" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0" stopColor="#f9a600" stopOpacity=".32" />
                    <stop offset="1" stopColor="#f9a600" stopOpacity="0" />
                  </linearGradient>
                </defs>
                <g stroke="#efece5">
                  <line x1="0" y1="30" x2="520" y2="30" />
                  <line x1="0" y1="70" x2="520" y2="70" />
                  <line x1="0" y1="110" x2="520" y2="110" />
                </g>
                <rect x="250" y="8" width="60" height="122" fill="#efece5" opacity=".7" />
                <text x="280" y="22" textAnchor="middle" fontSize="10" fill="#61594a">paused</text>
                <path d="M0 130 L250 70 L310 70 L520 20 L520 130 Z" fill="url(#shot-fill)" />
                <path d="M0 130 L250 70 L310 70 L520 20" fill="none" stroke="#e89b01" strokeWidth="2.2" strokeLinejoin="round" />
                <circle cx="520" cy="20" r="3.5" fill="#261b07" />
                <g fontSize="10" fill="#61594a">
                  <text x="0" y="146">Sep 16</text>
                  <text x="250" y="146" textAnchor="middle">Sep 30</text>
                  <text x="520" y="146" textAnchor="end">Today</text>
                </g>
              </svg>
            </div>
            <div className="flex min-w-0 flex-[2_1_230px] flex-col gap-2.5 rounded-lg border border-linen p-4">
              <div className="flex items-center justify-between gap-2">
                <span className="text-sm font-mid">Payout wallet</span>
                <span className="tag bg-honey">Testnet</span>
              </div>
              <div>
                <div className="eyebrow text-bark">Spendable</div>
                <div className="mono text-[22px] font-medium">42.5 TAZ</div>
              </div>
              <p className="m-0 text-xs text-bark">Payouts count as paid after 10 confirmations.</p>
            </div>
          </div>
          <div className="overflow-hidden rounded-lg border border-linen">
            <div className="overflow-x-auto">
              <table className="w-full text-[13.5px] leading-[1.25]">
                <thead>
                  <tr>
                    <th className={th}>Contributor</th>
                    <th className={th}>Rate</th>
                    <th className={th}>Status</th>
                    <th className={`${th} text-right`}>Earned</th>
                    <th className={`${th} text-right`}>Reserved</th>
                    <th className={`${th} text-right`}>Claimed</th>
                    <th className={`${th} text-right`}>Available</th>
                  </tr>
                </thead>
                <tbody>
                  {STREAMS.map((s) => (
                    <tr key={s.name} className="group">
                      <td className={`${td} font-mid`}>{s.name}</td>
                      <td className={`${td} mono`}>
                        {s.rate} <span className="font-landing text-bark">{s.per}</span>
                      </td>
                      <td className={td}>
                        {s.paused ? (
                          <PausedTag />
                        ) : (
                          <span className="tag border border-linen bg-paper">
                            <Dot />
                            Active
                          </span>
                        )}
                      </td>
                      <td className={`${td} mono text-right`}>{s.earned}</td>
                      <td className={`${td} mono text-right`}>{s.reserved}</td>
                      <td className={`${td} mono text-right`}>{s.claimed}</td>
                      <td className={`${td} mono text-right font-medium`}>{s.available}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function PhoneShot() {
  return (
    <div
      role="img"
      aria-label="Employer dashboard preview with demo data: balances, an earnings chart and three payroll streams"
      className="l-card overflow-hidden rounded-xl shadow-shot-sm md:hidden"
    >
      <div className="flex items-center gap-2 border-b border-linen bg-mist px-3 py-2.5">
        <div className="flex flex-1 items-center gap-1.5 overflow-hidden rounded-full bg-paper px-2.5 py-[7px] text-[11.5px] whitespace-nowrap text-bark">
          <LockIcon size={11} />
          <span className="truncate">
            <span className="text-espresso">[your-domain]</span>/app/orgs/kestrel-collective
          </span>
        </div>
      </div>
      <div className="flex flex-col gap-3 p-3.5 text-[13px]">
        <div className="flex items-center justify-between">
          <Wordmark className="h-4" alt="" />
          <span className="tag bg-linen text-[11.5px] text-bark">Demo data</span>
        </div>
        <div className="flex items-end justify-between gap-2">
          <div>
            <div className="text-[11px] text-bark">Kestrel Collective</div>
            <div className="text-[19px] font-semi tracking-[-0.3px]">Payroll streams</div>
          </div>
          <span className="btn-amber rounded-lg px-2.5 py-[7px] text-[12.5px] font-mid">New stream</span>
        </div>
        <div className="grid grid-cols-2 gap-2">
          {BALANCES.map((b) => (
            <div key={b.label} className={`rounded-lg border p-2.5 ${b.strong ? "border-espresso bg-ivory" : "border-linen"}`}>
              <div className={`text-[11px] ${b.strong ? "font-mid" : "text-bark"}`}>{b.label}</div>
              <div className="mono mt-[3px] text-base">{b.value}</div>
            </div>
          ))}
        </div>
        <div className="rounded-lg border border-linen px-2.5 pt-2.5 pb-1.5">
          <div className="flex justify-between text-[11.5px]">
            <span className="font-mid">Marco · earned over time</span>
            <span className="text-bark">flat while paused</span>
          </div>
          <svg viewBox="0 0 300 80" className="mt-1.5 block h-auto w-full">
            <defs>
              <linearGradient id="phone-fill" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0" stopColor="#f9a600" stopOpacity=".3" />
                <stop offset="1" stopColor="#f9a600" stopOpacity="0" />
              </linearGradient>
            </defs>
            <rect x="140" y="4" width="36" height="72" fill="#efece5" />
            <path d="M0 76 L140 42 L176 42 L300 10 L300 76 Z" fill="url(#phone-fill)" />
            <path d="M0 76 L140 42 L176 42 L300 10" fill="none" stroke="#e89b01" strokeWidth="2" />
            <circle cx="298" cy="10" r="3" fill="#261b07" />
          </svg>
        </div>
        <div className="flex flex-col">
          {STREAMS.map((s) => (
            <div key={s.name} className="flex items-center justify-between border-b border-mist py-2.5 last:border-b-0">
              <div>
                <div className="font-mid">
                  {s.name} {s.paused && <PausedTag small />}
                </div>
                <div className="mono text-[11.5px] text-bark">
                  {s.rate} ZEC / {s.per.endsWith("week") ? "week" : "30 days"}
                </div>
              </div>
              <div className="text-right">
                <div className="mono text-[13px]">{s.available}</div>
                <div className="text-[11px] text-bark">available</div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

export function ProductBand() {
  return (
    // On desktop the band starts partway down the mockup, so its top sits on the page background.
    <div className="mt-12 border-y border-espresso/8 bg-honey-band md:mt-[clamp(56px,7vw,88px)] md:border-t-0 md:bg-[linear-gradient(to_bottom,var(--color-cream)_280px,var(--color-honey-band)_280px)]">
      <section aria-label="Product preview with demo data" className="mx-auto max-w-[1280px] px-4 py-8 md:px-[clamp(12px,3vw,24px)] md:pt-0 md:pb-[clamp(40px,6vw,72px)]">
        <DesktopShot />
        <PhoneShot />
      </section>
    </div>
  );
}

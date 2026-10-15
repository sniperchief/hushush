import type { ReactNode } from "react";
import { ArrowRight, SectionIntro, containerClass } from "./ui";

const DAYS = Array.from({ length: 30 }, (_, i) => i + 1);
const EARNED_DAYS = 18;

function dayClass(day: number) {
  if (day < EARNED_DAYS) return "bg-amber";
  if (day === EARNED_DAYS) return "bg-espresso sm:shadow-[0_0_0_3px_var(--color-honey)]";
  if (day === 30) return "flex items-center justify-center border-2 border-dashed border-espresso bg-paper text-[7.5px] font-semi sm:text-[9px]";
  return "border border-linen bg-paper";
}

const PAYMENTS = ["Aug 31", "Aug 1", "Jul 2"];
const hi = "rounded-[3px] bg-honey px-[3px] text-espresso sm:px-1 sm:py-px";

function ProblemCard({ n, title, body, shortBody, children }: { n: string; title: string; body: string; shortBody: string; children: ReactNode }) {
  return (
    <article className="l-card flex flex-col overflow-hidden max-sm:rounded-xl">
      {children}
      <div className="flex flex-col gap-2 p-[18px] sm:gap-2.5 sm:p-7">
        <span className="mono text-[13px] text-bark max-sm:hidden">{n}</span>
        <h3 className="m-0 text-[21px] leading-[1.13] font-semi tracking-[-0.3px] sm:text-2xl sm:tracking-[-0.31px]">{title}</h3>
        <p className="m-0 text-[15px] leading-[1.3] text-bark sm:text-base sm:leading-[1.25]">
          <span className="sm:hidden">{shortBody}</span>
          <span className="max-sm:hidden">{body}</span>
        </p>
      </div>
    </article>
  );
}

const figure = "flex flex-col border-b border-linen bg-cream p-[18px] sm:p-7";

export function Problem() {
  return (
    <section className={`${containerClass} flex flex-col gap-5 py-14 sm:gap-12 sm:py-24`}>
      <SectionIntro
        eyebrow="The problem"
        title="Payroll shouldn't put your salary on public display."
        lead="Two things go wrong when crypto payroll copies the old model: pay arrives late, and everyone can read it."
        titleMax="max-w-[680px]"
      />

      <div className="grid grid-cols-[repeat(auto-fit,minmax(min(440px,100%),1fr))] gap-5">
        <ProblemCard
          n="01"
          title="Earned pay waits for payday"
          body="On a fixed schedule, money you've already earned sits out of reach until the calendar says so."
          shortBody="Money you've already earned sits out of reach until the calendar says so."
        >
          <div role="img" aria-label="A 30-day pay period: 18 days of work earned, nothing reachable until day 30" className={`${figure} gap-3 sm:gap-4`}>
            <div className="flex flex-wrap items-baseline justify-between gap-3 max-sm:hidden">
              <span className="eyebrow font-mid text-bark">One pay period</span>
              <span className="mono text-xs text-bark">1.5 ZEC / 30 days</span>
            </div>
            <div className="grid grid-cols-10 gap-1 sm:gap-[5px]">
              {DAYS.map((d) => (
                <span key={d} className={`aspect-square rounded-[3px] sm:rounded ${dayClass(d)}`}>
                  {d === 30 && "PAY"}
                </span>
              ))}
            </div>
            <div className="grid grid-cols-2 gap-2 sm:gap-2.5">
              <div className="rounded-lg border border-linen bg-paper p-2.5 sm:px-3.5 sm:py-3">
                <div className="text-[11px] text-bark sm:text-xs">Earned by day 18</div>
                <div className="mono mt-[3px] text-[17px] font-medium sm:mt-1 sm:text-xl">0.90 ZEC</div>
              </div>
              <div className="rounded-lg border border-espresso bg-paper p-2.5 sm:px-3.5 sm:py-3">
                <div className="text-[11px] font-mid sm:text-xs">You can reach</div>
                <div className="mono mt-[3px] text-[17px] font-medium sm:mt-1 sm:text-xl">0.00 ZEC</div>
              </div>
            </div>
          </div>
        </ProblemCard>

        <ProblemCard
          n="02"
          title="Public payments expose salaries"
          body="On a transparent chain, addresses, amounts and timing are open to anyone. Link one address to a person and their income is public."
          shortBody="Link one address to a person and their income is public."
        >
          <div
            role="img"
            aria-label="A public block explorer showing the same sender paying the same amount to the same address every 30 days"
            className={`${figure} gap-2.5 sm:gap-3.5`}
          >
            <div className="flex flex-wrap items-baseline justify-between gap-3 text-bark">
              <span className="eyebrow font-mid max-sm:text-[10.5px]">
                Public explorer<span className="max-sm:hidden"> · transparent address</span>
              </span>
              <span className="mono text-[11px] sm:text-xs">t1Ad…3mP</span>
            </div>
            <div className="overflow-hidden rounded-lg border border-linen bg-paper">
              <div className="eyebrow grid grid-cols-[72px_minmax(0,1fr)_auto] gap-3 border-b border-linen px-3.5 py-[9px] text-[11px] text-bark max-sm:hidden">
                <span>Date</span>
                <span>From</span>
                <span className="text-right">Amount</span>
              </div>
              {PAYMENTS.map((date) => (
                <div
                  key={date}
                  className="mono grid grid-cols-[56px_minmax(0,1fr)_auto] items-center gap-2 border-b border-mist px-3 py-[9px] text-xs last:border-b-0 sm:grid-cols-[72px_minmax(0,1fr)_auto] sm:gap-3 sm:px-3.5 sm:py-[11px] sm:text-[13px]"
                >
                  <span>{date}</span>
                  <span>
                    <mark className={hi}>t1Kx…9fQ</mark>
                  </span>
                  <span className="text-right">
                    <mark className={hi}>1.5000</mark>
                  </span>
                </div>
              ))}
            </div>
            <div className="flex items-start gap-2 text-[12.5px] sm:text-[13px]">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className="mt-px flex-none max-sm:hidden">
                <path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z" />
                <circle cx="12" cy="12" r="3" />
              </svg>
              <span>Same sender, same amount, every 30 days. Anyone can read this as a salary.</span>
            </div>
          </div>
        </ProblemCard>
      </div>

      <a
        href="#how"
        className="group flex items-center justify-between gap-3 rounded-lg border border-linen bg-paper p-4 text-[15px] leading-[1.3] font-mid text-espresso no-underline sm:flex-wrap sm:gap-4 sm:px-6 sm:py-5"
      >
        <span className="sm:text-lg sm:tracking-[-0.2px]">
          hushush fixes both: pay you can claim any second, sent<span className="max-sm:hidden"> as shielded transactions</span>
          <span className="sm:hidden"> shielded</span>.
        </span>
        <span className="inline-flex flex-none items-center gap-1.5 font-mid">
          <span className="underline group-hover:text-bark max-sm:hidden">See how it works</span>
          <ArrowRight size={18} />
        </span>
      </a>
    </section>
  );
}

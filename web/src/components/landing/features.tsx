import { CardStack, SwipeRail, type FeatureStep } from "./feature-steps";
import { Earned, PayoutStatus } from "./live";
import { ActiveTag, Dot, SectionIntro, Wordmark, containerClass } from "./ui";

// All names and amounts in these cards are demo data.

const cardTitle = "text-xl font-semi tracking-[-0.22px]";
const slideTitle = "text-[17px] font-semi";
const kv = "flex justify-between gap-2 border-b border-mist py-2.5 last:border-b-0";
const field = "flex h-11 items-center rounded-lg border border-linen px-3 text-sm";
const label = "text-[13px] text-bark";

/* ---------- workers ---------- */

const WORKER_STEPS: FeatureStep[] = [
  {
    title: "Watch it grow",
    body: "Your balance updates every second you're on the clock. Earned, reserved, claimed and available, always in plain view.",
    shortBody: "Your balance updates every second you're on the clock.",
    label: "Earnings growing every second",
  },
  {
    title: "Claim what you need",
    body: "Take any amount up to what's available. It's reserved for you straight away, so it can't be claimed twice.",
    shortBody: "Any amount up to what's available, reserved so it can't be claimed twice.",
    label: "Requesting a claim",
  },
  {
    title: "Paid only when it's confirmed",
    body: 'See the real network fee before you send. Your payout goes out shielded, and only says "paid" once the network confirms it.',
    shortBody: 'See the fee first. It only says "paid" once the network confirms it.',
    label: "Payout moving from pending to paid",
  },
];

function GrowthLine({ id, w, h, end, area = true }: { id: string; w: number; h: number; end: [number, number]; area?: boolean }) {
  const [x, y] = end;
  return (
    <svg viewBox={`0 0 ${w} ${h}`} aria-hidden="true" className="block h-auto w-full">
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#f9a600" stopOpacity=".3" />
          <stop offset="1" stopColor="#f9a600" stopOpacity="0" />
        </linearGradient>
      </defs>
      {area && <line x1="0" y1={h - 4} x2={w} y2={h - 4} stroke="#e3dfd5" />}
      {area && <path d={`M0 ${h - 4} L${x} ${y} L${x} ${h - 4} Z`} fill={`url(#${id})`} />}
      <path d={`M0 ${h - (area ? 4 : 2)} L${x} ${y}`} fill="none" stroke="#e89b01" strokeWidth={area ? 2.4 : 2.2} />
      <circle cx={x} cy={y} r={area ? 4.5 : 3.5} fill="#261b07" />
    </svg>
  );
}

const workerCards = [
  <>
    <div className="flex items-center justify-between">
      <Wordmark className="h-[18px]" alt="" />
      <ActiveTag />
    </div>
    <div className="flex flex-col gap-1.5">
      <span className="eyebrow text-bark">Earned so far</span>
      <span className="mono text-[34px] font-medium tracking-[-0.6px]">
        <Earned />
      </span>
      <span className="inline-flex items-center gap-1.5 text-sm text-bark">
        <Dot className="size-[7px] bg-amber" />
        Growing every second · 1.5 ZEC per 30 days
      </span>
    </div>
    <GrowthLine id="w-fill" w={360} h={90} end={[344, 12]} />
    <div className="grid grid-cols-3 gap-2 text-xs">
      <div className="rounded-lg border border-linen p-2.5">
        <div className="text-bark">Reserved</div>
        <div className="mono mt-[3px] text-[13px]">0.00</div>
      </div>
      <div className="rounded-lg border border-linen p-2.5">
        <div className="text-bark">Claimed</div>
        <div className="mono mt-[3px] text-[13px]">0.00</div>
      </div>
      <div className="rounded-lg border border-espresso bg-ivory p-2.5">
        <div className="font-mid">Available</div>
        <div className="mono mt-[3px] text-[13px]">all of it</div>
      </div>
    </div>
  </>,
  <>
    <div className="flex items-center justify-between">
      <span className={cardTitle}>Request a claim</span>
      <span className="mono text-xs text-bark">Kestrel Collective</span>
    </div>
    <div className="flex flex-col gap-2">
      <span className={label}>Amount (ZEC)</span>
      <div className="flex h-[60px] items-center rounded-lg border-[1.5px] border-espresso bg-paper pr-2 pl-3.5">
        <span className="mono flex-1 text-[28px] font-medium">1.25</span>
        <span className="rounded-md border border-stone px-3 py-1.5 text-[13px] font-mid">Max</span>
      </div>
      <span className="mono text-xs text-bark">Up to 1.24800000 available</span>
    </div>
    <span className="btn-amber flex h-12 items-center justify-center rounded-lg font-mid">Request claim</span>
    <div className="flex items-center gap-2.5 rounded-lg bg-cream px-3.5 py-3 text-[13px]">
      <span className="tag bg-honey">Pending · unpaid</span>
      <span className="text-bark">Reserved for you. Can&apos;t be claimed twice.</span>
    </div>
  </>,
  <>
    <div className="flex items-center justify-between">
      <span className={cardTitle}>Payout</span>
      <span className="mono text-xl font-medium">1.25 ZEC</span>
    </div>
    <div className="rounded-lg border border-linen px-3.5 py-0.5 text-[13.5px]">
      <div className={kv}>
        <span className="text-bark">Network fee (quoted)</span>
        <span className="mono">0.0001</span>
      </div>
      <div className={kv}>
        <span className="text-bark">To</span>
        <span className="mono">utest1q8…k9qz</span>
      </div>
      <div className={kv}>
        <span className="text-bark">Sent as</span>
        <span>Fully shielded</span>
      </div>
    </div>
    <div className="flex flex-col gap-2.5 rounded-lg bg-cream p-3.5">
      <PayoutStatus who="you" track="bg-linen" />
    </div>
  </>,
];

const workerSlides = [
  <>
    <span className="eyebrow text-[10.5px] text-bark">Earned so far</span>
    <span className="mono text-[27px] font-medium tracking-[-0.5px]">
      <Earned />
    </span>
    <span className="inline-flex items-center gap-1.5 text-[12.5px] text-bark">
      <Dot />
      Growing every second
    </span>
    <div className="mt-auto">
      <GrowthLine id="ws-fill" w={260} h={60} end={[250, 8]} area={false} />
    </div>
  </>,
  <>
    <span className={slideTitle}>Request a claim</span>
    <div className="flex h-[52px] items-center rounded-lg border-[1.5px] border-espresso pr-1.5 pl-3">
      <span className="mono flex-1 text-2xl">1.25</span>
      <span className="rounded-md border border-stone px-2.5 py-[5px] text-xs">Max</span>
    </div>
    <span className="btn-amber flex h-11 items-center justify-center rounded-lg text-sm font-mid">Request claim</span>
    <span className="tag self-start bg-honey">Pending · unpaid</span>
  </>,
  <>
    <div className="flex justify-between">
      <span className={slideTitle}>Payout</span>
      <span className="mono text-[17px]">1.25 ZEC</span>
    </div>
    <div className="text-[13px]">
      <div className={kv}>
        <span className="text-bark">Fee (quoted)</span>
        <span className="mono">0.0001</span>
      </div>
      <div className={kv}>
        <span className="text-bark">Sent as</span>
        <span>Fully shielded</span>
      </div>
    </div>
    <PayoutStatus who="you" noteClass="text-[12.5px]" />
  </>,
];

export function Workers() {
  return (
    <section id="earn" className={`${containerClass} flex scroll-mt-4 flex-col gap-5 pt-2 pb-14 sm:gap-14 sm:pt-6 sm:pb-24`}>
      {/* Old anchor, kept so existing #workers links still land here. */}
      <span id="workers" aria-hidden="true" className="block scroll-mt-4" />
      <SectionIntro
        eyebrow="Earn"
        title="Your pay, visible the moment you earn it."
        lead={
          <>
            <span className="sm:hidden">No waiting for payday. Swipe to see how it works.</span>
            <span className="max-sm:hidden">No waiting for payday. Watch your balance grow, take what you need, and always know where your money is.</span>
          </>
        }
      />
      <div className="max-sm:hidden">
        <CardStack steps={WORKER_STEPS} cards={workerCards} tone="light" />
      </div>
      <div className="sm:hidden">
        <SwipeRail steps={WORKER_STEPS} cards={workerSlides} tone="light" label="Worker features, swipe for more" />
      </div>
    </section>
  );
}

/* ---------- employers ---------- */

const EMPLOYER_STEPS: FeatureStep[] = [
  {
    title: "Start a stream in a minute",
    body: "Pick a worker, a fixed ZEC rate and when it starts. Pay builds up on its own from there.",
    shortBody: "Pick a worker, a fixed ZEC rate and a start.",
    label: "Creating a new payroll stream",
  },
  {
    title: "Pause, resume or end anytime",
    body: "Every change is logged with who made it. Ending a stream never takes back pay already earned.",
    shortBody: "Every change is logged. Earned pay is never taken back.",
    label: "Pausing a stream with its history",
  },
  {
    title: "Every claim, labelled honestly",
    body: "See what's pending, on the network or confirmed, next to your payout wallet's spendable balance.",
    shortBody: "Pending, on the network or confirmed, next to your wallet balance.",
    label: "Claims list with honest statuses",
  },
];

const CLAIMS = [
  { amount: "0.5", who: "Ava", status: "Sent · 6/10 confirmations", short: "Sent · 6/10", tone: "bg-wisteria" },
  { amount: "1.5", who: "Marco", status: "Checking payout · unpaid", tone: "bg-honey" },
  { amount: "0.25", who: "Priya", status: "Pending · unpaid", tone: "bg-honey", desktopOnly: true },
  { amount: "0.75", who: "Priya", status: "Paid · payout confirmed", tone: "bg-espresso text-cream" },
  { amount: "0.4", who: "Kyle", status: "Payout failed · not sent", tone: "bg-failed-bg text-failed-ink" },
];

const HISTORY = [
  { what: "Paused", when: "14 Oct" },
  { what: "Resumed", when: "2 Oct" },
  { what: "Stream created", when: "15 Sep" },
];

const resume = "inline-flex items-center rounded-lg border border-espresso px-3.5 font-mid";
const endStream = "inline-flex items-center rounded-lg border border-terracotta px-3.5 font-mid text-rust";

const employerCards = [
  <>
    <span className={cardTitle}>New stream</span>
    <div className="flex flex-col gap-1.5">
      <span className={label}>Worker</span>
      <div className={`${field} justify-between`}>
        <span>Ava Nolan (ava@example.com)</span>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
          <path d="m6 9 6 6 6-6" />
        </svg>
      </div>
    </div>
    <div className="grid grid-cols-2 gap-2.5">
      <div className="flex flex-col gap-1.5">
        <span className={label}>Rate (ZEC)</span>
        <div className="mono flex h-11 items-center rounded-lg border-[1.5px] border-espresso px-3 text-base">1.5</div>
      </div>
      <div className="flex flex-col gap-1.5">
        <span className={label}>Period</span>
        <div className={field}>per 30 days</div>
      </div>
    </div>
    <div className="grid grid-cols-2 gap-2.5">
      <div className="flex flex-col gap-1.5">
        <span className={label}>Start</span>
        <div className={field}>Now</div>
      </div>
      <div className="flex flex-col gap-1.5">
        <span className={label}>End (optional)</span>
        <div className="flex h-11 items-center rounded-lg border border-dashed border-stone px-3 text-sm text-bark">No end date</div>
      </div>
    </div>
    <span className="btn-amber flex h-[46px] items-center justify-center rounded-lg font-mid">Create stream</span>
  </>,
  <>
    <div className="flex flex-wrap items-center justify-between gap-2.5">
      <span className={cardTitle}>Marco Lindqvist</span>
      <span className="tag bg-linen text-bark">❚❚ Paused</span>
    </div>
    <span className="mono text-[13px] text-bark">9 ZEC per 30 days · earned 8.66550000</span>
    <div className="flex flex-wrap gap-2 text-sm">
      <span className={`${resume} h-[38px]`}>Resume</span>
      <span className={`${endStream} h-[38px]`}>End stream</span>
    </div>
    <div className="rounded-lg border border-honey bg-ivory px-3.5 py-3 text-[13px]">Accrual is paused. Wages earned so far are kept.</div>
    <ol className="m-0 flex list-none flex-col p-0 text-[13px]">
      {HISTORY.map((h) => (
        <li key={h.what} className="flex justify-between gap-2 border-b border-mist py-[9px] last:border-b-0">
          <span className="font-mid">{h.what}</span>
          <span className="text-bark">{h.when} · by Tom Hale</span>
        </li>
      ))}
    </ol>
  </>,
  <>
    <div className="flex items-center justify-between gap-2.5">
      <span className={cardTitle}>Claims</span>
      <span className="mono rounded bg-honey px-2 py-1 text-xs">Wallet · 42.5 TAZ</span>
    </div>
    <ul className="m-0 list-none p-0 text-[13.5px]">
      {CLAIMS.map((c) => (
        <li key={c.status} className="flex items-center justify-between gap-2 border-b border-mist py-[11px] last:border-b-0">
          <span>
            <span className="mono font-medium">{c.amount}</span> <span className="text-bark">· {c.who}</span>
          </span>
          <span className={`tag ${c.tone}`}>{c.status}</span>
        </li>
      ))}
    </ul>
  </>,
];

const employerSlides = [
  <>
    <span className={slideTitle}>New stream</span>
    <div className="flex h-[38px] items-center rounded-lg border border-linen px-2.5 text-[13px]">Ava Nolan</div>
    <div className="grid grid-cols-2 gap-2 text-[13px]">
      <div className="mono flex h-[38px] items-center rounded-lg border-[1.5px] border-espresso px-2.5">1.5</div>
      <div className="flex h-[38px] items-center rounded-lg border border-linen px-2.5">per 30 days</div>
    </div>
    <span className="btn-amber mt-auto flex h-[42px] items-center justify-center rounded-lg font-mid">Create stream</span>
  </>,
  <>
    <div className="flex items-center justify-between">
      <span className={slideTitle}>Marco Lindqvist</span>
      <span className="tag bg-linen text-bark">Paused</span>
    </div>
    <div className="flex gap-2 text-[13px]">
      <span className={`${resume} h-[34px] px-3`}>Resume</span>
      <span className={`${endStream} h-[34px] px-3`}>End stream</span>
    </div>
    <div className="rounded-lg border border-honey bg-ivory p-2.5 text-[13px]">Accrual is paused. Wages earned so far are kept.</div>
    <div className="flex justify-between gap-2 py-2 text-[13px]">
      <span className="font-mid">Paused</span>
      <span className="text-bark">14 Oct · by Tom</span>
    </div>
  </>,
  <>
    <div className="mb-0.5 flex items-center justify-between">
      <span className={slideTitle}>Claims</span>
      <span className="mono rounded bg-honey px-1.5 py-[3px] text-[11px]">42.5 TAZ</span>
    </div>
    <div className="-mt-1.5 text-[13px]">
      {CLAIMS.filter((c) => !c.desktopOnly).map((c) => (
        <div key={c.status} className="flex items-center justify-between gap-2 border-b border-mist py-[7px] last:border-b-0">
          <span className="mono">{c.amount}</span>
          <span className={`tag text-[11.5px] ${c.tone}`}>{c.short ?? c.status}</span>
        </div>
      ))}
    </div>
  </>,
];

export function Employers() {
  return (
    <section id="payroll" className="scroll-mt-4 bg-espresso text-cream">
      {/* Old anchor, kept so existing #employers links still land here. */}
      <span id="employers" aria-hidden="true" className="block scroll-mt-4" />
      <div className={`${containerClass} flex flex-col gap-5 py-14 sm:gap-14 sm:py-[104px]`}>
        <SectionIntro
          eyebrow="Payroll"
          eyebrowClass="bg-amber text-espresso"
          title="Run payroll like a ledger, not a guessing game."
          lead="Set it once, stay in control, and always know what's owed, reserved and paid."
          leadTone="text-sand"
        />
        <div className="max-sm:hidden">
          <CardStack steps={EMPLOYER_STEPS} cards={employerCards} tone="dark" reverse />
        </div>
        <div className="sm:hidden">
          <SwipeRail steps={EMPLOYER_STEPS} cards={employerSlides} tone="dark" label="Employer features, swipe for more" />
        </div>
      </div>
    </section>
  );
}

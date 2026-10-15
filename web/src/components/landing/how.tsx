import type { ReactNode } from "react";
import { Earned, PayoutStatus } from "./live";
import { ActiveTag, Dot, SectionIntro, Wordmark, containerClass } from "./ui";

const MOMENTS = [
  { when: "Aug 1 · 09:00 UTC", title: "A stream starts", body: "Her employer sets a fixed rate. Ava sees it the moment it begins.", aria: "Phone showing a new stream with nothing earned yet" },
  { when: "Aug 26 · 09:00 UTC", title: "Pay builds up", body: "Every active second adds to her balance, counted in whole zatoshi. Nothing is sent yet.", aria: "Phone showing earnings growing by the second" },
  {
    when: "Aug 26 · 09:02 UTC",
    title: "She claims, it lands",
    body: 'She reserves an amount, sees the real fee, and gets a shielded payout. It says "paid" only once it\'s confirmed.',
    aria: "Phone showing a claim moving from pending to paid",
  },
];

// Staggered upward, left to right.
const OFFSETS = ["mt-24", "mt-12", "mt-0"];

const tile = "rounded-lg border border-linen bg-paper p-2.5";
const tileLabel = "text-[11px] text-bark";
const kv = "flex justify-between border-b border-mist py-[9px] last:border-b-0";

function Phone({ aria, children }: { aria: string; children: ReactNode }) {
  return (
    <div role="img" aria-label={aria} className="h-[560px] w-[min(290px,100%)] self-center rounded-[44px] bg-espresso p-2.5 shadow-phone">
      <div className="flex h-full flex-col overflow-hidden rounded-[35px] bg-cream text-[13px]">
        <div className="flex items-center justify-between border-b border-linen bg-paper px-[18px] pt-7 pb-3.5">
          <Wordmark className="h-4" alt="" />
          <span className="flex size-[26px] items-center justify-center rounded-full bg-linen text-[10.5px] font-semi">AN</span>
        </div>
        <div className="flex flex-col gap-3.5 p-[18px]">{children}</div>
      </div>
    </div>
  );
}

const screens = [
  <>
    <div className="flex gap-2.5 rounded-lg border border-linen bg-paper p-3 shadow-warm">
      <Dot className="mt-1 size-2 bg-amber" />
      <span>
        <b className="font-semi">Kestrel Collective</b> started a payroll stream for you.
      </span>
    </div>
    <div className="flex flex-col gap-1">
      <span className="eyebrow text-[11px] text-bark">Rate</span>
      <span className="text-[26px] font-semi tracking-[-0.4px]">
        <span className="mono font-medium">1.5</span> ZEC
      </span>
      <span className="text-bark">per 30 days · starts now · no end date</span>
    </div>
    <div className="grid grid-cols-2 gap-2">
      <div className={tile}>
        <div className={tileLabel}>Earned</div>
        <div className="mono mt-[3px] text-[13px]">0.00000000</div>
      </div>
      <div className={tile}>
        <div className={tileLabel}>Available</div>
        <div className="mono mt-[3px] text-[13px]">0.00000000</div>
      </div>
    </div>
    <span className="self-start">
      <ActiveTag />
    </span>
  </>,
  <>
    <div className="flex flex-col gap-1">
      <span className="eyebrow text-[11px] text-bark">Earned so far</span>
      <span className="mono text-2xl font-medium tracking-[-0.4px]">
        <Earned />
      </span>
      <span className="inline-flex items-center gap-1.5 text-bark">
        <Dot />
        Growing every second
      </span>
    </div>
    <svg viewBox="0 0 250 90" aria-hidden="true" className="block h-auto w-full">
      <defs>
        <linearGradient id="how-fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#f9a600" stopOpacity=".3" />
          <stop offset="1" stopColor="#f9a600" stopOpacity="0" />
        </linearGradient>
      </defs>
      <line x1="0" y1="86" x2="250" y2="86" stroke="#e3dfd5" />
      <path d="M0 86 L238 14 L238 86 Z" fill="url(#how-fill)" />
      <path d="M0 86 L238 14" fill="none" stroke="#e89b01" strokeWidth="2.4" />
      <circle cx="238" cy="14" r="4" fill="#261b07" />
    </svg>
    <div className="grid grid-cols-2 gap-2">
      <div className={tile}>
        <div className={tileLabel}>Reserved</div>
        <div className="mono mt-[3px] text-[13px]">0.00000000</div>
      </div>
      <div className={tile}>
        <div className={tileLabel}>Claimed</div>
        <div className="mono mt-[3px] text-[13px]">0.00000000</div>
      </div>
    </div>
    <div className="flex items-center justify-between gap-2 rounded-lg border border-espresso bg-ivory p-3">
      <span className="font-mid">Available</span>
      <span className="mono text-[13px]">
        <Earned />
      </span>
    </div>
  </>,
  <>
    <div className="flex flex-col gap-1">
      <span className="eyebrow text-[11px] text-bark">Your claim</span>
      <span className="mono text-[26px] font-medium tracking-[-0.4px]">1.25 ZEC</span>
    </div>
    <div className="rounded-lg border border-linen bg-paper px-3 py-0.5">
      <div className={kv}>
        <span className="text-bark">Network fee</span>
        <span className="mono">0.0001</span>
      </div>
      <div className={kv}>
        <span className="text-bark">To</span>
        <span className="mono">utest1q8…k9qz</span>
      </div>
      <div className={kv}>
        <span className="text-bark">Sent as</span>
        <span>Shielded</span>
      </div>
    </div>
    <div className="flex flex-col gap-2.5 rounded-lg border border-linen bg-paper p-3.5">
      <PayoutStatus noteClass="" />
    </div>
  </>,
];

/** Compact cards for the phone timeline. */
const timelineCards = [
  <>
    <div className="flex gap-2">
      <Dot className="mt-1 size-[7px] bg-amber" />
      <span>
        <b className="font-semi">Kestrel Collective</b> started a payroll stream for you.
      </span>
    </div>
    <div className="flex items-baseline justify-between">
      <span className="text-xl font-semi">
        <span className="mono font-medium">1.5</span> ZEC
      </span>
      <span className="text-bark">per 30 days</span>
    </div>
  </>,
  <>
    <span className="text-[11px] text-bark">Earned so far · growing every second</span>
    <span className="mono text-2xl font-medium">
      <Earned />
    </span>
  </>,
  <>
    <div className="flex justify-between">
      <span>Claim</span>
      <span className="mono font-medium">1.25 ZEC</span>
    </div>
    <PayoutStatus note={false} />
  </>,
];

export function HowItWorks() {
  return (
    <section id="how" className={`${containerClass} flex flex-col gap-5 py-14 sm:gap-14 sm:pt-24`}>
      <SectionIntro
        eyebrow="How it works"
        title="One stream, three moments."
        lead={
          <>
            Follow Ava&apos;s pay from the day her stream starts to the moment it lands.
            <span className="max-sm:hidden"> Money only moves when she claims it.</span>
          </>
        }
        leadMax="sm:max-w-[420px]"
      />

      <ol className="m-0 grid list-none grid-cols-3 items-start gap-x-8 gap-y-10 p-0 max-lg:hidden">
        {MOMENTS.map((m, i) => (
          <li key={m.title} className={`flex flex-col gap-5 ${OFFSETS[i]}`}>
            <div className="flex items-center gap-2.5">
              <Dot className={i === 2 ? "size-2.5 bg-amber shadow-[0_0_0_3px_var(--color-honey)]" : "size-2.5 bg-espresso"} />
              <span className="mono text-[13px] text-bark">{m.when}</span>
              <span className="h-px flex-1 bg-linen" />
            </div>
            <Phone aria={m.aria}>{screens[i]}</Phone>
            <div className="flex flex-col gap-2">
              <h3 className="m-0 text-2xl leading-[1.13] font-semi tracking-[-0.31px]">
                <span className="mono mr-2 text-sm font-normal text-bark">{String(i + 1).padStart(2, "0")}</span>
                {m.title}
              </h3>
              <p className="m-0 text-bark">{m.body}</p>
            </div>
          </li>
        ))}
      </ol>

      <ol className="mt-2 ml-1.5 flex max-w-xl list-none flex-col gap-7 border-l-2 border-linen py-0 pr-0 pl-[22px] lg:hidden">
        {MOMENTS.map((m, i) => (
          <li key={m.title} className="relative flex flex-col gap-2.5">
            <span
              aria-hidden="true"
              className={`absolute rounded-full border-[3px] ${i === 2 ? "top-px -left-[31px] size-4 border-honey bg-amber" : "top-0.5 -left-[30px] size-3.5 border-cream bg-espresso"}`}
            />
            <span className="mono text-xs text-bark">{m.when}</span>
            <h3 className="m-0 text-[21px] font-semi tracking-[-0.3px]">{m.title}</h3>
            <p className="m-0 text-[15px] leading-[1.3] text-bark max-sm:hidden">{m.body}</p>
            <div role="img" aria-label={m.aria.replace("Phone showing", "Preview of")} className="l-card flex flex-col gap-2.5 rounded-xl p-3.5 text-[13px]">
              {timelineCards[i]}
            </div>
          </li>
        ))}
      </ol>
    </section>
  );
}

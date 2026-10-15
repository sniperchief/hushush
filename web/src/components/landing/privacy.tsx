import type { ReactNode } from "react";
import { Mark, SectionIntro, containerClass } from "./ui";

// Shielding hides amount, sender and recipient. Timing and fee stay visible:
// keep this copy honest and never call payouts "anonymous" or "untraceable".

const row = "flex min-h-[50px] items-center justify-between gap-3 border-b text-[15px] last:border-b-0";

function Row({ label, dark, children }: { label: string; dark?: boolean; children: ReactNode }) {
  return (
    <div className={`${row} ${dark ? "border-cream/14" : "border-mist"}`}>
      <span className={`text-[13px] ${dark ? "text-oat" : "text-bark"}`}>{label}</span>
      {children}
    </div>
  );
}

function Hidden() {
  return (
    <span className="inline-flex items-center gap-2 text-[13px] font-mid text-cream">
      <span className="inline-block h-3 w-[72px] rounded-[3px] bg-cocoa" />
      Hidden
    </span>
  );
}

function Visible({ children }: { children: ReactNode }) {
  return (
    <span className="flex items-center gap-1.5 sm:gap-2">
      <span className="mono">{children}</span>
      <span className="rounded bg-amber px-1.5 py-0.5 text-[11px] font-mid text-espresso">Visible</span>
    </span>
  );
}

const receipt = "flex w-full max-w-[500px] min-w-0 flex-col gap-1 rounded-xl p-[18px] max-lg:mx-auto sm:gap-1.5 sm:p-7 lg:flex-[1_1_380px]";

export function Privacy() {
  return (
    <section id="privacy" className={`${containerClass} flex flex-col gap-[18px] pt-2 pb-14 sm:gap-12 sm:pt-6 sm:pb-[104px]`}>
      <SectionIntro
        eyebrow="Privacy"
        eyebrowClass="bg-wisteria"
        title="Same payout. Only your team sees the details."
        lead="Here is one real payout, twice: what hushush keeps for your team, and what a stranger looking at the blockchain can see."
        titleMax="max-w-[660px]"
        leadMax="sm:max-w-[420px]"
      />

      <div className="flex flex-col items-stretch justify-center gap-[18px] sm:gap-5 lg:flex-row">
        <article className={`l-card ${receipt}`}>
          <div className="mb-1.5 flex items-center justify-between gap-3">
            <h3 className="m-0 text-lg font-semi sm:text-xl sm:tracking-[-0.22px]">Your hushush record</h3>
            <Mark className="h-[18px] sm:h-[22px]" />
          </div>
          <span className="mb-1.5 text-[13px] text-bark sm:mb-2.5 sm:text-sm">Seen by: you and the admins of your organization</span>
          <Row label="Who got paid">
            <span className="font-semi">Ava Nolan</span>
          </Row>
          <Row label="Who paid">
            <span className="font-semi">Kestrel Collective</span>
          </Row>
          <Row label="Amount">
            <span className="mono font-medium sm:text-lg">1.25 ZEC</span>
          </Row>
          <Row label="What it was for">
            <span>Design retainer</span>
          </Row>
          <Row label="When">
            <span className="mono">26 Aug, 09:02 UTC</span>
          </Row>
          <Row label="Network fee">
            <span className="mono">0.0001 ZEC</span>
          </Row>
        </article>

        <div aria-hidden="true" className="flex flex-none items-center justify-center gap-2.5 py-0 text-[12.5px] font-mid lg:flex-col lg:gap-2 lg:py-2 lg:text-xs">
          <span className="flex size-10 items-center justify-center rounded-full bg-amber shadow-pill lg:size-[52px]">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="#261b07" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="size-[18px] rotate-90 lg:size-[22px] lg:rotate-0">
              <path d="M5 12h14M13 6l6 6-6 6" />
            </svg>
          </span>
          <span className="text-center leading-[1.2]">
            Sent as a<br className="max-lg:hidden" /> shielded payment
          </span>
        </div>

        <article className={`${receipt} bg-espresso text-cream shadow-warm`}>
          <div className="mb-1.5 flex items-center justify-between gap-3">
            <h3 className="m-0 text-lg font-semi sm:text-xl sm:tracking-[-0.22px]">What the blockchain shows</h3>
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="#f9a600" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className="max-sm:hidden">
              <circle cx="12" cy="12" r="9" />
              <path d="M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z" />
            </svg>
          </div>
          <span className="mb-1.5 text-[13px] text-oat sm:mb-2.5 sm:text-sm">Seen by: anyone on the internet</span>
          <Row label="Who got paid" dark>
            <Hidden />
          </Row>
          <Row label="Who paid" dark>
            <Hidden />
          </Row>
          <Row label="Amount" dark>
            <Hidden />
          </Row>
          <Row label="What it was for" dark>
            <span className="text-oat">Never put on chain</span>
          </Row>
          <Row label="When" dark>
            <Visible>26 Aug, 09:02 UTC</Visible>
          </Row>
          <Row label="Network fee" dark>
            <Visible>0.0001 ZEC</Visible>
          </Row>
        </article>
      </div>

      <p className="m-0 max-w-[720px] text-[14.5px] leading-[1.3] text-bark sm:mx-auto sm:text-center sm:text-base sm:leading-[1.25]">
        Shielding hides who and how much. It doesn&apos;t hide that a payment happened, when, or its small network fee
        <span className="max-sm:hidden">, and we won&apos;t pretend it does</span>.
      </p>
    </section>
  );
}

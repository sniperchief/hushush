"use client";

import { useRouter } from "next/navigation";
import { useState, type ReactNode } from "react";
import type { Intent } from "@/lib/auth";
import { api } from "@/lib/client-api";
import { AuthAlert } from "./auth-ui";
import { GetPaidArt, PayTeamArt } from "./onboarding-art";

function useSetIntent() {
  const router = useRouter();
  const [pending, setPending] = useState<Intent>();
  const [error, setError] = useState<string>();
  const choose = async (intent: Intent) => {
    setPending(intent);
    setError(undefined);
    const r = await api("/api/account/intent", "POST", { intent });
    if (!r.ok) {
      setPending(undefined);
      return setError(r.error);
    }
    router.refresh();
  };
  return { pending, error, choose };
}

const OPTIONS: { intent: Intent; title: string; body: string; cta: string; Art: () => ReactNode }[] = [
  { intent: "pay", title: "I pay a team", body: "Stream pay to the people who work with you, every second, privately.", cta: "Start paying", Art: PayTeamArt },
  {
    intent: "get_paid",
    title: "I get paid through hushush",
    body: "See your pay grow as you earn it and withdraw to your own wallet.",
    cta: "Start getting paid",
    Art: GetPaidArt,
  },
];

/** First screen after sign-up: picks where the account starts. Both stay possible. */
export function IntentQuestion({ name }: { name: string }) {
  const { pending, error, choose } = useSetIntent();
  return (
    <div className="landing force-light font-landing text-espresso">
      <div className="mx-auto flex w-full max-w-[760px] flex-col gap-6 sm:gap-9 sm:pt-[clamp(8px,4vw,56px)] sm:pb-10">
        <div className="flex flex-col gap-2 sm:items-center sm:gap-2.5 sm:text-center">
          <span className="text-sm text-bark sm:text-[15px]">Welcome, {name}</span>
          <h1 className="m-0 text-[32px] leading-[1.08] font-semi tracking-[-0.8px] sm:text-[clamp(34px,4.4vw,48px)] sm:tracking-[-0.025em]">
            How will you use hushush?
          </h1>
          <p className="m-0 text-base leading-[1.4] text-bark sm:text-[17px]">Pick where to start. You can do both later.</p>
        </div>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-[repeat(auto-fit,minmax(min(300px,100%),1fr))] sm:gap-4">
          {OPTIONS.map(({ intent, title, body, cta, Art }) => (
            <button
              key={intent}
              type="button"
              disabled={pending !== undefined}
              onClick={() => choose(intent)}
              className="group relative flex w-full cursor-pointer flex-col gap-3 overflow-hidden rounded-xl border border-linen bg-paper p-[18px] text-left text-espresso shadow-warm transition-[border-color,transform,box-shadow] duration-200 active:scale-[.99] active:border-espresso disabled:cursor-wait sm:gap-3.5 sm:p-7 sm:hover:-translate-y-0.5 sm:hover:border-espresso sm:hover:shadow-lift motion-reduce:transition-none motion-reduce:hover:translate-y-0"
            >
              <span className="-mx-[18px] -mt-[18px] mb-1 block h-[118px] rounded-t-[11px] border-b border-linen bg-honey-band px-5 py-4 sm:-mx-7 sm:-mt-7 sm:mb-1.5 sm:h-[170px]">
                <Art />
              </span>
              <span className="text-lg font-semi sm:text-[22px] sm:tracking-[-0.3px]">{title}</span>
              <span className="text-sm leading-[1.4] text-bark sm:text-[15px] sm:leading-[1.45]">{body}</span>
              <span className="mt-0.5 flex items-center gap-2 text-[13px] font-semi sm:mt-1.5 sm:text-sm">
                {pending === intent ? "One moment…" : cta}
                <svg
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                  className="transition-transform duration-200 group-hover:translate-x-1 motion-reduce:transition-none max-sm:hidden"
                >
                  <path d="M5 12h14M13 6l6 6-6 6" />
                </svg>
              </span>
            </button>
          ))}
        </div>
        {error && <AuthAlert tone="bad">{error}</AuthAlert>}
      </div>
    </div>
  );
}

/** Link to the other side of hushush, e.g. a worker who starts paying people. `className` restyles the button. */
export function SwitchIntent({ to, children, className }: { to: Intent; children: ReactNode; className?: string }) {
  const { pending, error, choose } = useSetIntent();
  return (
    <span className="inline-flex flex-col items-start">
      <button
        type="button"
        disabled={pending !== undefined}
        onClick={() => choose(to)}
        className={className ?? "cursor-pointer text-sm text-ink-soft underline underline-offset-2 hover:text-ink disabled:cursor-wait disabled:opacity-60"}
      >
        {pending ? "One moment…" : children}
      </button>
      {error && <span className="mt-1 text-xs text-bad">{error}</span>}
    </span>
  );
}

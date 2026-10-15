"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";

export type FeatureStep = {
  title: string;
  body: string;
  /** Shorter body for the phone carousel. */
  shortBody?: string;
  /** Short description of the card for screen readers. */
  label: string;
};

type Tone = "light" | "dark";

/** Position of a card `d` steps away from the active one. */
function deckStyle(d: number) {
  if (d === 0) return { transform: "translate(0,0) rotate(0deg) scale(1)", opacity: 1, zIndex: 30 };
  if (d > 0) {
    return {
      transform: `translate(${d * 26}px,${-d * 22}px) rotate(${d * 2.5}deg) scale(${1 - d * 0.06})`,
      opacity: d === 1 ? 0.75 : 0.45,
      zIndex: 30 - d,
    };
  }
  return { transform: "translate(-60px,80px) rotate(-6deg) scale(.9)", opacity: 0, zIndex: 5 };
}

/**
 * Sticky stack of app cards beside numbered steps (tablet and desktop).
 * A step becomes active when its top passes 55% of the viewport, or on click.
 */
export function CardStack({
  steps,
  cards,
  tone,
  reverse = false,
}: {
  steps: FeatureStep[];
  cards: ReactNode[];
  tone: Tone;
  reverse?: boolean;
}) {
  const [active, setActive] = useState(0);
  const stepRefs = useRef<(HTMLLIElement | null)[]>([]);
  const dark = tone === "dark";

  useEffect(() => {
    const els = stepRefs.current;
    // The observer only signals that a step crossed the 55% line; the last
    // step above the line wins, as with a scroll listener.
    const pick = () => {
      const line = window.innerHeight * 0.55;
      let idx = -1;
      els.forEach((el, i) => {
        if (el && el.getBoundingClientRect().top < line) idx = i;
      });
      if (idx >= 0) setActive(idx);
    };
    const io = new IntersectionObserver(pick, { rootMargin: "0px 0px -45% 0px" });
    els.forEach((el) => el && io.observe(el));
    return () => io.disconnect();
  }, []);

  return (
    <div className={`flex flex-wrap items-start gap-x-16 gap-y-12 ${reverse ? "flex-row-reverse" : ""}`}>
      <div className="min-w-0 flex-[1_1_440px] self-start min-[900px]:sticky min-[900px]:top-24">
        <div role="img" aria-label={`App preview: ${steps[active]?.label ?? ""}`} className="relative mx-auto h-[500px] max-w-[520px]">
          <div
            className={`absolute inset-[40px_30px_30px_10px] rounded-2xl ${dark ? "rotate-3 bg-amber opacity-[.22]" : "-rotate-3 bg-honey opacity-55"}`}
          />
          {cards.map((card, i) => (
            <div
              key={i}
              style={deckStyle(i - active)}
              className="absolute inset-x-0 top-6 mx-auto flex w-[min(440px,100%)] origin-bottom flex-col gap-[18px] rounded-xl border border-linen bg-paper p-6 text-espresso shadow-deck will-change-transform [transition:transform_.7s_var(--ease-deck),opacity_.5s_ease] motion-reduce:transition-none"
            >
              {card}
            </div>
          ))}
        </div>
        <div aria-hidden="true" className="mt-2 flex justify-center gap-1.5">
          {steps.map((_, i) => (
            <span
              key={i}
              className={`h-1.5 rounded-[3px] transition-[width] duration-300 motion-reduce:transition-none ${dark ? "bg-amber" : "bg-espresso"} ${i === active ? "w-7" : "w-1.5"}`}
            />
          ))}
        </div>
      </div>

      <ol className="m-0 flex min-w-0 flex-[1_1_360px] list-none flex-col p-0">
        {steps.map((s, i) => {
          const on = i === active;
          return (
            <li
              key={s.title}
              ref={(el) => {
                stepRefs.current[i] = el;
              }}
              className={`flex items-center border-t min-[900px]:min-h-[min(56vh,440px)] ${dark ? "border-umber-line" : "border-linen"}`}
            >
              <button
                type="button"
                onClick={() => setActive(i)}
                aria-pressed={on}
                className={`flex w-full cursor-pointer items-start gap-5 rounded-lg py-8 text-left transition-opacity duration-[400ms] focus-visible:outline-offset-4 motion-reduce:transition-none ${on ? "opacity-100" : "opacity-[.42]"}`}
              >
                <span
                  className={`mono flex size-10 flex-none items-center justify-center rounded-lg text-sm font-medium transition-colors duration-300 motion-reduce:transition-none ${
                    on ? "bg-amber text-espresso" : dark ? "bg-umber text-cream" : "bg-linen text-bark"
                  }`}
                >
                  {String(i + 1).padStart(2, "0")}
                </span>
                <span className="flex flex-col gap-2.5">
                  <span className="text-[clamp(24px,2.4vw,30px)] leading-[1.13] font-semi tracking-[-0.4px]">{s.title}</span>
                  <span className={`text-lg ${dark ? "text-sand" : "text-bark"}`}>{s.body}</span>
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    </div>
  );
}

/** Phone layout: horizontal scroll-snap carousel whose dots follow the visible slide. */
export function SwipeRail({
  steps,
  cards,
  tone,
  label,
}: {
  steps: FeatureStep[];
  cards: ReactNode[];
  tone: Tone;
  label: string;
}) {
  const [active, setActive] = useState(0);
  const railRef = useRef<HTMLDivElement>(null);
  const slideRefs = useRef<(HTMLDivElement | null)[]>([]);
  const dark = tone === "dark";

  useEffect(() => {
    const rail = railRef.current;
    if (!rail) return;
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) setActive(slideRefs.current.indexOf(e.target as HTMLDivElement));
        }
      },
      { root: rail, threshold: 0.6 },
    );
    slideRefs.current.forEach((el) => el && io.observe(el));
    return () => io.disconnect();
  }, []);

  return (
    <>
      <div
        ref={railRef}
        role="region"
        aria-label={label}
        tabIndex={0}
        className="rail -mx-4 flex snap-x snap-mandatory gap-3 overflow-x-auto px-4 pt-1 pb-5"
      >
        {steps.map((s, i) => (
          <div
            key={s.title}
            ref={(el) => {
              slideRefs.current[i] = el;
            }}
            className="flex flex-[0_0_300px] snap-center flex-col gap-3.5"
          >
            <div role="img" aria-label={`App preview: ${s.label}`} className="l-card flex h-[250px] flex-col gap-3 rounded-xl p-[18px] text-espresso">
              {cards[i]}
            </div>
            <div className="flex gap-3">
              <span
                className={`mono flex size-[30px] flex-none items-center justify-center rounded-md text-[12.5px] ${
                  i === active ? "bg-amber text-espresso" : dark ? "bg-umber text-cream" : "bg-linen text-espresso"
                }`}
              >
                {String(i + 1).padStart(2, "0")}
              </span>
              <div>
                <div className="text-[19px] font-semi tracking-[-0.3px]">{s.title}</div>
                <div className={`mt-1 text-[14.5px] leading-[1.3] ${dark ? "text-sand" : "text-bark"}`}>{s.shortBody ?? s.body}</div>
              </div>
            </div>
          </div>
        ))}
      </div>
      <div aria-hidden="true" className="-mt-1.5 flex justify-center gap-1.5">
        {steps.map((_, i) => (
          <span
            key={i}
            className={`h-1.5 rounded-[3px] transition-[width] duration-300 motion-reduce:transition-none ${
              i === active ? `w-6 ${dark ? "bg-amber" : "bg-espresso"}` : `w-1.5 ${dark ? "bg-umber" : "bg-stone"}`
            }`}
          />
        ))}
      </div>
    </>
  );
}

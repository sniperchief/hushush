import type { ComponentProps, ReactNode } from "react";

// Building blocks for the sign-up, sign-in, email-link and onboarding screens
// (warm landing palette, light only).

/** Amber, 54px, full width: the only filled button. */
export const primaryButton =
  "btn-amber flex h-[54px] sm:h-11 w-full cursor-pointer items-center justify-center gap-2.5 rounded-lg text-base font-semi no-underline transition-[transform,background-color] duration-150 active:scale-[.98] disabled:cursor-not-allowed disabled:opacity-75 disabled:active:scale-100 motion-reduce:transition-none";

/** Transparent with a 1px espresso border. */
export const secondaryButton =
  "flex h-[54px] sm:h-11 w-full cursor-pointer items-center justify-center gap-2.5 rounded-lg border border-espresso bg-transparent text-base font-semi text-espresso transition-[transform,background-color] duration-150 hover:bg-paper active:scale-[.98] disabled:cursor-not-allowed disabled:opacity-75 disabled:active:scale-100 motion-reduce:transition-none";

/** Underlined text button ("Use a different email", "I'm here to get paid instead"). */
export const textButton =
  "min-h-11 cursor-pointer border-0 bg-transparent p-0 text-sm text-bark underline underline-offset-[3px] hover:text-espresso sm:min-h-8";

/** Link with an amber-deep underline that grows on hover. */
export const growLink =
  "font-semi text-espresso no-underline bg-[linear-gradient(var(--color-amber-deep),var(--color-amber-deep))] bg-no-repeat bg-[position:0_100%] bg-[length:0_2px] transition-[background-size] duration-[250ms] hover:bg-[length:100%_2px] max-sm:bg-[length:100%_2px] motion-reduce:transition-none";

export const hintClass = "m-0 text-[13px] leading-[1.4] text-bark";

/**
 * Label above a borderless input with a 2px linen underline; an amber-deep
 * line grows across it on focus.
 */
export function UnderlineField({ id, label, ...input }: { id: string; label: string } & ComponentProps<"input">) {
  return (
    <div className="group relative flex flex-col gap-2 sm:gap-1.5">
      <label htmlFor={id} className="text-[11.5px] font-semi tracking-[.1em] text-bark uppercase sm:text-xs">
        {label}
      </label>
      <input
        id={id}
        {...input}
        className="w-full rounded-none border-0 border-b-2 border-linen bg-transparent px-0 pt-1.5 pb-3 text-[17px] text-espresso sm:pt-1 sm:pb-2 sm:text-base outline-none placeholder:text-placeholder"
      />
      <span
        aria-hidden="true"
        className="absolute bottom-0 left-0 h-0.5 w-0 bg-amber-deep transition-[width] duration-300 ease-out group-focus-within:w-full motion-reduce:transition-none"
      />
    </div>
  );
}

export function OrDivider() {
  return (
    <div
      role="separator"
      className="flex items-center gap-3 text-xs font-semi tracking-[.1em] text-bark uppercase before:h-px before:flex-1 before:bg-linen after:h-px after:flex-1 after:bg-linen sm:gap-3.5 sm:text-[13px]"
    >
      or
    </div>
  );
}

const alertTones = {
  bad: "border-failed-line bg-failed-bg text-failed-ink",
  ok: "border-honey bg-honey-soft text-espresso",
  info: "border-linen bg-cream text-bark",
};

export function AuthAlert({ tone, children }: { tone: keyof typeof alertTones; children: ReactNode }) {
  return (
    <div role={tone === "bad" ? "alert" : "status"} className={`rounded-lg border px-3.5 py-3 text-sm leading-[1.45] ${alertTones[tone]}`}>
      {children}
    </div>
  );
}

/** Fingerprint icon; a ring pulses around it while a passkey prompt is open. */
export function Fingerprint({ waiting }: { waiting: boolean }) {
  return (
    <span className="relative inline-flex">
      <span
        aria-hidden="true"
        className={`absolute -inset-1.5 rounded-full border-2 border-espresso opacity-0 ${waiting ? "animate-ring motion-reduce:animate-none" : ""}`}
      />
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
        <path d="M12 11c0 3.5-1 6.5-3 9" />
        <path d="M8.5 6.6A6 6 0 0 1 18 11c0 1.6-.1 3.1-.4 4.5" />
        <path d="M6 9.2A6 6 0 0 0 6 11c0 2-.4 3.8-1.2 5.4" />
        <path d="M15 11a3 3 0 0 0-6 0c0 2.7-.6 5.2-1.8 7.4" />
        <path d="M15 14.5c-.3 2.1-1 4-2 5.6" />
      </svg>
    </span>
  );
}

/** Desktop: white card. Phones: no card, the form sits on the page. */
export const authCard =
  "flex flex-col gap-6 sm:gap-3.5 sm:rounded-xl sm:border sm:border-linen sm:bg-paper sm:p-5 sm:shadow-card";

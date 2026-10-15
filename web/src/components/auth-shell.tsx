import Link from "next/link";
import type { ReactNode } from "react";
import { Wordmark } from "./landing/ui";

/** The landing page's dark testnet notice. */
export function TestnetStrip() {
  return (
    <div className="bg-espresso px-4 py-[7px] text-center text-xs text-paper sm:py-1.5 sm:text-[13px]">
      <span className="sm:hidden">Testnet preview · test coins only</span>
      <span className="max-sm:hidden">Testnet preview: payouts use test coins, not real ZEC.</span>
    </div>
  );
}

/** Wordmark header. `aside` shows on the right (phones only unless `asideAlways`). */
export function AuthHeader({ aside, asideAlways = false }: { aside?: ReactNode; asideAlways?: boolean }) {
  return (
    <header className="flex items-center justify-between gap-4 border-b border-linen py-2 pr-2 pl-5 sm:py-2.5 sm:pr-[clamp(20px,4vw,40px)] sm:pl-[clamp(20px,4vw,40px)]">
      <Link href="/" aria-label="hushush home" className="flex items-center">
        <Wordmark className="h-[22px] sm:h-[26px]" alt="" />
      </Link>
      {aside && <div className={asideAlways ? "" : "sm:hidden"}>{aside}</div>}
    </header>
  );
}

/** Phone header link, e.g. "Sign in" on the sign-up page. */
export function HeaderLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <Link href={href} className="flex min-h-11 items-center px-3 text-sm font-semi text-espresso no-underline">
      {children}
    </Link>
  );
}

/** Centered column for the sign-up and sign-in pages. */
export function AuthMain({ children }: { children: ReactNode }) {
  return (
    <main className="flex flex-1 justify-center px-5 pt-8 pb-6 sm:items-start sm:pt-3 sm:pb-4">
      <div className="flex w-full max-w-[420px] flex-col gap-[26px] sm:gap-3">{children}</div>
    </main>
  );
}

export function AuthTitle({ title, lead }: { title: string; lead: string }) {
  return (
    <div className="flex flex-col gap-2 sm:gap-1 sm:text-center">
      <h1 className="m-0 text-[32px] leading-[1.08] font-semi tracking-[-0.8px] sm:text-[28px] sm:leading-[1.1] sm:tracking-[-0.025em]">{title}</h1>
      <p className="m-0 text-base leading-[1.4] text-bark sm:text-[15px]">{lead}</p>
    </div>
  );
}

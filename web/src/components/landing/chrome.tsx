import Link from "next/link";
import { MobileMenu } from "./mobile-menu";
import { Wordmark, containerClass, leadClass } from "./ui";

const NAV = [
  { href: "#how", label: "How it works" },
  { href: "#employers", label: "For employers" },
  { href: "#workers", label: "For workers" },
  { href: "#privacy", label: "Privacy" },
];

const navLink = "text-sm text-espresso no-underline hover:text-bark";
const bigButton = "inline-flex min-h-[46px] items-center justify-center rounded-lg px-[18px] py-3 font-mid no-underline max-sm:h-[52px] max-sm:w-full";
// Side by side on phones, so slightly tighter than the stacked buttons.
const heroButton = "whitespace-nowrap max-sm:px-3 max-sm:text-[15px]";

export function Banner() {
  return (
    <div className="bg-espresso px-4 py-2 text-center text-[12.5px] text-paper sm:py-[9px] sm:text-sm">
      <span className="sm:hidden">Testnet preview · test coins, not real ZEC</span>
      <span className="max-sm:hidden">
        Testnet preview: payouts use test coins, not real ZEC.{" "}
        <a href="#privacy" className="text-honey hover:text-honey">
          How shielding works
        </a>
      </span>
    </div>
  );
}

export function Header() {
  return (
    <header className="relative border-b border-linen">
      <div className="mx-auto flex max-w-[1200px] flex-wrap items-center justify-between gap-x-6 gap-y-3 py-2.5 pr-2 pl-4 md:px-[clamp(16px,4vw,32px)] md:py-4">
        <Link href="/" aria-label="hushush home" className="flex items-center">
          <Wordmark className="h-[22px] md:h-[26px]" alt="" preload />
        </Link>
        <div className="md:hidden">
          <MobileMenu />
        </div>
        <div className="flex flex-wrap items-center gap-x-6 gap-y-2 max-md:hidden">
          <nav aria-label="Primary" className="flex flex-wrap gap-x-5 gap-y-2">
            {NAV.map((l) => (
              <a key={l.href} href={l.href} className={navLink}>
                {l.label}
              </a>
            ))}
          </nav>
          <div className="flex items-center gap-3">
            <Link href="/login" className={`${navLink} inline-flex min-h-10 items-center font-mid`}>
              Sign in
            </Link>
            <Link href="/signup" className="btn-amber inline-flex min-h-10 items-center rounded-lg px-3 py-2 text-sm font-mid no-underline">
              Get started
            </Link>
          </div>
        </div>
      </div>
    </header>
  );
}

export function Hero() {
  return (
    <section className="flex flex-col gap-5 px-4 pt-12 sm:items-center sm:gap-0 sm:px-[clamp(16px,4vw,32px)] sm:pt-[clamp(56px,8vw,96px)] sm:text-center">
      <h1 className="m-0 max-w-[15em] text-[clamp(56px,9vw,124px)] leading-[.98] font-semi tracking-[-2px] text-balance sm:tracking-[-0.035em]">
        Payroll, kept private.
      </h1>
      <p className="m-0 max-w-[720px] text-lg leading-[1.3] text-pretty text-bark sm:mt-7 sm:text-[clamp(19px,1.9vw,25px)] sm:leading-[1.25] sm:tracking-[-0.3px]">
        Private streaming payroll for remote teams, freelancers, DAOs, and organizations that pay employees in ZEC.
      </p>
      <div className="mt-1 grid grid-cols-2 gap-2.5 sm:mt-6 sm:flex sm:flex-wrap sm:justify-center sm:gap-3">
        <Link href="/signup" className={`btn-amber ${bigButton} ${heroButton}`}>
          Create an account
        </Link>
        <a href="#how" className={`btn-ghost ${bigButton} ${heroButton}`}>
          See how it works
        </a>
      </div>
    </section>
  );
}

export function FinalCta() {
  return (
    <section className={`${containerClass} pb-16 sm:pb-[105px]`}>
      <div className="flex flex-col gap-[18px] border-t border-linen pt-14 sm:items-center sm:gap-6 sm:pt-24 sm:text-center">
        <h2 className="m-0 text-[clamp(38px,5.6vw,65px)] leading-none font-semi tracking-[-0.022em] max-sm:text-[42px] max-sm:tracking-[-1.2px]">
          Make payroll private by design.
        </h2>
        <p className={leadClass}>Give your team a clearer way to earn, claim, and receive ZEC.</p>
        <div className="flex flex-col gap-[18px] sm:flex-row sm:flex-wrap sm:justify-center sm:gap-3">
          <Link href="/signup" className={`btn-amber ${bigButton}`}>
            Create an account
          </Link>
          <Link href="/login" className={`btn-ghost ${bigButton}`}>
            Sign in
          </Link>
        </div>
      </div>
    </section>
  );
}

const footLink = "flex min-h-8 items-center text-sm text-espresso no-underline hover:text-bark sm:min-h-0";

export function Footer() {
  return (
    <footer className="border-t border-linen">
      <div className={`${containerClass} flex flex-col justify-between gap-6 pt-8 pb-10 text-sm sm:flex-row sm:flex-wrap sm:gap-8 sm:pt-10 sm:pb-12`}>
        <div className="flex max-w-[340px] flex-col gap-2.5 sm:gap-3">
          <Wordmark className="h-[22px] sm:h-6" />
          <p className="m-0 text-bark">Private streaming payroll with shielded Zcash payouts. Testnet preview.</p>
        </div>
        <nav aria-label="Footer" className="grid grid-cols-2 gap-x-4 gap-y-2.5 sm:flex sm:flex-wrap sm:gap-12">
          <div className="flex flex-col gap-2.5">
            <span className="font-semi max-sm:hidden">Product</span>
            <a className={footLink} href="#how">How it works</a>
            <a className={footLink} href="#employers">For employers</a>
            <a className={footLink} href="#workers">For workers</a>
          </div>
          <div className="flex flex-col gap-2.5">
            <span className="font-semi max-sm:hidden">Privacy &amp; code</span>
            <a className={footLink} href="#privacy">How shielding works</a>
            <a className={footLink} href="https://github.com/sniperchief/hushush">GitHub</a>
          </div>
        </nav>
      </div>
    </footer>
  );
}

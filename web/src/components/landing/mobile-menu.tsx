"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { NAV_GROUPS } from "./nav";
import { Wordmark } from "./ui";

const ease = "duration-300 ease-[cubic-bezier(.2,.8,.2,1)] motion-reduce:transition-none";

function MenuIcon({ open }: { open: boolean }) {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden="true">
      {open ? <path d="M6 6l12 12M18 6 6 18" /> : <path d="M4 7h16M4 12h16M4 17h10" />}
    </svg>
  );
}

/**
 * Phone navigation: a full-screen dialog that slides in from the right, the
 * side the menu button is on. Locks page scroll and keeps focus inside while open.
 */
export function MobileMenu({ signedIn = false }: { signedIn?: boolean }) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  /** Close from the × or Escape: focus goes back to the menu button. */
  const dismiss = () => {
    setOpen(false);
    triggerRef.current?.focus({ preventScroll: true });
  };

  useEffect(() => {
    if (!open) return;
    const panel = panelRef.current;
    document.body.style.overflow = "hidden";
    closeRef.current?.focus();

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        triggerRef.current?.focus({ preventScroll: true });
        return;
      }
      if (e.key !== "Tab" || !panel) return;
      const items = panel.querySelectorAll<HTMLElement>("a[href], button");
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last?.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first?.focus();
      }
    };
    // The menu only exists below md; don't leave the page locked after a rotate or resize.
    const wide = window.matchMedia("(min-width: 768px)");
    const onWide = () => wide.matches && setOpen(false);

    document.addEventListener("keydown", onKey);
    wide.addEventListener("change", onWide);
    return () => {
      document.body.style.overflow = "";
      document.removeEventListener("keydown", onKey);
      wide.removeEventListener("change", onWide);
    };
  }, [open]);

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        aria-label="Open menu"
        aria-expanded={open}
        aria-controls="mobile-menu"
        onClick={() => setOpen(true)}
        className="flex size-11 cursor-pointer items-center justify-center text-espresso"
      >
        <MenuIcon open={false} />
      </button>

      <div
        ref={panelRef}
        id="mobile-menu"
        role="dialog"
        aria-modal="true"
        aria-label="Menu"
        inert={!open}
        // Visible at once on open (so the × can take focus); hidden only after the slide-out.
        className={`fixed inset-0 z-50 flex flex-col bg-cream text-espresso md:hidden ${ease} ${
          open ? "visible translate-x-0 transition-transform" : "invisible translate-x-full transition-[transform,visibility]"
        }`}
      >
        {/* Mirrors the page header so the × lands where the menu button was. */}
        <div className="flex items-center justify-between border-b border-linen py-2.5 pr-2 pl-4">
          <Link href="/" aria-label="hushush home" onClick={() => setOpen(false)} className="flex items-center">
            <Wordmark className="h-[22px]" alt="" />
          </Link>
          <button ref={closeRef} type="button" aria-label="Close menu" onClick={dismiss} className="flex size-11 cursor-pointer items-center justify-center">
            <MenuIcon open />
          </button>
        </div>

        <nav aria-label="Primary" className="flex-1 overflow-y-auto px-4 pt-2">
          {NAV_GROUPS.map((g, gi) => (
            <section key={g.label} aria-labelledby={`menu-${g.label}`} className="pt-4">
              <h2 id={`menu-${g.label}`} className="m-0 pb-1 text-xs font-mid tracking-[0.08em] text-bark uppercase">
                {g.label}
              </h2>
              <ul className="m-0 list-none p-0">
                {g.items.map((l, i) => {
                  const n = NAV_GROUPS.slice(0, gi).reduce((k, x) => k + x.items.length, 0) + i;
                  return (
                    <li
                      key={l.href}
                      style={{ transitionDelay: open ? `${120 + n * 40}ms` : "0ms" }}
                      className={`border-b border-linen transition-[opacity,transform] ${ease} ${open ? "translate-x-0 opacity-100" : "translate-x-4 opacity-0"}`}
                    >
                      <a
                        href={l.href}
                        onClick={() => setOpen(false)}
                        className="flex min-h-16 items-center justify-between text-[28px] font-semi tracking-[-0.5px] text-espresso no-underline"
                      >
                        {l.label}
                        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className="text-bark">
                          <path d="m9 6 6 6-6 6" />
                        </svg>
                      </a>
                    </li>
                  );
                })}
              </ul>
            </section>
          ))}
        </nav>

        <div
          style={{ transitionDelay: open ? "280ms" : "0ms" }}
          className={`flex flex-col gap-2.5 px-4 pt-4 pb-[max(24px,env(safe-area-inset-bottom))] transition-opacity ${ease} ${open ? "opacity-100" : "opacity-0"}`}
        >
          {signedIn ? (
            <Link href="/app" className="btn-amber flex h-[52px] items-center justify-center rounded-lg text-base font-mid no-underline">
              Go to dashboard
            </Link>
          ) : (
            <>
              <Link href="/signup" className="btn-amber flex h-[52px] items-center justify-center rounded-lg text-base font-mid no-underline">
                Get started
              </Link>
              <Link href="/login" className="btn-ghost flex h-[52px] items-center justify-center rounded-lg text-base font-mid no-underline">
                Sign in
              </Link>
            </>
          )}
        </div>
      </div>
    </>
  );
}

"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "./icons";

export interface NavItem {
  href: string;
  label: string;
  icon: IconName;
  count?: number;
  /** Amber badge, hidden when the count is 0. */
  alert?: boolean;
  current?: boolean;
}

/** Sidebar links: the main sections, a divider, then the account section. */
export function SidebarLinks({ items, account }: { items: NavItem[]; account: NavItem[] }) {
  const link = (i: NavItem) => (
    <a
      key={i.label}
      href={i.href}
      aria-current={i.current ? "page" : undefined}
      className={`flex h-11 items-center gap-3 rounded-lg px-3.5 text-[15px] text-espresso no-underline hover:bg-mist ${i.current ? "bg-mist font-semi" : ""}`}
    >
      <Icon name={i.icon} className={i.current ? "text-espresso" : "text-bark"} />
      {i.label}
      {i.count !== undefined && (!i.alert || i.count > 0) && (
        <span className={`ml-auto rounded-full px-2 py-0.5 text-xs font-semi ${i.alert ? "bg-amber" : "border border-linen bg-cream"}`}>{i.count}</span>
      )}
    </a>
  );
  return (
    <div className="flex flex-col gap-1">
      {items.map(link)}
      <div className="mx-1 my-3 h-px bg-mist" />
      {account.map(link)}
    </div>
  );
}

/** Phones: menu button that opens the sidebar as a drawer from the left. */
export function MobileMenu({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const close = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    document.body.style.overflow = "hidden";
    close.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        button.current?.focus({ preventScroll: true });
      }
    };
    const wide = window.matchMedia("(min-width: 900px)");
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
        ref={button}
        type="button"
        aria-label="Open menu"
        aria-expanded={open}
        aria-controls="dashboard-drawer"
        onClick={() => setOpen(true)}
        className="flex size-10 flex-none cursor-pointer items-center justify-center rounded-lg bg-cream text-espresso"
      >
        <Icon name="menu" size={20} strokeWidth={2} />
      </button>
      <div
        className={`fixed inset-0 z-40 bg-espresso/30 transition-opacity duration-300 motion-reduce:transition-none ${open ? "opacity-100" : "pointer-events-none opacity-0"}`}
        onClick={() => setOpen(false)}
        aria-hidden="true"
      />
      <div
        id="dashboard-drawer"
        role="dialog"
        aria-modal="true"
        aria-label="Menu"
        inert={!open}
        // Following a section link closes the drawer.
        onClick={(e) => (e.target as HTMLElement).closest("a") && setOpen(false)}
        className={`fixed inset-y-0 left-0 z-50 flex w-[min(300px,85vw)] flex-col gap-5 overflow-y-auto bg-paper px-4 pt-4 pb-6 shadow-deck duration-300 ease-[cubic-bezier(.2,.8,.2,1)] motion-reduce:transition-none ${
          open ? "visible translate-x-0 transition-transform" : "invisible -translate-x-full transition-[transform,visibility]"
        }`}
      >
        <div className="flex justify-end">
          <button
            ref={close}
            type="button"
            aria-label="Close menu"
            onClick={() => {
              setOpen(false);
              button.current?.focus({ preventScroll: true });
            }}
            className="flex size-10 cursor-pointer items-center justify-center rounded-lg text-espresso"
          >
            <Icon name="close" size={20} strokeWidth={2} />
          </button>
        </div>
        {children}
      </div>
    </>
  );
}

/** Phones: fixed bottom tab bar. */
export function BottomTabs({ tabs }: { tabs: NavItem[] }) {
  return (
    <nav
      aria-label="Dashboard tabs"
      className="fixed inset-x-0 bottom-0 z-30 grid grid-cols-4 border-t border-linen bg-paper px-1 pt-1.5 pb-[max(14px,env(safe-area-inset-bottom))] min-[900px]:hidden"
    >
      {tabs.map((t) => (
        <a
          key={t.label}
          href={t.href}
          aria-current={t.current ? "page" : undefined}
          className={`flex min-h-12 flex-col items-center justify-center gap-[3px] text-[11.5px] no-underline ${t.current ? "font-semi text-espresso" : "text-bark"}`}
        >
          <span className="relative">
            <Icon name={t.icon} size={20} />
            {t.alert && (t.count ?? 0) > 0 && <span className="absolute -top-1 -right-1.5 size-2 rounded-full bg-amber" aria-hidden="true" />}
          </span>
          {t.label}
        </a>
      ))}
    </nav>
  );
}

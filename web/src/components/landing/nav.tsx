"use client";

import { useEffect, useId, useRef, useState } from "react";

/** The landing page's menus (desktop dropdowns and the phone menu share them). */
export const NAV_GROUPS = [
  {
    label: "Product",
    items: [
      { href: "#payroll", label: "Payroll" },
      { href: "#earn", label: "Earn" },
    ],
  },
  {
    label: "Company",
    items: [
      { href: "#how", label: "How it works" },
      { href: "#privacy", label: "Privacy" },
    ],
  },
] as const;

type Group = (typeof NAV_GROUPS)[number];

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={`transition-transform duration-200 motion-reduce:transition-none ${open ? "rotate-180" : ""}`}
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

/**
 * One header dropdown. Opens on hover, and on click / Enter / Space for touch
 * and keyboard users; closes on Escape (focus returns to the button), on a
 * click outside, when focus leaves it, or after following a link.
 */
function NavDropdown({ group }: { group: Group }) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const panelId = useId();

  const show = () => {
    clearTimeout(closeTimer.current);
    setOpen(true);
  };
  // A short grace period, so moving the pointer from the button to the panel doesn't close it.
  const hideSoon = () => {
    clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => setOpen(false), 120);
  };

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        buttonRef.current?.focus();
      }
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  useEffect(() => () => clearTimeout(closeTimer.current), []);

  return (
    <div
      ref={rootRef}
      className="relative"
      onPointerEnter={(e) => e.pointerType === "mouse" && show()}
      onPointerLeave={(e) => e.pointerType === "mouse" && hideSoon()}
      onBlur={(e) => {
        if (!rootRef.current?.contains(e.relatedTarget as Node)) setOpen(false);
      }}
    >
      <button
        ref={buttonRef}
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((o) => !o)}
        className="inline-flex min-h-10 cursor-pointer items-center gap-1 text-sm text-espresso hover:text-bark"
      >
        {group.label}
        <Chevron open={open} />
      </button>
      {/* pt-2 bridges the gap under the button so the hover isn't lost on the way down. */}
      <div
        id={panelId}
        hidden={!open}
        className="absolute top-full left-1/2 z-40 -translate-x-1/2 pt-2"
      >
        <ul className="m-0 min-w-[180px] list-none rounded-xl border border-linen bg-paper p-1.5 shadow-[0_12px_32px_-12px_rgba(43,31,22,0.28)]">
          {group.items.map((item) => (
            <li key={item.href}>
              <a
                href={item.href}
                onClick={() => setOpen(false)}
                className="block rounded-lg px-3 py-2.5 text-sm text-espresso no-underline hover:bg-cream focus-visible:bg-cream focus-visible:outline-none"
              >
                {item.label}
              </a>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

export function DesktopNav() {
  return (
    <nav aria-label="Primary" className="flex items-center gap-x-6">
      {NAV_GROUPS.map((g) => (
        <NavDropdown key={g.label} group={g} />
      ))}
    </nav>
  );
}

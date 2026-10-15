"use client";

import { useEffect, useRef, useState } from "react";
import { initials } from "@/lib/chart-data";
import { LogoutButton } from "../forms/logout-button";

/** Avatar (+ name on desktop) that opens a small menu with the account details and Sign out. */
export function ProfileMenu({ name, company, role }: { name: string; company?: string; role: "employer" | "worker" }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        button.current?.focus();
      }
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    // Move focus into the menu so keyboard users land on Sign out.
    root.current?.querySelector<HTMLElement>("[role=menu] button")?.focus();
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const tone = role === "employer" ? "bg-espresso text-amber" : "bg-honey text-espresso";

  return (
    <div ref={root} className="relative">
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls="profile-menu"
        aria-label={`Account menu for ${name}`}
        onClick={() => setOpen((o) => !o)}
        className="flex min-w-0 cursor-pointer items-center gap-2.5 rounded-full p-0.5 text-sm text-espresso hover:bg-cream min-[900px]:rounded-lg min-[900px]:py-1 min-[900px]:pr-2 min-[900px]:pl-1"
      >
        <span aria-hidden="true" className={`flex size-[34px] flex-none items-center justify-center rounded-full text-xs font-semi min-[900px]:size-9 min-[900px]:text-[13px] ${tone}`}>
          {initials(name)}
        </span>
        <span className="max-w-[180px] truncate max-[899px]:hidden">{name}</span>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true" className={`text-bark transition-transform max-[899px]:hidden motion-reduce:transition-none ${open ? "rotate-180" : ""}`}>
          <path d="m6 9 6 6 6-6" />
        </svg>
      </button>
      {open && (
        <div
          id="profile-menu"
          role="menu"
          aria-label="Account"
          className="absolute top-[calc(100%+8px)] right-0 z-50 w-64 overflow-hidden rounded-xl border border-linen bg-paper shadow-deck"
        >
          <div className="flex items-center gap-3 border-b border-mist px-4 py-3.5">
            <span aria-hidden="true" className={`flex size-9 flex-none items-center justify-center rounded-full text-[13px] font-semi ${tone}`}>
              {initials(name)}
            </span>
            <div className="min-w-0">
              <div className="truncate text-sm font-semi">{name}</div>
              <div className="truncate text-[12.5px] text-bark">{company ? `${company} · paying` : "Getting paid"}</div>
            </div>
          </div>
          <div className="p-1.5">
            <LogoutButton className="flex h-10 w-full cursor-pointer items-center rounded-lg px-3 text-left text-sm text-failed-ink hover:bg-cream focus-visible:bg-cream disabled:opacity-60" role="menuitem" />
          </div>
        </div>
      )}
    </div>
  );
}

import type { Metadata, Viewport } from "next";
import Link from "next/link";
import type { ReactNode } from "react";
import { LogoutButton } from "@/components/forms/logout-button";
import { currentUser } from "@/lib/session";
import "./globals.css";

export const metadata: Metadata = {
  title: { default: "hushush", template: "%s · hushush" },
  description: "Private, streaming ZEC payroll.",
};

export const viewport: Viewport = { width: "device-width", initialScale: 1 };

export default async function RootLayout({ children }: { children: ReactNode }) {
  const user = await currentUser();
  return (
    <html lang="en">
      <body className="min-h-dvh">
        <header className="border-b border-line bg-surface/80 backdrop-blur">
          <div className="mx-auto flex h-14 max-w-6xl items-center justify-between gap-4 px-4 sm:px-6">
            <Link href={user ? "/app" : "/"} className="flex items-center gap-2 font-semibold tracking-tight">
              <span aria-hidden className="grid h-7 w-7 place-items-center rounded-lg bg-ink text-sm text-surface">ħ</span>
              hushush
            </Link>
            {user ? (
              <div className="flex min-w-0 items-center gap-2 text-sm">
                <span className="hidden truncate text-ink-soft sm:inline">{user.displayName}</span>
                <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-ink-soft">{user.kind === "employer" ? "Employer" : "Worker"}</span>
                <LogoutButton />
              </div>
            ) : (
              <nav className="flex items-center gap-1 text-sm">
                <Link href="/login" className="rounded-lg px-3 py-2 hover:bg-muted">Sign in</Link>
                <Link href="/signup" className="rounded-lg bg-ink px-3 py-2 text-surface hover:opacity-90">Employer sign-up</Link>
              </nav>
            )}
          </div>
        </header>
        <main className="mx-auto max-w-6xl px-4 py-8 sm:px-6 sm:py-10">{children}</main>
      </body>
    </html>
  );
}

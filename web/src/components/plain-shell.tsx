import Link from "next/link";
import type { ReactNode } from "react";
import { AuthHeader, TestnetStrip } from "./auth-shell";
import { LogoutButton } from "./forms/logout-button";
import { currentUser } from "@/lib/session";

/** The plain app header (wordmark, name, Sign out) around a centered column. */
export async function PlainShell({ children }: { children: ReactNode }) {
  const user = await currentUser();
  return (
    <>
      {/* Same strip and wordmark header as sign-up / onboarding (light, landing palette). */}
      <div className="landing bg-cream font-landing text-espresso [color-scheme:light]">
        <TestnetStrip />
        <AuthHeader
          asideAlways
          aside={
            user ? (
              <div className="flex min-w-0 items-center gap-3 text-sm">
                <span className="hidden truncate text-bark sm:inline">{user.displayName}</span>
                <LogoutButton className="h-11 cursor-pointer px-3 text-sm font-semi text-espresso disabled:opacity-60 sm:h-9 sm:rounded-lg sm:border sm:border-stone sm:font-normal sm:hover:bg-paper" />
              </div>
            ) : (
              <nav className="flex items-center gap-1 text-sm">
                <Link href="/login" className="flex min-h-11 items-center px-3 font-semi text-espresso no-underline">
                  Sign in
                </Link>
                <Link href="/signup" className="btn-amber flex min-h-10 items-center rounded-lg px-3 font-semi no-underline">
                  Sign up
                </Link>
              </nav>
            )
          }
        />
      </div>
      <main className="mx-auto max-w-6xl px-4 py-8 sm:px-6 sm:py-10">{children}</main>
    </>
  );
}

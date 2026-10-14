import "server-only";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { getSessionUser, sessionCookieName, type SessionUser } from "./auth";

/** The signed-in account for a server component, or null. */
export async function currentUser(): Promise<SessionUser | null> {
  const jar = await cookies();
  return getSessionUser(jar.get(sessionCookieName())?.value);
}

/** Redirects to /login (preserving the destination) when signed out. */
export async function requirePageUser(next: string): Promise<SessionUser> {
  const user = await currentUser();
  if (!user) redirect(`/login?next=${encodeURIComponent(next)}`);
  return user;
}

/** Only allow same-site relative redirect targets. */
export function safeNext(next: string | string[] | undefined, fallback = "/app"): string {
  const n = Array.isArray(next) ? next[0] : next;
  return n && n.startsWith("/") && !n.startsWith("//") && !n.startsWith("/\\") ? n : fallback;
}

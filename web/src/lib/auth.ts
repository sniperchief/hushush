import "server-only";
import { randomUUID } from "node:crypto";
import type pg from "pg";
import { z } from "zod";
import { hashToken, newToken } from "./crypto";
import { db } from "./db";
import { env } from "./env";
import { AppError } from "./errors";
import { payroll } from "./payroll";

export const SESSION_ABSOLUTE_MS = 30 * 24 * 3600 * 1000; // 30 days
export const SESSION_IDLE_MS = 7 * 24 * 3600 * 1000; // 7 days

/** Fixed at sign-up. Employer and worker accounts never share screens. */
export type AccountKind = "employer" | "worker";

export interface SessionUser {
  id: string;
  displayName: string;
  kind: AccountKind;
}

export const displayNameSchema = z.string().trim().min(1, "Enter your name.").max(100, "Use at most 100 characters.");
export const kindSchema = z.enum(["employer", "worker"]);

/** Session cookie name; `__Host-` prefix (Secure, host-only) over HTTPS. */
export function sessionCookieName(): string {
  return env.appUrl.startsWith("https://") ? "__Host-hh_session" : "hh_session";
}

/**
 * Creates the payroll user and the web account for a new passkey sign-up,
 * inside `client`'s transaction for the web rows. Accounts have no e-mail:
 * payroll's `users.email` gets a unique placeholder under the reserved
 * `.invalid` TLD (RFC 2606), which is never shown or sent to.
 */
export async function createAccount(client: pg.PoolClient, kind: AccountKind, displayName: string): Promise<string> {
  const placeholder = `u-${randomUUID()}@users.hushush.invalid`;
  const { id } = await payroll.createUser(placeholder, displayName);
  await client.query("INSERT INTO accounts (user_id, kind) VALUES ($1, $2)", [id, kind]);
  return id;
}

export async function createSession(userId: string): Promise<{ token: string; expiresAt: Date }> {
  const token = newToken();
  const expiresAt = new Date(Date.now() + SESSION_ABSOLUTE_MS);
  await db().query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)", [
    hashToken(token),
    userId,
    expiresAt,
  ]);
  return { token, expiresAt };
}

/** Resolves a session token to its account, enforcing absolute and idle expiry. */
export async function getSessionUser(token: string | undefined): Promise<SessionUser | null> {
  if (!token || token.length > 200) return null;
  const { rows } = await db().query<{ id: string; display_name: string; kind: AccountKind; stale: boolean }>(
    `SELECT u.id, u.display_name, a.kind, (s.last_seen_at < now() - interval '5 minutes') AS stale
     FROM sessions s
     JOIN users u ON u.id = s.user_id
     JOIN accounts a ON a.user_id = u.id
     WHERE s.token_hash = $1
       AND s.expires_at > now()
       AND s.last_seen_at > now() - ($2 || ' milliseconds')::interval`,
    [hashToken(token), String(SESSION_IDLE_MS)],
  );
  const r = rows[0];
  if (!r) return null;
  if (r.stale) {
    await db().query("UPDATE sessions SET last_seen_at = now() WHERE token_hash = $1", [hashToken(token)]);
  }
  return { id: r.id, displayName: r.display_name, kind: r.kind };
}

export async function logout(token: string | undefined): Promise<void> {
  if (token) await db().query("DELETE FROM sessions WHERE token_hash = $1", [hashToken(token)]);
}

export function requireKind(user: SessionUser, kind: AccountKind): void {
  if (user.kind !== kind) {
    throw new AppError(
      403,
      "wrong_account",
      kind === "employer" ? "This needs an employer account." : "This needs a worker account.",
    );
  }
}

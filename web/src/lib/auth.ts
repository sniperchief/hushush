import "server-only";
import { randomUUID } from "node:crypto";
import type pg from "pg";
import { z } from "zod";
import { hashToken, newToken } from "./crypto";
import { db } from "./db";
import { env } from "./env";
import { payroll } from "./payroll";

export const SESSION_ABSOLUTE_MS = 30 * 24 * 3600 * 1000; // 30 days
export const SESSION_IDLE_MS = 7 * 24 * 3600 * 1000; // 7 days

/**
 * The answer to "How will you use hushush?": only picks the screen an account
 * starts on, and can be changed any time. It grants nothing; payroll decides
 * who may manage a company or withdraw from a stream. null = not answered yet.
 */
export type Intent = "pay" | "get_paid";

export interface SessionUser {
  id: string;
  displayName: string;
  intent: Intent | null;
}

export const displayNameSchema = z.string().trim().min(1, "Enter your name.").max(100, "Use at most 100 characters.");
export const intentSchema = z.enum(["pay", "get_paid"]);

/** Session cookie name; `__Host-` prefix (Secure, host-only) over HTTPS. */
export function sessionCookieName(): string {
  return env.appUrl.startsWith("https://") ? "__Host-hh_session" : "hh_session";
}

/**
 * Creates the payroll user and the web account for a new sign-up,
 * inside `client`'s transaction for the web rows. Accounts have no e-mail:
 * payroll's `users.email` gets a unique placeholder under the reserved
 * `.invalid` TLD (RFC 2606), which is never shown or sent to.
 */
export async function createAccount(client: pg.PoolClient, displayName: string): Promise<string> {
  const placeholder = `u-${randomUUID()}@users.hushush.invalid`;
  const { id } = await payroll.createUser(placeholder, displayName);
  await client.query("INSERT INTO accounts (user_id) VALUES ($1)", [id]);
  return id;
}

export async function setIntent(userId: string, intent: Intent): Promise<void> {
  await db().query("UPDATE accounts SET intent = $2 WHERE user_id = $1", [userId, intent]);
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
  const { rows } = await db().query<{ id: string; display_name: string; intent: Intent | null; stale: boolean }>(
    `SELECT u.id, u.display_name, a.intent, (s.last_seen_at < now() - interval '5 minutes') AS stale
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
  return { id: r.id, displayName: r.display_name, intent: r.intent };
}

export async function logout(token: string | undefined): Promise<void> {
  if (token) await db().query("DELETE FROM sessions WHERE token_hash = $1", [hashToken(token)]);
}

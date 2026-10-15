import "server-only";
import { z } from "zod";
import { createAccount, createSession, type SessionUser } from "./auth";
import { hashToken, newToken } from "./crypto";
import { db } from "./db";
import { env } from "./env";
import { AppError } from "./errors";
import { sendMail } from "./mail";

// E-mail sign-in links, alongside passkeys. Optional for any account.
//
// A link carries a random token; only its hash is stored. Tokens are single
// use and valid for 15 minutes. The link opens a confirmation page, and only
// its button (a POST) consumes the token, so mail scanners that prefetch links
// can't use it up. Responses never say whether an address has an account.

export const LINK_TTL_MS = 15 * 60 * 1000;
/** Links per address per TTL window, across all purposes. */
export const MAX_LINKS_PER_WINDOW = 3;

export const emailSchema = z
  .string()
  .trim()
  .toLowerCase()
  .pipe(z.email("Enter a valid email address.").max(254, "Use at most 254 characters."));

type Purpose = "signup" | "login" | "link";

const linkExpired = () => new AppError(410, "link_expired", "This sign-in link has expired or was already used. Request a new one.");

/** The account that signs in with `email`, if any. */
async function accountForEmail(email: string): Promise<string | null> {
  const { rows } = await db().query<{ user_id: string }>("SELECT user_id FROM account_emails WHERE email_lower = $1", [email]);
  return rows[0]?.user_id ?? null;
}

/** The sign-in address of an account, if it has one. */
export async function accountEmail(userId: string): Promise<string | null> {
  const { rows } = await db().query<{ email_lower: string }>("SELECT email_lower FROM account_emails WHERE user_id = $1", [userId]);
  return rows[0]?.email_lower ?? null;
}

/** Stores a token and returns it, or null when the address hit its rate limit. */
async function issueToken(purpose: Purpose, email: string, userId: string | null, payload: object = {}): Promise<string | null> {
  await db().query("DELETE FROM email_login_tokens WHERE expires_at < now() - interval '1 day'");
  const { rows } = await db().query<{ n: string }>(
    "SELECT count(*) AS n FROM email_login_tokens WHERE email_lower = $1 AND created_at > now() - ($2 || ' milliseconds')::interval",
    [email, String(LINK_TTL_MS)],
  );
  if (Number(rows[0]?.n ?? 0) >= MAX_LINKS_PER_WINDOW) return null;
  const token = newToken();
  await db().query(
    "INSERT INTO email_login_tokens (token_hash, purpose, email_lower, user_id, payload, expires_at) VALUES ($1, $2, $3, $4, $5, $6)",
    [hashToken(token), purpose, email, userId, JSON.stringify(payload), new Date(Date.now() + LINK_TTL_MS)],
  );
  return token;
}

const linkUrl = (token: string) => `${env.appUrl}/auth/email?token=${encodeURIComponent(token)}`;
const minutes = LINK_TTL_MS / 60_000;
const footer = `The link works once and expires in ${minutes} minutes.\n\nIf you didn't ask for this, you can ignore this email.\n\nhushush · private streaming payroll`;

/** Login form: e-mails a sign-in link if the address belongs to an account. */
export async function requestLogin(email: string): Promise<void> {
  const userId = await accountForEmail(email);
  if (!userId) return;
  const token = await issueToken("login", email, userId);
  if (!token) return;
  await sendMail({
    to: email,
    subject: "Your hushush sign-in link",
    text: `Sign in to hushush:\n\n${linkUrl(token)}\n\n${footer}`,
  });
}

/** Sign-up form: e-mails a link that creates the account (or signs in, if the address already has one). */
export async function requestSignup(email: string, displayName: string): Promise<void> {
  const existing = await accountForEmail(email);
  if (existing) {
    const token = await issueToken("login", email, existing);
    if (!token) return;
    await sendMail({
      to: email,
      subject: "Your hushush sign-in link",
      text: `You already have a hushush account with this email, so there's nothing to sign up for. Sign in instead:\n\n${linkUrl(token)}\n\n${footer}`,
    });
    return;
  }
  const token = await issueToken("signup", email, null, { displayName });
  if (!token) return;
  await sendMail({
    to: email,
    subject: "Finish creating your hushush account",
    text: `Confirm your email to create your hushush account:\n\n${linkUrl(token)}\n\n${footer}`,
  });
}

/** Signed-in account: e-mails a link that makes `email` its sign-in address. */
export async function requestLink(user: SessionUser, email: string): Promise<void> {
  const token = await issueToken("link", email, user.id);
  if (!token) return;
  await sendMail({
    to: email,
    subject: "Confirm your hushush sign-in email",
    text: `Confirm this address so you can sign in to hushush with an email link (for example if you lose your passkey):\n\n${linkUrl(token)}\n\n${footer}`,
  });
}

/** What a link would do, without using it (for the confirmation page). */
export async function peekLink(token: string | undefined): Promise<{ purpose: Purpose; email: string } | null> {
  if (!token || token.length > 200) return null;
  const { rows } = await db().query<{ purpose: Purpose; email_lower: string }>(
    "SELECT purpose, email_lower FROM email_login_tokens WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()",
    [hashToken(token)],
  );
  return rows[0] ? { purpose: rows[0].purpose, email: rows[0].email_lower } : null;
}

export type LinkResult =
  | { purpose: "signup" | "login"; userId: string; session: { token: string; expiresAt: Date } }
  | { purpose: "link"; userId: string };

/** Uses a link: single use, even when the follow-up step then fails. */
export async function consumeLink(token: string): Promise<LinkResult> {
  if (token.length > 200) throw linkExpired();
  const { rows } = await db().query<{ purpose: Purpose; email_lower: string; user_id: string | null; payload: { displayName?: string } }>(
    `UPDATE email_login_tokens SET used_at = now()
     WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
     RETURNING purpose, email_lower, user_id, payload`,
    [hashToken(token)],
  );
  const t = rows[0];
  if (!t) throw linkExpired();

  if (t.purpose === "link") {
    try {
      await db().query(
        `INSERT INTO account_emails (user_id, email_lower) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET email_lower = EXCLUDED.email_lower, created_at = now()`,
        [t.user_id, t.email_lower],
      );
    } catch (e) {
      if ((e as { code?: string }).code === "23505") {
        throw new AppError(409, "email_taken", "This email is already used by another hushush account.");
      }
      throw e;
    }
    return { purpose: "link", userId: t.user_id! };
  }

  if (t.purpose === "login") {
    // The address may have been changed since the link was sent.
    if ((await accountForEmail(t.email_lower)) !== t.user_id) throw linkExpired();
    return { purpose: "login", userId: t.user_id!, session: await createSession(t.user_id!) };
  }

  // signup. Whoever controls the address owns its account, so if one was
  // created meanwhile (another link, a race), sign in to it instead.
  const existing = await accountForEmail(t.email_lower);
  if (existing) return { purpose: "login", userId: existing, session: await createSession(existing) };
  const client = await db().connect();
  let userId: string;
  try {
    await client.query("BEGIN");
    userId = await createAccount(client, t.payload.displayName ?? "hushush user");
    await client.query("INSERT INTO account_emails (user_id, email_lower) VALUES ($1, $2)", [userId, t.email_lower]);
    await client.query("COMMIT");
  } catch (e) {
    await client.query("ROLLBACK");
    if ((e as { code?: string }).code === "23505") {
      const winner = await accountForEmail(t.email_lower);
      if (winner) return { purpose: "login", userId: winner, session: await createSession(winner) };
    }
    throw e;
  } finally {
    client.release();
  }
  return { purpose: "signup", userId, session: await createSession(userId) };
}

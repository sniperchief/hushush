import "server-only";
import { z } from "zod";
import { DUMMY_PASSWORD_HASH, hashPassword, hashToken, newToken, verifyPassword } from "./crypto";
import { db } from "./db";
import { env } from "./env";
import { AppError } from "./errors";
import { sendMail } from "./mail";
import { PayrollApiError, payroll } from "./payroll";

export const SESSION_ABSOLUTE_MS = 30 * 24 * 3600 * 1000; // 30 days
export const SESSION_IDLE_MS = 7 * 24 * 3600 * 1000; // 7 days
const VERIFY_TTL_MS = 24 * 3600 * 1000;
const LOGIN_WINDOW_MIN = 15;
const LOGIN_MAX_FAILURES = 10;

export interface SessionUser {
  id: string;
  email: string;
  displayName: string;
}

export const emailSchema = z
  .string()
  .trim()
  .min(3)
  .max(320)
  .pipe(z.email({ message: "Enter a valid email address." }));

export const signupSchema = z.object({
  email: emailSchema,
  displayName: z.string().trim().min(1, "Enter your name.").max(100),
  password: z
    .string()
    .min(12, "Use at least 12 characters.")
    .max(256, "Use at most 256 characters."),
});

export const loginSchema = z.object({
  email: emailSchema,
  password: z.string().min(1).max(256),
});

/** Session cookie name; `__Host-` prefix (Secure, host-only) over HTTPS. */
export function sessionCookieName(): string {
  return env.appUrl.startsWith("https://") ? "__Host-hh_session" : "hh_session";
}

async function sendVerification(userId: string, email: string): Promise<void> {
  const token = newToken();
  await db().query(
    "INSERT INTO email_verification_tokens (token_hash, user_id, expires_at) VALUES ($1, $2, $3)",
    [hashToken(token), userId, new Date(Date.now() + VERIFY_TTL_MS)],
  );
  await sendMail({
    to: email,
    subject: "Verify your hushush email",
    text: `Confirm your email address to finish creating your hushush account:\n\n${env.appUrl}/verify?token=${token}\n\nThis link expires in 24 hours. If you didn't sign up, ignore this email.`,
  });
}

/**
 * Creates an account and emails a verification link. The response is the same
 * whether or not the email is already registered (no account enumeration);
 * an existing account holder gets a notice email instead.
 */
export async function signup(input: z.input<typeof signupSchema>): Promise<void> {
  const { email, displayName, password } = signupSchema.parse(input);
  const passwordHash = await hashPassword(password);
  let userId: string;
  try {
    userId = (await payroll.createUser(email, displayName)).id;
  } catch (e) {
    if (!(e instanceof PayrollApiError && e.code === "conflict")) {
      if (e instanceof PayrollApiError && e.status === 422) throw new AppError(422, "validation", e.message);
      throw e;
    }
    const { rows } = await db().query<{ id: string; has_cred: boolean }>(
      `SELECT u.id, (c.user_id IS NOT NULL) AS has_cred
       FROM users u LEFT JOIN auth_credentials c ON c.user_id = u.id
       WHERE lower(u.email) = lower($1)`,
      [email],
    );
    const existing = rows[0];
    if (!existing) throw e;
    if (existing.has_cred) {
      await sendMail({
        to: email,
        subject: "You already have a hushush account",
        text: `Someone tried to sign up with this email. You already have an account — sign in at ${env.appUrl}/login.\nIf this wasn't you, no action is needed.`,
      });
      return;
    }
    // A previous sign-up created the user but not the credentials; finish it.
    userId = existing.id;
  }
  const { rowCount } = await db().query(
    "INSERT INTO auth_credentials (user_id, password_hash) VALUES ($1, $2) ON CONFLICT (user_id) DO NOTHING",
    [userId, passwordHash],
  );
  if (rowCount) await sendVerification(userId, email);
}

/** Re-sends a verification email if the account exists and is unverified. */
export async function resendVerification(emailInput: string): Promise<void> {
  const email = emailSchema.parse(emailInput);
  const { rows } = await db().query<{ id: string; email: string; recent: string }>(
    `SELECT u.id, u.email,
            (SELECT COUNT(*) FROM email_verification_tokens t
             WHERE t.user_id = u.id AND t.created_at > now() - interval '1 hour') AS recent
     FROM users u JOIN auth_credentials c ON c.user_id = u.id
     WHERE lower(u.email) = lower($1) AND c.email_verified_at IS NULL`,
    [email],
  );
  const u = rows[0];
  if (u && Number(u.recent) < 3) await sendVerification(u.id, u.email);
}

/** Consumes a verification token. Single use; expires after 24 hours. */
export async function verifyEmail(token: string): Promise<boolean> {
  if (!token || token.length > 200) return false;
  const client = await db().connect();
  try {
    await client.query("BEGIN");
    const { rows } = await client.query<{ user_id: string }>(
      `UPDATE email_verification_tokens SET used_at = now()
       WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
       RETURNING user_id`,
      [hashToken(token)],
    );
    const userId = rows[0]?.user_id;
    if (userId) {
      await client.query(
        "UPDATE auth_credentials SET email_verified_at = COALESCE(email_verified_at, now()), updated_at = now() WHERE user_id = $1",
        [userId],
      );
    }
    await client.query("COMMIT");
    return Boolean(userId);
  } catch (e) {
    await client.query("ROLLBACK");
    throw e;
  } finally {
    client.release();
  }
}

/** Verifies credentials and opens a session. Returns the raw session token. */
export async function login(input: z.input<typeof loginSchema>): Promise<{ token: string; expiresAt: Date }> {
  const { email, password } = loginSchema.parse(input);
  const emailLower = email.toLowerCase();
  const { rows: fails } = await db().query<{ n: string }>(
    `SELECT COUNT(*) AS n FROM login_attempts
     WHERE email_lower = $1 AND NOT succeeded AND attempted_at > now() - ($2 || ' minutes')::interval`,
    [emailLower, String(LOGIN_WINDOW_MIN)],
  );
  if (Number(fails[0]?.n ?? 0) >= LOGIN_MAX_FAILURES) {
    throw new AppError(429, "too_many_attempts", "Too many failed sign-in attempts. Try again in 15 minutes.");
  }
  const { rows } = await db().query<{ id: string; password_hash: string; verified: boolean }>(
    `SELECT u.id, c.password_hash, (c.email_verified_at IS NOT NULL) AS verified
     FROM users u JOIN auth_credentials c ON c.user_id = u.id WHERE lower(u.email) = $1`,
    [emailLower],
  );
  const user = rows[0];
  // Always run scrypt so response time doesn't reveal whether the email exists.
  const ok = await verifyPassword(password, user?.password_hash ?? DUMMY_PASSWORD_HASH);
  await db().query("INSERT INTO login_attempts (email_lower, succeeded) VALUES ($1, $2)", [
    emailLower,
    Boolean(user && ok),
  ]);
  if (!user || !ok) throw new AppError(401, "invalid_credentials", "Incorrect email or password.");
  if (!user.verified) {
    throw new AppError(403, "email_not_verified", "Verify your email address before signing in. Check your inbox.");
  }
  return createSession(user.id);
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

/** Resolves a session token to a verified user, enforcing absolute and idle expiry. */
export async function getSessionUser(token: string | undefined): Promise<SessionUser | null> {
  if (!token || token.length > 200) return null;
  const { rows } = await db().query<{ id: string; email: string; display_name: string; stale: boolean }>(
    `SELECT u.id, u.email, u.display_name, (s.last_seen_at < now() - interval '5 minutes') AS stale
     FROM sessions s
     JOIN users u ON u.id = s.user_id
     JOIN auth_credentials c ON c.user_id = u.id
     WHERE s.token_hash = $1
       AND s.expires_at > now()
       AND s.last_seen_at > now() - ($2 || ' milliseconds')::interval
       AND c.email_verified_at IS NOT NULL`,
    [hashToken(token), String(SESSION_IDLE_MS)],
  );
  const r = rows[0];
  if (!r) return null;
  if (r.stale) {
    await db().query("UPDATE sessions SET last_seen_at = now() WHERE token_hash = $1", [hashToken(token)]);
  }
  return { id: r.id, email: r.email, displayName: r.display_name };
}

export async function logout(token: string | undefined): Promise<void> {
  if (token) await db().query("DELETE FROM sessions WHERE token_hash = $1", [hashToken(token)]);
}

// E-mail sign-in links for employers, through the real route handlers. Mail
// goes to the dev outbox (no SMTP in tests), which is where links are read from.
import { randomUUID } from "node:crypto";
import { describe, expect, it } from "vitest";
import * as accountEmailRoute from "@/app/api/account/email/route";
import * as startRoute from "@/app/api/auth/email/start/route";
import * as verifyRoute from "@/app/api/auth/email/verify/route";
import { getSessionUser } from "@/lib/auth";
import { db } from "@/lib/db";
import { peekLink } from "@/lib/email-login";
import { call, newEmployer, newWorker } from "./support/helpers";

const address = () => `owner-${randomUUID()}@Example.test`;

/** Mails sent to `to`, oldest first. */
async function outbox(to: string): Promise<{ subject: string; body: string }[]> {
  const { rows } = await db().query("SELECT subject, body FROM dev_outbox WHERE to_address = $1 ORDER BY id", [to.toLowerCase()]);
  return rows;
}

/** The token in the latest mail to `to`. */
async function latestToken(to: string): Promise<string> {
  const mails = await outbox(to);
  const m = mails.at(-1)?.body.match(/\/auth\/email\?token=([^\s]+)/);
  expect(m, "no link mailed").toBeTruthy();
  return decodeURIComponent(m![1]!);
}

const start = (body: Record<string, string>, opts: { origin?: string | null } = {}) => call(startRoute.POST, "POST", {}, { body, ...opts });
const verify = (token: string) => call(verifyRoute.POST, "POST", {}, { body: { token } });
const sessionOf = (r: { cookies: Record<string, string> }) => getSessionUser(r.cookies.hh_session);

describe("e-mail sign-up", () => {
  it("creates an account only once the link is used", async () => {
    const email = address();
    const s = await start({ mode: "signup", email, displayName: "Mail Owner" });
    expect(s.status).toBe(202);
    const mails = await outbox(email);
    expect(mails).toHaveLength(1);
    expect(mails[0]!.subject).toBe("Finish creating your hushush account");
    const before = Number((await db().query("SELECT count(*) FROM accounts")).rows[0].count);

    const token = await latestToken(email);
    // Looking at the link (the confirmation page) doesn't use it.
    expect(await peekLink(token)).toEqual({ purpose: "signup", email: email.toLowerCase() });
    expect(Number((await db().query("SELECT count(*) FROM accounts")).rows[0].count)).toBe(before);

    const v = await verify(token);
    expect(v.status, JSON.stringify(v.body)).toBe(200);
    expect(v.body).toEqual({ purpose: "signup" });
    const user = await sessionOf(v);
    expect(user).toMatchObject({ displayName: "Mail Owner", intent: null });
    const { rows } = await db().query("SELECT email_lower FROM account_emails WHERE user_id = $1", [user!.id]);
    expect(rows[0].email_lower).toBe(email.toLowerCase());
  });

  it("stores only token hashes, and a link works once", async () => {
    const email = address();
    await start({ mode: "signup", email, displayName: "Once" });
    const token = await latestToken(email);
    const { rows: raw } = await db().query("SELECT 1 FROM email_login_tokens WHERE token_hash = $1", [token]);
    expect(raw).toHaveLength(0);
    expect((await verify(token)).status).toBe(200);
    const again = await verify(token);
    expect(again.status).toBe(410);
    expect(again.cookies.hh_session).toBeUndefined();
    expect(await peekLink(token)).toBeNull();
  });

  it("an address that already has an account gets a sign-in link instead of a second account", async () => {
    const email = address();
    await start({ mode: "signup", email, displayName: "First" });
    const first = await sessionOf(await verify(await latestToken(email)));

    await start({ mode: "signup", email, displayName: "Second" });
    expect((await outbox(email)).at(-1)!.body).toMatch(/already have a hushush account/);
    const v = await verify(await latestToken(email));
    expect(v.body).toEqual({ purpose: "login" });
    expect((await sessionOf(v))!.id).toBe(first!.id);
  });

  it("two pending sign-up links for one address end in one account", async () => {
    const email = address();
    await start({ mode: "signup", email, displayName: "A" });
    const a = await latestToken(email);
    await start({ mode: "signup", email, displayName: "B" });
    const b = await latestToken(email);
    const ua = await sessionOf(await verify(a));
    const vb = await verify(b);
    expect(vb.body).toEqual({ purpose: "login" });
    expect((await sessionOf(vb))!.id).toBe(ua!.id);
  });
});

describe("e-mail sign-in", () => {
  it("signs in to the account that owns the address", async () => {
    const email = address();
    await start({ mode: "signup", email, displayName: "Owner" });
    const owner = await sessionOf(await verify(await latestToken(email)));

    const s = await start({ mode: "login", email: email.toUpperCase() });
    expect(s.status).toBe(202);
    expect((await outbox(email)).at(-1)!.subject).toBe("Your hushush sign-in link");
    const v = await verify(await latestToken(email));
    expect(v.body).toEqual({ purpose: "login" });
    expect((await sessionOf(v))!.id).toBe(owner!.id);
  });

  it("answers the same for unknown addresses and sends nothing", async () => {
    const email = address();
    const s = await start({ mode: "login", email });
    expect(s.status).toBe(202);
    expect(s.body).toEqual({ ok: true });
    expect(await outbox(email)).toHaveLength(0);
  });

  it("sends at most 3 links per address per 15 minutes", async () => {
    const email = address();
    for (let i = 0; i < 5; i++) expect((await start({ mode: "signup", email, displayName: "R" })).status).toBe(202);
    expect(await outbox(email)).toHaveLength(3);
  });

  it("expired links are refused", async () => {
    const email = address();
    await start({ mode: "signup", email, displayName: "Late" });
    const token = await latestToken(email);
    await db().query("UPDATE email_login_tokens SET expires_at = now() - interval '1 second' WHERE email_lower = $1", [email.toLowerCase()]);
    expect((await verify(token)).status).toBe(410);
  });

  it("rejects cross-site requests and invalid input", async () => {
    expect((await start({ mode: "login", email: address() }, { origin: "https://evil.example" })).status).toBe(403);
    expect((await start({ mode: "login", email: "not-an-email" })).status).toBe(422);
    expect((await start({ mode: "signup", email: address(), displayName: " " })).status).toBe(422);
    expect((await verify("x".repeat(43))).status).toBe(410);
  });
});

describe("backup sign-in e-mail for passkey accounts", () => {
  it("is set by confirming a link, then works for e-mail sign-in", async () => {
    const employer = await newEmployer("Passkey Boss");
    const email = address();
    const r = await call(accountEmailRoute.POST, "POST", {}, { cookie: employer.cookie, body: { email } });
    expect(r.status, JSON.stringify(r.body)).toBe(202);
    expect((await outbox(email)).at(-1)!.subject).toBe("Confirm your hushush sign-in email");

    const v = await verify(await latestToken(email));
    expect(v.body).toEqual({ purpose: "link" });
    expect(v.cookies.hh_session).toBeUndefined(); // confirming an address doesn't sign anyone in

    await start({ mode: "login", email });
    const login = await verify(await latestToken(email));
    expect((await sessionOf(login))!.id).toBe(employer.id);
  });

  it("changing the address voids sign-in links sent to the old one", async () => {
    const employer = await newEmployer("Mover");
    const oldEmail = address();
    await call(accountEmailRoute.POST, "POST", {}, { cookie: employer.cookie, body: { email: oldEmail } });
    await verify(await latestToken(oldEmail));
    await start({ mode: "login", email: oldEmail });
    const stale = await latestToken(oldEmail);

    const newEmail = address();
    await call(accountEmailRoute.POST, "POST", {}, { cookie: employer.cookie, body: { email: newEmail } });
    await verify(await latestToken(newEmail));
    expect((await verify(stale)).status).toBe(410);
  });

  it("can't take an address another account already uses", async () => {
    const email = address();
    await start({ mode: "signup", email, displayName: "Holder" });
    await verify(await latestToken(email));
    const other = await newEmployer("Grabber");
    await call(accountEmailRoute.POST, "POST", {}, { cookie: other.cookie, body: { email } });
    const v = await verify(await latestToken(email));
    expect(v.status).toBe(409);
  });

  it("works for any signed-in account, including people who get paid", async () => {
    expect((await call(accountEmailRoute.POST, "POST", {}, { body: { email: address() } })).status).toBe(401);
    const worker = await newWorker("Mail Worker");
    const email = address();
    expect((await call(accountEmailRoute.POST, "POST", {}, { cookie: worker.cookie, body: { email } })).status).toBe(202);
    expect((await verify(await latestToken(email))).body).toEqual({ purpose: "link" });
    await start({ mode: "login", email });
    expect((await sessionOf(await verify(await latestToken(email))))!.id).toBe(worker.id);
  });
});

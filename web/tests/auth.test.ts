import { describe, expect, it } from "vitest";
import { hashPassword, verifyPassword } from "@/lib/crypto";
import { getSessionUser } from "@/lib/auth";
import { db } from "@/lib/db";
import * as signupRoute from "@/app/api/auth/signup/route";
import * as loginRoute from "@/app/api/auth/login/route";
import * as logoutRoute from "@/app/api/auth/logout/route";
import * as verifyRoute from "@/app/api/auth/verify/route";
import * as orgsRoute from "@/app/api/orgs/route";
import { PASSWORD, call, lastMail, newUser, signIn, tokenFrom, uniqueEmail } from "./support/helpers";

const tokenOf = (cookie: string) => decodeURIComponent(cookie.split("=")[1] ?? "");

describe("password hashing", () => {
  it("hashes with scrypt + salt and verifies in constant time", async () => {
    const h1 = await hashPassword("a long enough passphrase");
    const h2 = await hashPassword("a long enough passphrase");
    expect(h1).toMatch(/^scrypt\$131072\$8\$1\$/);
    expect(h1).not.toBe(h2); // random salt
    expect(await verifyPassword("a long enough passphrase", h1)).toBe(true);
    expect(await verifyPassword("a long enough passphrasE", h1)).toBe(false);
    expect(await verifyPassword("x", "not-a-hash")).toBe(false);
  });
});

describe("sign-up, verification and sign-in", () => {
  it("requires a verified email before sign-in, and verification links are single-use", async () => {
    const email = uniqueEmail("v");
    const s = await call(signupRoute.POST, "POST", {}, { body: { email, displayName: "Vera", password: PASSWORD } });
    expect(s.status).toBe(202);

    const early = await call(loginRoute.POST, "POST", {}, { body: { email, password: PASSWORD } });
    expect(early.status).toBe(403);
    expect(early.body.error).toBe("email_not_verified");

    expect((await call(verifyRoute.POST, "POST", {}, { body: { token: "bogus" } })).status).toBe(400);
    const token = tokenFrom((await lastMail(email)).body, "verify");
    expect((await call(verifyRoute.POST, "POST", {}, { body: { token } })).status).toBe(200);
    expect((await call(verifyRoute.POST, "POST", {}, { body: { token } })).status).toBe(400);

    const ok = await call(loginRoute.POST, "POST", {}, { body: { email: email.toUpperCase(), password: PASSWORD } });
    expect(ok.status).toBe(200);
    const setCookie = ok.res.headers.get("set-cookie") ?? "";
    expect(setCookie).toMatch(/HttpOnly/i);
    expect(setCookie).toMatch(/SameSite=lax/i);
  });

  it("stores only hashes of passwords and tokens", async () => {
    const u = await newUser("hash");
    const { rows } = await db().query("SELECT password_hash FROM auth_credentials WHERE user_id = $1", [u.id]);
    expect(rows[0].password_hash).not.toContain(PASSWORD);
    const raw = tokenOf(u.cookie);
    const s = await db().query("SELECT 1 FROM sessions WHERE token_hash = $1", [raw]);
    expect(s.rowCount).toBe(0); // raw token is never stored
  });

  it("rejects wrong passwords and unknown emails with the same error", async () => {
    const u = await newUser("wp");
    const a = await call(loginRoute.POST, "POST", {}, { body: { email: u.email, password: "wrong password here" } });
    const b = await call(loginRoute.POST, "POST", {}, { body: { email: uniqueEmail("nobody"), password: "wrong password here" } });
    expect([a.status, b.status]).toEqual([401, 401]);
    expect(a.body).toEqual(b.body);
  });

  it("does not reveal whether an email is registered, and keeps the original password", async () => {
    const u = await newUser("dup");
    const again = await call(signupRoute.POST, "POST", {}, { body: { email: u.email, displayName: "Impostor", password: "another long password" } });
    expect(again.status).toBe(202);
    expect((await lastMail(u.email)).subject).toMatch(/already have/i);
    await signIn(u.email); // original password still works
    const hijack = await call(loginRoute.POST, "POST", {}, { body: { email: u.email, password: "another long password" } });
    expect(hijack.status).toBe(401);
  });

  it("enforces password policy and input validation", async () => {
    const weak = await call(signupRoute.POST, "POST", {}, { body: { email: uniqueEmail("w"), displayName: "W", password: "short" } });
    expect(weak.status).toBe(422);
    const bad = await call(signupRoute.POST, "POST", {}, { body: { email: "not-an-email", displayName: "W", password: PASSWORD } });
    expect(bad.status).toBe(422);
  });

  it("throttles repeated failed sign-ins", async () => {
    const u = await newUser("thr");
    for (let i = 0; i < 10; i++) {
      await call(loginRoute.POST, "POST", {}, { body: { email: u.email, password: "nope nope nope" } });
    }
    const blocked = await call(loginRoute.POST, "POST", {}, { body: { email: u.email, password: PASSWORD } });
    expect(blocked.status).toBe(429);
  });
});

describe("sessions", () => {
  it("logout invalidates the session server-side", async () => {
    const u = await newUser("lo");
    expect(await getSessionUser(tokenOf(u.cookie))).not.toBeNull();
    expect((await call(logoutRoute.POST, "POST", {}, { cookie: u.cookie })).status).toBe(200);
    expect(await getSessionUser(tokenOf(u.cookie))).toBeNull();
    const r = await call(orgsRoute.POST, "POST", {}, { cookie: u.cookie, body: { name: "X" } });
    expect(r.status).toBe(401);
  });

  it("expires sessions after the absolute lifetime and after idling", async () => {
    const a = await newUser("exp");
    await db().query("UPDATE sessions SET expires_at = now() - interval '1 second', created_at = now() - interval '31 days' WHERE user_id = $1", [a.id]);
    expect(await getSessionUser(tokenOf(a.cookie))).toBeNull();
    const b = await newUser("idle");
    await db().query("UPDATE sessions SET last_seen_at = now() - interval '8 days' WHERE user_id = $1", [b.id]);
    expect(await getSessionUser(tokenOf(b.cookie))).toBeNull();
  });

  it("rejects forged and missing session cookies", async () => {
    const r = await call(orgsRoute.POST, "POST", {}, { cookie: "hh_session=forged", body: { name: "X" } });
    expect(r.status).toBe(401);
    expect((await call(orgsRoute.POST, "POST", {}, { body: { name: "X" } })).status).toBe(401);
  });
});

describe("CSRF and request hygiene", () => {
  it("blocks cross-site and origin-less state changes", async () => {
    const u = await newUser("csrf");
    expect((await call(orgsRoute.POST, "POST", {}, { cookie: u.cookie, origin: "https://evil.example", body: { name: "X" } })).status).toBe(403);
    expect((await call(orgsRoute.POST, "POST", {}, { cookie: u.cookie, origin: null, body: { name: "X" } })).status).toBe(403);
  });

  it("requires JSON bodies", async () => {
    const u = await newUser("ct");
    const r = await call(orgsRoute.POST, "POST", {}, { cookie: u.cookie, body: "name=X", contentType: "application/x-www-form-urlencoded" });
    expect(r.status).toBe(415);
  });
});

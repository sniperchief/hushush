// Passkey accounts and sessions, through the real route handlers and the real
// WebAuthn verifier (@simplewebauthn/server). The authenticator is a SOFTWARE
// one (scripts/lib/soft-passkey.mjs) that produces genuine signed responses.
import { describe, expect, it } from "vitest";
import * as regOptions from "@/app/api/auth/passkey/register/options/route";
import * as regVerify from "@/app/api/auth/passkey/register/verify/route";
import * as loginOptions from "@/app/api/auth/passkey/login/options/route";
import * as logoutRoute from "@/app/api/auth/logout/route";
import * as companyRoute from "@/app/api/company/route";
import * as streamsRoute from "@/app/api/streams/route";
import { getSessionUser } from "@/lib/auth";
import { db } from "@/lib/db";
import { authenticate, call, newPasskey, register, signIn, signUp } from "./support/helpers";

const tokenOf = (cookie: string) => cookie.split("=")[1];

describe("passkey sign-up", () => {
  it("creates an employer or worker account and signs it in; no password or e-mail", async () => {
    for (const kind of ["employer", "worker"] as const) {
      const { verify } = await register(kind, `New ${kind}`);
      expect(verify.status).toBe(201);
      expect(verify.body).toEqual({ kind });
      const cookie = verify.res.headers.getSetCookie().find((c) => c.startsWith("hh_session="))!;
      expect(cookie).toMatch(/HttpOnly/i);
      expect(cookie).toMatch(/SameSite=lax/i);
      const u = await getSessionUser(cookie.split(";")[0]!.split("=")[1]);
      expect(u).toMatchObject({ kind, displayName: `New ${kind}` });
      // The payroll user has only a reserved, undeliverable placeholder e-mail.
      const { rows } = await db().query("SELECT email FROM users WHERE id = $1", [u!.id]);
      expect(rows[0].email).toMatch(/^u-[0-9a-f-]{36}@users\.hushush\.invalid$/);
    }
  });

  it("asks the authenticator for a discoverable, user-verified passkey", async () => {
    const o = await call(regOptions.POST, "POST", {}, { body: { kind: "worker", displayName: "W" } });
    expect(o.body).toMatchObject({
      rp: { id: "localhost", name: "hushush" },
      attestation: "none",
      authenticatorSelection: { residentKey: "required", userVerification: "required" },
    });
    const cookie = o.res.headers.getSetCookie().find((c) => c.startsWith("hh_webauthn="))!;
    expect(cookie).toMatch(/HttpOnly/i);
    expect(cookie).toMatch(/SameSite=strict/i);
    expect(cookie).toMatch(/Max-Age=300/i);
  });

  it("stores only the public key and hashes of challenge handles", async () => {
    const { options, passkey } = await register("worker", "Hash Check");
    const handle = options.cookies.hh_webauthn!;
    const { rows: ch } = await db().query("SELECT handle_hash FROM webauthn_challenges WHERE handle_hash = encode(sha256($1::bytea), 'hex')", [handle]);
    expect(ch).toHaveLength(1);
    const { rows: raw } = await db().query("SELECT 1 FROM webauthn_challenges WHERE handle_hash = $1", [handle]);
    expect(raw).toHaveLength(0);
    const { rows: pk } = await db().query("SELECT public_key, counter FROM passkeys WHERE credential_id = $1", [passkey.id]);
    expect(pk[0].public_key.length).toBeGreaterThan(60); // COSE public key, no secret
  });

  it("rejects a response for another origin, a wrong challenge, or without user verification", async () => {
    const before = Number((await db().query("SELECT count(*) FROM accounts")).rows[0].count);
    const wrongOrigin = await register("worker", "X", newPasskey(), (r) => {
      const cd = JSON.parse(Buffer.from(String((r.response as Record<string, string>).clientDataJSON), "base64url").toString());
      cd.origin = "https://evil.example";
      (r.response as Record<string, string>).clientDataJSON = Buffer.from(JSON.stringify(cd)).toString("base64url");
    });
    expect(wrongOrigin.verify.status).toBe(401);
    const wrongChallenge = await register("worker", "X", newPasskey(), (r) => {
      const cd = JSON.parse(Buffer.from(String((r.response as Record<string, string>).clientDataJSON), "base64url").toString());
      cd.challenge = Buffer.from("not the challenge").toString("base64url");
      (r.response as Record<string, string>).clientDataJSON = Buffer.from(JSON.stringify(cd)).toString("base64url");
    });
    expect(wrongChallenge.verify.status).toBe(401);
    const noUv = newPasskey();
    noUv.userVerified = false;
    expect((await register("worker", "X", noUv)).verify.status).toBe(401);
    const after = Number((await db().query("SELECT count(*) FROM accounts")).rows[0].count);
    expect(after).toBe(before);
  });

  it("challenges are single-use and need their cookie", async () => {
    const pk = newPasskey();
    const o = await call(regOptions.POST, "POST", {}, { body: { kind: "worker", displayName: "Once" } });
    const response = pk.register(o.body);
    const cookie = `hh_webauthn=${o.cookies.hh_webauthn}`;
    expect((await call(regVerify.POST, "POST", {}, { body: { response } })).status).toBe(401); // no cookie
    // The cookie-less attempt did not consume it; the real one succeeds once.
    expect((await call(regVerify.POST, "POST", {}, { body: { response }, cookie })).status).toBe(201);
    expect((await call(regVerify.POST, "POST", {}, { body: { response }, cookie })).status).toBe(401);
  });

  it("one passkey belongs to one account", async () => {
    const pk = newPasskey();
    expect((await register("worker", "First", pk)).verify.status).toBe(201);
    const again = await register("employer", "Second", pk);
    expect(again.verify.status).toBe(409);
    expect(again.verify.body.error).toBe("passkey_exists");
  });

  it("validates the name and account kind", async () => {
    expect((await call(regOptions.POST, "POST", {}, { body: { kind: "worker", displayName: "  " } })).status).toBe(422);
    expect((await call(regOptions.POST, "POST", {}, { body: { kind: "admin", displayName: "A" } })).status).toBe(422);
    expect((await call(regOptions.POST, "POST", {}, { body: { kind: "worker", displayName: "x".repeat(101) } })).status).toBe(422);
  });
});

describe("passkey sign-in", () => {
  it("signs in with the registered passkey and advances the counter", async () => {
    const u = await signUp("worker", "Returning");
    const again = await signIn(u);
    expect(again.cookie).not.toBe(u.cookie);
    expect(await getSessionUser(tokenOf(again.cookie))).toMatchObject({ id: u.id, kind: "worker" });
    const { rows } = await db().query("SELECT counter, last_used_at FROM passkeys WHERE credential_id = $1", [u.passkey.id]);
    expect(Number(rows[0].counter)).toBe(1);
    expect(rows[0].last_used_at).toBeTruthy();
  });

  it("rejects unknown passkeys, other origins and replayed or cloned counters", async () => {
    expect((await authenticate(newPasskey())).status).toBe(401);
    const u = await signUp("employer", "Careful");
    expect((await authenticate(u.passkey, { origin: "https://evil.example" })).status).toBe(401);
    expect((await authenticate(u.passkey)).status).toBe(200); // counter 2 (the bad origin attempt used 1)
    expect((await authenticate(u.passkey, { counter: 2 })).status).toBe(401); // not increasing
    expect((await authenticate(u.passkey, { counter: 1 })).status).toBe(401);
    expect((await authenticate(u.passkey, { counter: 10 })).status).toBe(200);
  });

  it("a login challenge can't finish a sign-up and vice versa", async () => {
    const o = await call(loginOptions.POST, "POST", {});
    const response = newPasskey().register({ ...o.body, user: { id: "x", name: "x", displayName: "x" } });
    const r = await call(regVerify.POST, "POST", {}, { body: { response }, cookie: `hh_webauthn=${o.cookies.hh_webauthn}` });
    expect(r.status).toBe(401);
  });
});

describe("sessions", () => {
  it("logout invalidates the session server-side", async () => {
    const u = await signUp("worker", "Leaving");
    expect(await getSessionUser(tokenOf(u.cookie))).toBeTruthy();
    expect((await call(logoutRoute.POST, "POST", {}, { cookie: u.cookie })).status).toBe(200);
    expect(await getSessionUser(tokenOf(u.cookie))).toBeNull();
  });

  it("expires sessions after the absolute lifetime and after idling", async () => {
    const a = await signUp("worker", "Absolute");
    await db().query("UPDATE sessions SET created_at = now() - interval '31 days', expires_at = now() - interval '1 second' WHERE token_hash = encode(sha256($1::bytea), 'hex')", [tokenOf(a.cookie)]);
    expect(await getSessionUser(tokenOf(a.cookie))).toBeNull();
    const b = await signUp("worker", "Idle");
    await db().query("UPDATE sessions SET last_seen_at = now() - interval '8 days' WHERE token_hash = encode(sha256($1::bytea), 'hex')", [tokenOf(b.cookie)]);
    expect(await getSessionUser(tokenOf(b.cookie))).toBeNull();
  });

  it("rejects forged and missing session cookies", async () => {
    expect(await getSessionUser("forged-token")).toBeNull();
    expect(await getSessionUser(undefined)).toBeNull();
    expect((await call(companyRoute.POST, "POST", {}, { body: { name: "X" } })).status).toBe(401);
    expect((await call(companyRoute.POST, "POST", {}, { cookie: "hh_session=forged", body: { name: "X" } })).status).toBe(401);
  });
});

describe("account kinds never share screens or powers", () => {
  it("workers can't create companies or streams; employers can't own two companies", async () => {
    const w = await signUp("worker", "Just Working");
    const r = await call(companyRoute.POST, "POST", {}, { cookie: w.cookie, body: { name: "Sneaky" } });
    expect(r.status).toBe(403);
    expect(r.body.error).toBe("wrong_account");
    const s = await call(streamsRoute.POST, "POST", {}, { cookie: w.cookie, body: { workerName: "x", totalZec: "1", startAt: null, endAt: 9_999_999_999 } });
    expect(s.status).toBe(403);

    const e = await signUp("employer", "Boss");
    expect((await call(companyRoute.POST, "POST", {}, { cookie: e.cookie, body: { name: "One" } })).status).toBe(201);
    const two = await call(companyRoute.POST, "POST", {}, { cookie: e.cookie, body: { name: "Two" } });
    expect(two.status).toBe(409);
  });
});

describe("CSRF and request hygiene", () => {
  it("blocks cross-site and origin-less state changes", async () => {
    const u = await signUp("employer", "Origin");
    for (const origin of ["https://evil.example", null]) {
      const r = await call(companyRoute.POST, "POST", {}, { cookie: u.cookie, body: { name: "X" }, origin });
      expect(r.status).toBe(403);
      expect(r.body.error).toBe("bad_origin");
      const o = await call(regOptions.POST, "POST", {}, { body: { kind: "worker", displayName: "W" }, origin });
      expect(o.status).toBe(403);
    }
  });

  it("requires JSON bodies", async () => {
    const r = await call(regOptions.POST, "POST", {}, { body: "kind=worker&displayName=W", contentType: "application/x-www-form-urlencoded" });
    expect(r.status).toBe(415);
  });
});

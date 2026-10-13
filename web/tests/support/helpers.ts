import { randomUUID } from "node:crypto";
import { NextRequest } from "next/server";
import { expect } from "vitest";
import { db } from "@/lib/db";

import * as signupRoute from "@/app/api/auth/signup/route";
import * as loginRoute from "@/app/api/auth/login/route";
import * as verifyRoute from "@/app/api/auth/verify/route";
import * as orgsRoute from "@/app/api/orgs/route";
import * as inviteRoute from "@/app/api/orgs/[orgId]/invitations/route";
import * as acceptRoute from "@/app/api/invitations/accept/route";
import * as streamsRoute from "@/app/api/orgs/[orgId]/streams/route";
import * as claimsRoute from "@/app/api/streams/[streamId]/claims/route";
import * as lifecycleRoute from "@/app/api/streams/[streamId]/lifecycle/[action]/route";

export const ORIGIN = "http://localhost:3000";

type Handler<P> = (req: NextRequest, ctx: { params: Promise<P> }) => Promise<Response>;

export interface CallOpts {
  body?: unknown;
  cookie?: string;
  origin?: string | null;
  contentType?: string;
}

/** Invokes a route handler exactly as Next.js would. */
export async function call<P extends Record<string, string>>(
  handler: Handler<P>,
  method: string,
  params: P,
  opts: CallOpts = {},
): Promise<{ status: number; body: Record<string, unknown>; res: Response }> {
  const headers = new Headers();
  if (opts.origin !== null) headers.set("origin", opts.origin ?? ORIGIN);
  if (opts.cookie) headers.set("cookie", opts.cookie);
  if (opts.body !== undefined) headers.set("content-type", opts.contentType ?? "application/json");
  const req = new NextRequest(`${ORIGIN}/api/test`, {
    method,
    headers,
    body: opts.body === undefined ? undefined : typeof opts.body === "string" ? opts.body : JSON.stringify(opts.body),
  });
  const res = await handler(req, { params: Promise.resolve(params) });
  const text = await res.text();
  return { status: res.status, body: text ? JSON.parse(text) : {}, res };
}

export const uniqueEmail = (tag: string) => `${tag}-${randomUUID().slice(0, 8)}@example.test`;

/** Latest dev-outbox mail to `to`. */
export async function lastMail(to: string): Promise<{ subject: string; body: string }> {
  const { rows } = await db().query("SELECT subject, body FROM dev_outbox WHERE to_address = $1 ORDER BY id DESC LIMIT 1", [to]);
  expect(rows[0], `no mail to ${to}`).toBeTruthy();
  return rows[0];
}

export function tokenFrom(body: string, kind: "verify" | "invite"): string {
  const m = kind === "verify" ? body.match(/\/verify\?token=([A-Za-z0-9_-]+)/) : body.match(/\/invite\/([A-Za-z0-9_-]+)/);
  expect(m, `no ${kind} link in mail`).toBeTruthy();
  return m![1]!;
}

export interface TestUser {
  id: string;
  email: string;
  password: string;
  cookie: string;
}

export const PASSWORD = "correct horse battery staple";

/** Signs up, verifies via the emailed link, and signs in. */
export async function newUser(tag: string, name = tag): Promise<TestUser> {
  const email = uniqueEmail(tag);
  const s = await call(signupRoute.POST, "POST", {}, { body: { email, displayName: name, password: PASSWORD } });
  expect(s.status).toBe(202);
  const token = tokenFrom((await lastMail(email)).body, "verify");
  expect((await call(verifyRoute.POST, "POST", {}, { body: { token } })).status).toBe(200);
  return signIn(email);
}

export async function signIn(email: string, password = PASSWORD): Promise<TestUser> {
  const r = await call(loginRoute.POST, "POST", {}, { body: { email, password } });
  expect(r.status, JSON.stringify(r.body)).toBe(200);
  const setCookie = r.res.headers.get("set-cookie") ?? "";
  const cookie = setCookie.split(";")[0]!;
  const { rows } = await db().query("SELECT id FROM users WHERE lower(email) = lower($1)", [email]);
  return { id: rows[0].id, email, password, cookie };
}

export async function createOrg(owner: TestUser, name = "Org"): Promise<string> {
  const r = await call(orgsRoute.POST, "POST", {}, { cookie: owner.cookie, body: { name } });
  expect(r.status, JSON.stringify(r.body)).toBe(201);
  return r.body.id as string;
}

/** Invites `user` into `orgId` as `role` and accepts. */
export async function join(inviter: TestUser, orgId: string, user: TestUser, role: "member" | "admin" = "member"): Promise<void> {
  const inv = await call(inviteRoute.POST, "POST", { orgId }, { cookie: inviter.cookie, body: { email: user.email, role } });
  expect(inv.status, JSON.stringify(inv.body)).toBe(201);
  const token = tokenFrom((await lastMail(user.email)).body, "invite");
  const acc = await call(acceptRoute.POST, "POST", {}, { cookie: user.cookie, body: { token } });
  expect(acc.status, JSON.stringify(acc.body)).toBe(200);
}

export interface StreamBody {
  id: string;
  status: string;
  balance: { earned_zats: number; reserved_zats: number; claimed_zats: number; available_zats: number; available_zec: string; earned_zec: string };
}

/** 1 ZEC per second (3600 ZEC/hour) so accrual is observable within a test. */
export async function fastStream(manager: TestUser, orgId: string, recipient: TestUser, extra: Record<string, unknown> = {}): Promise<StreamBody> {
  const r = await call(streamsRoute.POST, "POST", { orgId }, {
    cookie: manager.cookie,
    body: { recipientUserId: recipient.id, rateZec: "3600", ratePeriodSecs: 3600, startAt: null, endAt: null, ...extra },
  });
  expect(r.status, JSON.stringify(r.body)).toBe(201);
  return r.body as unknown as StreamBody;
}

export const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function lifecycle(user: TestUser, streamId: string, action: "pause" | "resume" | "end") {
  return call(lifecycleRoute.POST, "POST", { streamId, action }, { cookie: user.cookie });
}

export async function claim(user: TestUser, streamId: string, amountZec: string, idempotencyKey: string = randomUUID()) {
  return call(claimsRoute.POST, "POST", { streamId }, { cookie: user.cookie, body: { amountZec, idempotencyKey } });
}

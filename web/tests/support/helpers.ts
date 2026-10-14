import { randomUUID } from "node:crypto";
import { NextRequest } from "next/server";
import { expect } from "vitest";
import { payroll, type FlowStream } from "@/lib/payroll";
// SOFTWARE authenticator: real WebAuthn responses for the real verifier.
import { SoftPasskey } from "../../scripts/lib/soft-passkey.mjs";

import * as regOptions from "@/app/api/auth/passkey/register/options/route";
import * as regVerify from "@/app/api/auth/passkey/register/verify/route";
import * as loginOptions from "@/app/api/auth/passkey/login/options/route";
import * as loginVerify from "@/app/api/auth/passkey/login/verify/route";
import * as companyRoute from "@/app/api/company/route";
import * as streamsRoute from "@/app/api/streams/route";
import * as joinRoute from "@/app/api/join/[token]/route";
import * as addressRoute from "@/app/api/streams/[streamId]/address/route";
import * as withdrawRoute from "@/app/api/streams/[streamId]/withdraw/route";

export { SoftPasskey };
export const ORIGIN = "http://localhost:3000";
export const RP_ID = "localhost";

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
): Promise<{ status: number; body: Record<string, unknown>; res: Response; cookies: Record<string, string> }> {
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
  const cookies: Record<string, string> = {};
  for (const c of res.headers.getSetCookie()) {
    const [pair] = c.split(";");
    const i = pair!.indexOf("=");
    cookies[pair!.slice(0, i)] = pair!.slice(i + 1);
  }
  return { status: res.status, body: text ? JSON.parse(text) : {}, res, cookies };
}

export type Kind = "employer" | "worker";

export interface TestUser {
  id: string;
  kind: Kind;
  name: string;
  cookie: string;
  passkey: SoftPasskey;
}

export const newPasskey = () => new SoftPasskey({ origin: ORIGIN, rpId: RP_ID });

/** Runs the registration ceremony; returns the raw verify result. */
export async function register(kind: Kind, name: string, passkey = newPasskey(), tamper?: (r: Record<string, unknown>) => void) {
  const o = await call(regOptions.POST, "POST", {}, { body: { kind, displayName: name } });
  expect(o.status, JSON.stringify(o.body)).toBe(200);
  const response = passkey.register(o.body) as Record<string, unknown>;
  tamper?.(response);
  const v = await call(regVerify.POST, "POST", {}, { body: { response }, cookie: `hh_webauthn=${o.cookies.hh_webauthn}` });
  return { options: o, verify: v, passkey };
}

/** Runs the sign-in ceremony with `passkey`; returns the raw verify result. */
export async function authenticate(passkey: SoftPasskey, opts: { origin?: string; counter?: number } = {}) {
  const o = await call(loginOptions.POST, "POST", {});
  expect(o.status).toBe(200);
  const response = passkey.authenticate(o.body, opts);
  return call(loginVerify.POST, "POST", {}, { body: { response }, cookie: `hh_webauthn=${o.cookies.hh_webauthn}` });
}

async function userOf(cookie: string): Promise<string> {
  const { getSessionUser } = await import("@/lib/auth");
  const u = await getSessionUser(cookie.split("=")[1]);
  expect(u).toBeTruthy();
  return u!.id;
}

export async function signUp(kind: Kind, name: string = kind): Promise<TestUser> {
  const { verify, passkey } = await register(kind, name);
  expect(verify.status, JSON.stringify(verify.body)).toBe(201);
  const cookie = `hh_session=${verify.cookies.hh_session}`;
  return { id: await userOf(cookie), kind, name, cookie, passkey };
}

export async function signIn(user: TestUser): Promise<TestUser> {
  const r = await authenticate(user.passkey);
  expect(r.status, JSON.stringify(r.body)).toBe(200);
  return { ...user, cookie: `hh_session=${r.cookies.hh_session}` };
}

/** An employer account with its company. */
export async function newEmployer(name = "Employer", company = `${name} Co`): Promise<TestUser & { companyId: string }> {
  const u = await signUp("employer", name);
  const r = await call(companyRoute.POST, "POST", {}, { cookie: u.cookie, body: { name: company } });
  expect(r.status, JSON.stringify(r.body)).toBe(201);
  return { ...u, companyId: r.body.id as string };
}

export const newWorker = (name = "Worker") => signUp("worker", name);

export interface Created {
  offerId: string;
  link: string;
  token: string;
}

/** Creates a stream offer: `totalZec` over `seconds`, starting now (or `startAt`). */
export async function createStream(employer: TestUser, totalZec = "0.001", seconds = 10, extra: { startAt?: number; workerName?: string } = {}): Promise<Created> {
  const now = Math.floor(Date.now() / 1000);
  const startAt = extra.startAt ?? null;
  const r = await call(streamsRoute.POST, "POST", {}, {
    cookie: employer.cookie,
    body: { workerName: extra.workerName ?? "Wendy", totalZec, startAt, endAt: (startAt ?? now) + seconds },
  });
  expect(r.status, JSON.stringify(r.body)).toBe(201);
  const link = r.body.link as string;
  return { offerId: (r.body.offer as { id: string }).id, link, token: link.split("/join/")[1]! };
}

export async function join(worker: TestUser, token: string) {
  return call(joinRoute.POST, "POST", { token }, { cookie: worker.cookie });
}

/** Creates a stream and has `worker` join it; returns the stream id. */
export async function boundStream(employer: TestUser, worker: TestUser, totalZec = "0.001", seconds = 10): Promise<string> {
  const c = await createStream(employer, totalZec, seconds);
  const j = await join(worker, c.token);
  expect(j.status, JSON.stringify(j.body)).toBe(201);
  return j.body.streamId as string;
}

export const setAddress = (u: TestUser, streamId: string, address: string) =>
  call(addressRoute.POST, "POST", { streamId }, { cookie: u.cookie, body: { address } });

export const withdraw = (u: TestUser, streamId: string, idempotencyKey: string = randomUUID(), opts: { origin?: string | null } = {}) =>
  call(withdrawRoute.POST, "POST", { streamId }, { cookie: u.cookie, body: { idempotencyKey }, ...opts });

export const flow = (u: TestUser, streamId: string): Promise<FlowStream> => payroll.flowStream(u.id, streamId);

export const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Controls of the MOCK wallet (tests/support/mock-wallet.ts). */
export const mock = async (path: string, body: unknown = {}) =>
  (await fetch(`${process.env.MOCK_WALLET_URL}/__mock/${path}`, { method: "POST", body: JSON.stringify(body) })).json() as Promise<{
    txs_built: number;
    submits: number;
    tip: number;
  }>;

export async function waitFor<T>(get: () => Promise<T>, ok: (v: T) => boolean, ms = 15_000): Promise<T> {
  const until = Date.now() + ms;
  for (;;) {
    const v = await get();
    if (ok(v)) return v;
    if (Date.now() > until) throw new Error(`timed out; last=${JSON.stringify(v)}`);
    await sleep(250);
  }
}

import "server-only";
import { NextResponse, type NextRequest } from "next/server";
import { ZodError, type z } from "zod";
import { getSessionUser, sessionCookieName, type SessionUser } from "./auth";
import { env } from "./env";
import { AppError, unauthorized } from "./errors";
import { PayrollApiError } from "./payroll";

export function json(data: unknown, status = 200): NextResponse {
  return NextResponse.json(data, { status, headers: { "cache-control": "no-store" } });
}

function errorResponse(e: unknown): NextResponse {
  if (e instanceof AppError) return json({ error: e.code, message: e.message }, e.status);
  if (e instanceof PayrollApiError) {
    // payroll-api messages are written for end users; internal errors are opaque.
    const status = e.status >= 500 ? 503 : e.status;
    return json({ error: e.code, message: e.message }, status);
  }
  if (e instanceof ZodError) {
    return json({ error: "validation", message: e.issues[0]?.message ?? "Invalid input." }, 422);
  }
  console.error(e);
  return json({ error: "internal", message: "Something went wrong." }, 500);
}

/**
 * CSRF defence for state-changing requests: the browser-supplied Origin must
 * be this app's origin, and the body must be JSON (which a cross-site form
 * cannot send without a CORS preflight). Session cookies are also SameSite=Lax.
 */
export function assertSameOrigin(req: NextRequest): void {
  const origin = req.headers.get("origin");
  if (!origin || origin !== new URL(env.appUrl).origin) {
    throw new AppError(403, "bad_origin", "Cross-site request blocked.");
  }
}

export async function readJson<S extends z.ZodType>(req: NextRequest, schema: S): Promise<z.output<S>> {
  if (!req.headers.get("content-type")?.toLowerCase().startsWith("application/json")) {
    throw new AppError(415, "unsupported_media_type", "Expected a JSON request body.");
  }
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    throw new AppError(400, "bad_json", "Malformed JSON.");
  }
  return schema.parse(body);
}

export function sessionToken(req: NextRequest): string | undefined {
  return req.cookies.get(sessionCookieName())?.value;
}

export async function requireUser(req: NextRequest): Promise<SessionUser> {
  const user = await getSessionUser(sessionToken(req));
  if (!user) throw unauthorized();
  return user;
}

/** Wraps a state-changing route handler: origin check, error mapping. */
export function mutation<Ctx>(fn: (req: NextRequest, ctx: Ctx) => Promise<NextResponse>) {
  return async (req: NextRequest, ctx: Ctx): Promise<NextResponse> => {
    try {
      assertSameOrigin(req);
      return await fn(req, ctx);
    } catch (e) {
      return errorResponse(e);
    }
  };
}

export function setSessionCookie(res: NextResponse, token: string, expiresAt: Date): void {
  res.cookies.set(sessionCookieName(), token, {
    httpOnly: true,
    secure: env.appUrl.startsWith("https://"),
    sameSite: "lax",
    path: "/",
    expires: expiresAt,
  });
}

export function clearSessionCookie(res: NextResponse): void {
  res.cookies.set(sessionCookieName(), "", {
    httpOnly: true,
    secure: env.appUrl.startsWith("https://"),
    sameSite: "lax",
    path: "/",
    maxAge: 0,
  });
}

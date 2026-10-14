import type { AuthenticationResponseJSON } from "@simplewebauthn/server";
import { z } from "zod";
import { clearCookie, json, mutation, readJson, setSessionCookie } from "@/lib/http";
import { ceremonyCookieName, verifyAuthentication } from "@/lib/passkeys";

const schema = z.object({ response: z.custom<AuthenticationResponseJSON>((v) => typeof v === "object" && v !== null) });

/** Finishes a passkey sign-in. */
export const POST = mutation(async (req) => {
  const { response } = await readJson(req, schema);
  const r = await verifyAuthentication(req.cookies.get(ceremonyCookieName())?.value, response);
  const res = json({ kind: r.kind });
  clearCookie(res, ceremonyCookieName());
  setSessionCookie(res, r.session.token, r.session.expiresAt);
  return res;
});

import type { RegistrationResponseJSON } from "@simplewebauthn/server";
import { z } from "zod";
import { clearCookie, json, mutation, readJson, setSessionCookie } from "@/lib/http";
import { ceremonyCookieName, verifyRegistration } from "@/lib/passkeys";

const schema = z.object({ response: z.custom<RegistrationResponseJSON>((v) => typeof v === "object" && v !== null) });

/** Finishes a passkey sign-up: creates the account and signs it in. */
export const POST = mutation(async (req) => {
  const { response } = await readJson(req, schema);
  const r = await verifyRegistration(req.cookies.get(ceremonyCookieName())?.value, response);
  const res = json({ ok: true }, 201);
  clearCookie(res, ceremonyCookieName());
  setSessionCookie(res, r.session.token, r.session.expiresAt);
  return res;
});

import { z } from "zod";
import { displayNameSchema, kindSchema } from "@/lib/auth";
import { json, mutation, readJson, setCeremonyCookie } from "@/lib/http";
import { CEREMONY_COOKIE_MAX_AGE, ceremonyCookieName, registrationOptions } from "@/lib/passkeys";

const schema = z.object({ kind: kindSchema, displayName: displayNameSchema });

/** Starts a passkey sign-up (a new employer or worker account). */
export const POST = mutation(async (req) => {
  const { kind, displayName } = await readJson(req, schema);
  const { options, handle } = await registrationOptions(kind, displayName);
  const res = json(options);
  setCeremonyCookie(res, ceremonyCookieName(), handle, CEREMONY_COOKIE_MAX_AGE);
  return res;
});

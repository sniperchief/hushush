import { z } from "zod";
import { displayNameSchema } from "@/lib/auth";
import { json, mutation, readJson, setCeremonyCookie } from "@/lib/http";
import { CEREMONY_COOKIE_MAX_AGE, ceremonyCookieName, registrationOptions } from "@/lib/passkeys";

const schema = z.object({ displayName: displayNameSchema });

/** Starts a passkey sign-up (a new account). */
export const POST = mutation(async (req) => {
  const { displayName } = await readJson(req, schema);
  const { options, handle } = await registrationOptions(displayName);
  const res = json(options);
  setCeremonyCookie(res, ceremonyCookieName(), handle, CEREMONY_COOKIE_MAX_AGE);
  return res;
});

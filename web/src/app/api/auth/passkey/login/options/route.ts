import { json, mutation, setCeremonyCookie } from "@/lib/http";
import { authenticationOptions, CEREMONY_COOKIE_MAX_AGE, ceremonyCookieName } from "@/lib/passkeys";

/** Starts a passkey sign-in. */
export const POST = mutation(async () => {
  const { options, handle } = await authenticationOptions();
  const res = json(options);
  setCeremonyCookie(res, ceremonyCookieName(), handle, CEREMONY_COOKIE_MAX_AGE);
  return res;
});

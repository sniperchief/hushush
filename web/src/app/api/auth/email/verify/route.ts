import { z } from "zod";
import { consumeLink } from "@/lib/email-login";
import { json, mutation, readJson, setSessionCookie } from "@/lib/http";

const schema = z.object({ token: z.string().min(1).max(200) });

/** Uses an e-mailed link: signs in (creating the account for a sign-up link), or confirms a backup address. */
export const POST = mutation(async (req) => {
  const { token } = await readJson(req, schema);
  const r = await consumeLink(token);
  const res = json({ purpose: r.purpose });
  if (r.purpose !== "link") setSessionCookie(res, r.session.token, r.session.expiresAt);
  return res;
});

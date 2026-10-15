import { z } from "zod";
import { emailSchema, requestLink } from "@/lib/email-login";
import { json, mutation, readJson, requireUser } from "@/lib/http";

const schema = z.object({ email: emailSchema });

/** Signed-in employer: sends a confirmation link to set the account's sign-in e-mail. */
export const POST = mutation(async (req) => {
  const user = await requireUser(req);
  const { email } = await readJson(req, schema);
  await requestLink(user, email);
  return json({ ok: true }, 202);
});

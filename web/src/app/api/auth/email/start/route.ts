import { z } from "zod";
import { displayNameSchema } from "@/lib/auth";
import { emailSchema, requestLogin, requestSignup } from "@/lib/email-login";
import { json, mutation, readJson } from "@/lib/http";

const schema = z.discriminatedUnion("mode", [
  z.object({ mode: z.literal("login"), email: emailSchema }),
  z.object({ mode: z.literal("signup"), email: emailSchema, displayName: displayNameSchema }),
]);

/**
 * E-mails an employer sign-in or sign-up link. The reply is the same whether
 * or not the address has an account, so it can't be used to look one up.
 */
export const POST = mutation(async (req) => {
  const body = await readJson(req, schema);
  if (body.mode === "login") await requestLogin(body.email);
  else await requestSignup(body.email, body.displayName);
  return json({ ok: true }, 202);
});

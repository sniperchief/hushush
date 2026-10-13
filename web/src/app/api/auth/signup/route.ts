import { signup, signupSchema } from "@/lib/auth";
import { json, mutation, readJson } from "@/lib/http";

export const POST = mutation(async (req) => {
  await signup(await readJson(req, signupSchema));
  // Same response whether or not the email was already registered.
  return json({ ok: true, message: "Check your email for a verification link." }, 202);
});

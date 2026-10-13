import { z } from "zod";
import { emailSchema, resendVerification } from "@/lib/auth";
import { json, mutation, readJson } from "@/lib/http";

export const POST = mutation(async (req) => {
  const { email } = await readJson(req, z.object({ email: emailSchema }));
  await resendVerification(email);
  return json({ ok: true, message: "If that account needs verification, a new link is on its way." }, 202);
});

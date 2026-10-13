import { login, loginSchema } from "@/lib/auth";
import { json, mutation, readJson, setSessionCookie } from "@/lib/http";

export const POST = mutation(async (req) => {
  const { token, expiresAt } = await login(await readJson(req, loginSchema));
  const res = json({ ok: true });
  setSessionCookie(res, token, expiresAt);
  return res;
});

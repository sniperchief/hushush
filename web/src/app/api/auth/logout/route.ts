import { logout } from "@/lib/auth";
import { clearSessionCookie, json, mutation, sessionToken } from "@/lib/http";

export const POST = mutation(async (req) => {
  await logout(sessionToken(req));
  const res = json({ ok: true });
  clearSessionCookie(res);
  return res;
});

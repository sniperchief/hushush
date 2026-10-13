import { z } from "zod";
import { verifyEmail } from "@/lib/auth";
import { AppError } from "@/lib/errors";
import { json, mutation, readJson } from "@/lib/http";

export const POST = mutation(async (req) => {
  const { token } = await readJson(req, z.object({ token: z.string().min(1).max(200) }));
  if (!(await verifyEmail(token))) {
    throw new AppError(400, "invalid_token", "This verification link is invalid, expired or already used.");
  }
  return json({ ok: true });
});

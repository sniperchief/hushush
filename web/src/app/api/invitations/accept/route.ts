import { z } from "zod";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { acceptInvitation } from "@/lib/invitations";

export const POST = mutation(async (req) => {
  const user = await requireUser(req);
  const { token } = await readJson(req, z.object({ token: z.string().min(1).max(200) }));
  return json(await acceptInvitation(user, token));
});

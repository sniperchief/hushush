import { json, mutation, readJson, requireUser } from "@/lib/http";
import { createInvitation, inviteSchema } from "@/lib/invitations";

type Ctx = { params: Promise<{ orgId: string }> };

export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { orgId } = await params;
  return json(await createInvitation(user, orgId, await readJson(req, inviteSchema)), 201);
});

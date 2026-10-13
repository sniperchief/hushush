import { json, mutation, requireUser } from "@/lib/http";
import { revokeInvitation } from "@/lib/invitations";

type Ctx = { params: Promise<{ orgId: string; invitationId: string }> };

export const DELETE = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { orgId, invitationId } = await params;
  await revokeInvitation(user.id, orgId, invitationId);
  return json({ ok: true });
});

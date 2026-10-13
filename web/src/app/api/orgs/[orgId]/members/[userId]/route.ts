import { z } from "zod";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ orgId: string; userId: string }> };

export const PATCH = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { orgId, userId } = await params;
  const { role } = await readJson(req, z.object({ role: z.enum(["owner", "admin", "member"]) }));
  return json(await payroll.setMemberRole(user.id, orgId, userId, role));
});

export const DELETE = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { orgId, userId } = await params;
  return json(await payroll.removeMember(user.id, orgId, userId));
});

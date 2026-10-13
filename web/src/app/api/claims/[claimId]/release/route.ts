import { json, mutation, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ claimId: string }> };

export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { claimId } = await params;
  return json(await payroll.releaseClaim(user.id, claimId));
});

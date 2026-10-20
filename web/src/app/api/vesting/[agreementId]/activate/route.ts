import { json, mutation, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ agreementId: string }> };

/** Activates a draft. Only before its start date (payroll enforces it). */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { agreementId } = await params;
  return json(await payroll.activateVesting(user.id, agreementId));
});

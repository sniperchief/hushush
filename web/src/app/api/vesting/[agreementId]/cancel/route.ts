import { json, mutation, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ agreementId: string }> };

/** Cancels an agreement: vesting stops now; what has vested stays owed. */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { agreementId } = await params;
  return json(await payroll.cancelVesting(user.id, agreementId));
});

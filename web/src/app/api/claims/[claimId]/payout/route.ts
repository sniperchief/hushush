import { json, mutation, readJson, requireUser } from "@/lib/http";
import { addressSchema } from "@/lib/payout-input";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ claimId: string }> };

/**
 * The contributor's deliberate "pay out this claim" action. Idempotent per
 * claim: repeating it returns the existing payout and never pays twice.
 */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { claimId } = await params;
  const b = await readJson(req, addressSchema);
  const out = await payroll.requestPayout(user.id, claimId, b.address);
  return json(out, out.duplicate ? 200 : 201);
});

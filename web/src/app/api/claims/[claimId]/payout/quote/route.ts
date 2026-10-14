import { json, mutation, readJson, requireUser } from "@/lib/http";
import { addressSchema } from "@/lib/payout-input";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ claimId: string }> };

/** Validates the address and returns the real fee before the user confirms. */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { claimId } = await params;
  const b = await readJson(req, addressSchema);
  return json(await payroll.quotePayout(user.id, claimId, b.address));
});

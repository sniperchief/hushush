import { json, mutation, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ offerId: string }> };

/** Cancels a stream nobody has joined yet. */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { offerId } = await params;
  return json(await payroll.cancelOffer(user.id, offerId));
});

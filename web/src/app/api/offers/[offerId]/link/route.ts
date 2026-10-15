import { newWorkerLink } from "@/lib/company";
import { json, mutation, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ offerId: string }> };

/** Issues a new worker link for a stream nobody has joined; the old one stops working. */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { offerId } = await params;
  const link = newWorkerLink();
  const offer = await payroll.relinkOffer(user.id, offerId, link.linkHash);
  return json({ offer, link: link.url });
});

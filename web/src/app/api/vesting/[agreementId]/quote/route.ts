import { z } from "zod";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ agreementId: string }> };

const schema = z.object({ amountZec: z.string().trim().min(1, "Enter an amount.").max(32) });

/** The network fee for withdrawing `amountZec` now. Nothing is reserved. */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { agreementId } = await params;
  const { amountZec } = await readJson(req, schema);
  return json(await payroll.vestingQuote(user.id, agreementId, amountZec));
});

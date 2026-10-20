import { z } from "zod";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ agreementId: string }> };

const schema = z.object({
  amountZec: z.string().trim().min(1, "Enter an amount.").max(32),
  idempotencyKey: z.string().min(8).max(128).regex(/^[\x21-\x7e]+$/),
});

/**
 * Withdraws vested compensation to the locked address. Never pays twice: a
 * repeat (double tap, retry, refresh) returns the same withdrawal.
 */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { agreementId } = await params;
  const { amountZec, idempotencyKey } = await readJson(req, schema);
  const r = await payroll.withdrawVesting(user.id, agreementId, amountZec, idempotencyKey);
  return json(r, r.duplicate ? 200 : 201);
});

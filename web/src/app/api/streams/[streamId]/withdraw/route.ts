import { z } from "zod";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ streamId: string }> };

const schema = z.object({ idempotencyKey: z.string().min(8).max(128).regex(/^[\x21-\x7e]+$/) });

/**
 * "Withdraw": pays everything available to the locked address. Never pays
 * twice: a repeat (double tap, retry, refresh) returns the same withdrawal.
 */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { streamId } = await params;
  const { idempotencyKey } = await readJson(req, schema);
  const r = await payroll.withdraw(user.id, streamId, idempotencyKey);
  return json(r, r.duplicate ? 200 : 201);
});

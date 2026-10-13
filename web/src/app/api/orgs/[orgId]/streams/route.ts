import { z } from "zod";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";
import { RATE_PERIODS } from "@/lib/periods";

const unix = z.number().int().nonnegative().max(32503680000);

const schema = z.object({
  recipientUserId: z.string().uuid("Choose a contributor."),
  // Format check only; the payroll engine parses this exactly into zatoshis.
  rateZec: z.string().trim().regex(/^\d{1,8}(\.\d{1,8})?$/, "Enter a ZEC amount like 1.5 (max 8 decimals)."),
  ratePeriodSecs: z
    .number()
    .int()
    .refine((s) => RATE_PERIODS.some((p) => p.secs === s), "Choose a pay period."),
  startAt: unix.nullable(),
  endAt: unix.nullable(),
});

type Ctx = { params: Promise<{ orgId: string }> };

export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { orgId } = await params;
  const b = await readJson(req, schema);
  const stream = await payroll.createStream(user.id, orgId, {
    recipient_user_id: b.recipientUserId,
    rate_zec: b.rateZec,
    rate_period_secs: b.ratePeriodSecs,
    start_at: b.startAt,
    end_at: b.endAt,
  });
  return json(stream, 201);
});

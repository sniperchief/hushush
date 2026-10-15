import { json, mutation, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ streamId: string }> };

/** Ends a stream now. Earned wages stay withdrawable by the worker. */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { streamId } = await params;
  await payroll.endStream(user.id, streamId);
  return json(await payroll.flowStream(user.id, streamId));
});

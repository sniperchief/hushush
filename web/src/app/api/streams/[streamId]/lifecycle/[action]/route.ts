import { json, mutation, requireUser } from "@/lib/http";
import { notFound } from "@/lib/errors";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ streamId: string; action: string }> };

export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { streamId, action } = await params;
  if (action !== "pause" && action !== "resume" && action !== "end") throw notFound();
  return json(await payroll.lifecycle(user.id, streamId, action));
});

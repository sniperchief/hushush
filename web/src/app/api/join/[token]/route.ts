import { requireKind } from "@/lib/auth";
import { linkHashOf } from "@/lib/company";
import { notFound } from "@/lib/errors";
import { json, mutation, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ token: string }> };

/** Binds the stream behind a worker link to the signed-in worker (single use). */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  requireKind(user, "worker");
  const hash = linkHashOf((await params).token);
  if (!hash) throw notFound("This worker link is not valid.");
  const r = await payroll.bindLink(user.id, hash);
  return json({ streamId: r.stream_id, duplicate: r.duplicate }, r.duplicate ? 200 : 201);
});

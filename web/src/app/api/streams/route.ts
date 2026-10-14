import { z } from "zod";
import { requireKind } from "@/lib/auth";
import { employerCompany, newWorkerLink } from "@/lib/company";
import { AppError } from "@/lib/errors";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

const unix = z.number().int().positive();
const schema = z.object({
  workerName: z.string().trim().min(1, "Enter the worker's name.").max(100, "Use at most 100 characters."),
  totalZec: z.string().trim().min(1, "Enter the total in ZEC.").max(32),
  startAt: unix.nullable(),
  endAt: unix,
});

/**
 * Creates a stream for a worker who signs up later: returns its worker link
 * (shown once — only its hash is stored).
 */
export const POST = mutation(async (req) => {
  const user = await requireUser(req);
  requireKind(user, "employer");
  const body = await readJson(req, schema);
  const company = await employerCompany(user);
  if (!company) throw new AppError(409, "no_company", "Name your company first.");
  const link = newWorkerLink();
  const offer = await payroll.createOffer(user.id, company.id, {
    worker_name: body.workerName,
    total_zec: body.totalZec,
    start_at: body.startAt,
    end_at: body.endAt,
    link_hash: link.linkHash,
  });
  return json({ offer, link: link.url }, 201);
});

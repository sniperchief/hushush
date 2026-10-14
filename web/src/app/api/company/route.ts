import { z } from "zod";
import { requireKind } from "@/lib/auth";
import { employerCompany } from "@/lib/company";
import { AppError } from "@/lib/errors";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

const schema = z.object({ name: z.string().trim().min(1, "Enter your company's name.").max(100, "Use at most 100 characters.") });

/** Names the employer's company (once; one employer account = one company). */
export const POST = mutation(async (req) => {
  const user = await requireUser(req);
  requireKind(user, "employer");
  const { name } = await readJson(req, schema);
  if (await employerCompany(user)) throw new AppError(409, "company_exists", "Your account already has a company.");
  const { id } = await payroll.createOrg(user.id, name);
  return json({ id }, 201);
});

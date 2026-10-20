import { z } from "zod";
import { employerCompany } from "@/lib/company";
import { AppError } from "@/lib/errors";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

const DAY = 86_400;

const schema = z.object({
  employeeUserId: z.string().uuid("Choose an employee."),
  totalZec: z.string().trim().min(1, "Enter the total in ZEC.").max(32),
  startAt: z.number().int().positive(),
  durationDays: z.number().int().min(1, "The duration must be at least one day.").max(36_500, "The duration can be at most 100 years."),
  cliffDays: z.number().int().min(0).max(36_500).nullable(),
});

/**
 * Creates a draft vesting agreement for the signed-in employer's company.
 * The company comes from the session; payroll checks the employee is one of
 * its members and every term.
 */
export const POST = mutation(async (req) => {
  const user = await requireUser(req);
  const body = await readJson(req, schema);
  const company = await employerCompany(user);
  if (!company) throw new AppError(409, "no_company", "Name your company first.");
  const agreement = await payroll.createVesting(user.id, company.id, {
    employee_user_id: body.employeeUserId,
    total_zec: body.totalZec,
    start_at: body.startAt,
    duration_secs: body.durationDays * DAY,
    cliff_secs: body.cliffDays ? body.cliffDays * DAY : null,
  });
  return json({ agreement }, 201);
});

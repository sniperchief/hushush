import { z } from "zod";
import { employerCompany } from "@/lib/company";
import { AppError } from "@/lib/errors";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

const schema = z.object({ idempotencyKey: z.uuid() });

/** Testnet: "Get test TAZ" tops the company's payout wallet up from the shared treasury. */
export const POST = mutation(async (req) => {
  const user = await requireUser(req);
  const { idempotencyKey } = await readJson(req, schema);
  const company = await employerCompany(user);
  if (!company) throw new AppError(409, "no_company", "Name your company first.");
  const r = await payroll.faucet(user.id, company.id, idempotencyKey);
  return json(r, r.state === "failed" ? 200 : 201);
});

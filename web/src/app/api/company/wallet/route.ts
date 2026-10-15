import type { NextRequest } from "next/server";
import { employerCompany } from "@/lib/company";
import { AppError } from "@/lib/errors";
import { json, requireUser, toResponse } from "@/lib/http";
import { payroll } from "@/lib/payroll";

/** The company's own payout wallet: balance, funding address and faucet availability (read-only). */
export async function GET(req: NextRequest) {
  try {
    const user = await requireUser(req);
    const company = await employerCompany(user);
    if (!company) throw new AppError(409, "no_company", "Name your company first.");
    return json(await payroll.payoutWallet(user.id, company.id));
  } catch (e) {
    return toResponse(e);
  }
}

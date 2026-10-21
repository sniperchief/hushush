import "server-only";
import { cache } from "react";
import type { Company } from "./company";
import { PayrollApiError, payroll, type PayoutWalletSummary } from "./payroll";

// Per-request loaders shared by the dashboard frame (sidebar counts) and its
// pages, so one navigation asks payroll-api once for each. React's cache keys
// on the arguments, so they take ids, not objects.

/** The company this account owns (MVP: at most one), or null. */
export const companyOf = cache(async (userId: string): Promise<Company | null> => {
  const o = (await payroll.listOrgs(userId)).find((m) => m.role === "owner");
  return o ? { id: o.organization_id, name: o.organization_name } : null;
});

export const employerHomeOf = cache((userId: string, orgId: string) => payroll.employerHome(userId, orgId));
export const orgVestingOf = cache((userId: string, orgId: string) => payroll.orgVesting(userId, orgId));
export const workerStreamsOf = cache((userId: string) => payroll.workerStreams(userId));
export const myVestingOf = cache((userId: string) => payroll.myVesting(userId));

/** The company's payout wallet, or the reason it can't be shown. */
export const payoutWalletOf = cache(async (userId: string, orgId: string): Promise<PayoutWalletSummary | string> => {
  try {
    return await payroll.payoutWallet(userId, orgId);
  } catch (e) {
    if (e instanceof PayrollApiError && e.status < 500) return e.message;
    if (e instanceof PayrollApiError) return "The payout wallet is unavailable right now.";
    throw e;
  }
});

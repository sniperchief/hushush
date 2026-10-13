import "server-only";
import { notFound } from "next/navigation";
import { PayrollApiError } from "./payroll";

/** Maps payroll "not found / forbidden" to a 404 page (no existence leak). */
export async function orNotFound<T>(p: Promise<T>): Promise<T> {
  try {
    return await p;
  } catch (e) {
    if (e instanceof PayrollApiError && (e.status === 404 || e.status === 403)) notFound();
    throw e;
  }
}

import "server-only";
import type { SessionUser } from "./auth";
import { env } from "./env";
import { hashToken, newToken } from "./crypto";
import { payroll } from "./payroll";

export interface Company {
  id: string;
  name: string;
}

/** The company this account owns, if it pays people (MVP: at most one per account). */
export async function employerCompany(user: SessionUser): Promise<Company | null> {
  const o = (await payroll.listOrgs(user.id)).find((m) => m.role === "owner");
  return o ? { id: o.organization_id, name: o.organization_name } : null;
}

/**
 * A fresh worker link. Only the token's SHA-256 is stored (in payroll), so
 * the link can be shown once; the employer can replace it while unused.
 */
export function newWorkerLink(): { url: string; linkHash: string } {
  const token = newToken();
  return { url: `${env.appUrl}/join/${token}`, linkHash: hashToken(token) };
}

/** Link tokens are base64url of 32 bytes. */
export function linkHashOf(token: string): string | null {
  return /^[A-Za-z0-9_-]{43}$/.test(token) ? hashToken(token) : null;
}

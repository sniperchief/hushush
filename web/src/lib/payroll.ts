import "server-only";
import { env } from "./env";

// Client for the internal payroll-api (Rust). All payroll rules, accrual and
// money parsing/formatting happen there; this module only moves JSON.

export type Role = "owner" | "admin" | "member";
export type StreamStatus = "active" | "paused" | "ended";
export type ClaimStatus = "reserved" | "released" | "settled";
/** Only `confirmed` means paid. `submitted` = broadcast, awaiting confirmations. */
export type PayoutState = "reserved" | "submitted" | "confirmed" | "failed" | "requires_reconciliation";

export interface Membership {
  organization_id: string;
  organization_name: string;
  role: Role;
  joined_at: number;
}

export interface Member {
  user_id: string;
  email: string;
  display_name: string;
  role: Role;
  joined_at: number;
}

export interface BalanceView {
  as_of: number;
  earned_zats: number;
  earned_zec: string;
  reserved_zats: number;
  reserved_zec: string;
  claimed_zats: number;
  claimed_zec: string;
  available_zats: number;
  available_zec: string;
}

export interface StreamView {
  id: string;
  organization_id: string;
  organization_name: string;
  recipient_user_id: string;
  recipient_display_name: string;
  recipient_email: string;
  created_by: string;
  rate_zats: number;
  rate_zec: string;
  rate_period_secs: number;
  start_at: number;
  end_at: number | null;
  status: StreamStatus;
  ended_at: number | null;
  created_at: number;
  balance: BalanceView;
}

export interface StreamEvent {
  kind: "created" | "paused" | "resumed" | "ended";
  actor_user_id: string;
  occurred_at: number;
  banked_active_secs_after: number;
  earned_zats_after: number;
}

export interface PayoutView {
  id: string;
  claim_id: string;
  state: PayoutState;
  amount_zats: number;
  amount_zec: string;
  fee_estimate_zats: number;
  fee_zats: number | null;
  fee_zec: string | null;
  txid: string | null;
  mined_height: number | null;
  confirmations: number;
  required_confirmations: number;
  expiry_height: number | null;
  detail: string | null;
  network: "testnet";
  created_at: number;
  submitted_at: number | null;
  resolved_at: number | null;
  updated_at: number;
}

export interface ClaimView {
  id: string;
  stream_id: string;
  requested_by: string;
  amount_zats: number;
  amount_zec: string;
  status: ClaimStatus;
  created_at: number;
  resolved_at: number | null;
  settlement_ref: string | null;
  payout: PayoutView | null;
}

export interface StreamDetail {
  stream: StreamView;
  events: StreamEvent[];
  claims: ClaimView[];
  payouts_enabled: boolean;
}

export interface PayoutQuote {
  claim_id: string;
  amount_zats: number;
  amount_zec: string;
  fee_zats: number | null;
  fee_zec: string | null;
  wallet_debit_zats: number | null;
  valid_recipient: boolean;
  sufficient: boolean;
  reason: string | null;
  network: "testnet";
  required_confirmations: number;
}

export interface PayoutWalletSummary {
  network: "testnet";
  spendable_zats: number;
  spendable_zec: string;
  pending_zats: number;
  pending_zec: string;
  funding_address: string;
  chain_tip: number | null;
  last_sync_at: number | null;
  required_confirmations: number;
}

/** An error returned by payroll-api, with its HTTP status and error code. */
export class PayrollApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
    public readonly body: Record<string, unknown> = {},
  ) {
    super(message);
  }
}

async function call<T>(method: string, path: string, actor: string | null, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {
    authorization: `Bearer ${env.payrollApiToken}`,
    accept: "application/json",
  };
  if (actor) headers["x-actor-id"] = actor;
  if (body !== undefined) headers["content-type"] = "application/json";
  let res: Response;
  try {
    res = await fetch(`${env.payrollApiUrl}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      cache: "no-store",
    });
  } catch {
    throw new PayrollApiError(503, "payroll_unavailable", "The payroll service is unavailable. Try again shortly.");
  }
  const text = await res.text();
  const json = text ? (JSON.parse(text) as Record<string, unknown>) : {};
  if (!res.ok) {
    throw new PayrollApiError(
      res.status,
      String(json.error ?? "error"),
      String(json.message ?? `payroll-api returned ${res.status}`),
      json,
    );
  }
  return json as T;
}

const enc = encodeURIComponent;

export const payroll = {
  createUser: (email: string, displayName: string) =>
    call<{ id: string }>("POST", "/v1/users", null, { email, display_name: displayName }),
  listOrgs: (actor: string) => call<Membership[]>("GET", "/v1/orgs", actor),
  createOrg: (actor: string, name: string) => call<{ id: string }>("POST", "/v1/orgs", actor, { name }),
  listMembers: (actor: string, org: string) => call<Member[]>("GET", `/v1/orgs/${enc(org)}/members`, actor),
  addMember: (actor: string, org: string, userId: string, role: Role) =>
    call<{ ok: true }>("POST", `/v1/orgs/${enc(org)}/members`, actor, { user_id: userId, role }),
  setMemberRole: (actor: string, org: string, userId: string, role: Role) =>
    call<{ ok: true }>("PATCH", `/v1/orgs/${enc(org)}/members/${enc(userId)}`, actor, { role }),
  removeMember: (actor: string, org: string, userId: string) =>
    call<{ ok: true }>("DELETE", `/v1/orgs/${enc(org)}/members/${enc(userId)}`, actor),
  listOrgStreams: (actor: string, org: string) => call<StreamView[]>("GET", `/v1/orgs/${enc(org)}/streams`, actor),
  createStream: (
    actor: string,
    org: string,
    req: { recipient_user_id: string; rate_zec: string; rate_period_secs: number; start_at: number | null; end_at: number | null },
  ) => call<StreamView>("POST", `/v1/orgs/${enc(org)}/streams`, actor, req),
  myStreams: (actor: string) => call<StreamView[]>("GET", "/v1/me/streams", actor),
  stream: (actor: string, id: string) => call<StreamDetail>("GET", `/v1/streams/${enc(id)}`, actor),
  lifecycle: (actor: string, id: string, op: "pause" | "resume" | "end") =>
    call<StreamView>("POST", `/v1/streams/${enc(id)}/${op}`, actor),
  reserveClaim: (actor: string, id: string, amountZec: string, idempotencyKey: string) =>
    call<{ claim: ClaimView; duplicate: boolean }>("POST", `/v1/streams/${enc(id)}/claims`, actor, {
      amount_zec: amountZec,
      idempotency_key: idempotencyKey,
    }),
  releaseClaim: (actor: string, claimId: string) => call<ClaimView>("POST", `/v1/claims/${enc(claimId)}/release`, actor),
  quotePayout: (actor: string, claimId: string, address: string) =>
    call<PayoutQuote>("POST", `/v1/claims/${enc(claimId)}/payout/quote`, actor, { address }),
  // Idempotent per claim: a repeat returns the existing payout (duplicate: true).
  requestPayout: (actor: string, claimId: string, address: string) =>
    call<{ payout: PayoutView; duplicate: boolean }>("POST", `/v1/claims/${enc(claimId)}/payout`, actor, { address }),
  payoutWallet: (actor: string, org: string) => call<PayoutWalletSummary>("GET", `/v1/orgs/${enc(org)}/payout-wallet`, actor),
};

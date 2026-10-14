import "server-only";
import { env } from "./env";

// Client for the internal payroll-api (Rust). All payroll rules, accrual,
// milestones and money parsing/formatting happen there; this module only
// moves JSON.

export type Role = "owner" | "admin" | "member";
export type StreamStatus = "active" | "paused" | "ended";
export type OfferStatus = "open" | "bound" | "cancelled";
/** Only `confirmed` means paid. `submitted` = broadcast, awaiting confirmations. */
export type PayoutState = "reserved" | "submitted" | "confirmed" | "failed" | "requires_reconciliation";

export interface Membership {
  organization_id: string;
  organization_name: string;
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

/** A stream offer nobody has bound yet (or that was cancelled). */
export interface OfferView {
  id: string;
  organization_id: string;
  organization_name: string;
  worker_name: string;
  total_zats: number;
  total_zec: string;
  start_at: number;
  end_at: number;
  created_at: number;
  status: OfferStatus;
  stream_id: string | null;
  cancelled_at: number | null;
}

/** What a worker link shows before sign-in. */
export interface LinkPreview {
  organization_name: string;
  worker_name: string;
  total_zats: number;
  total_zec: string;
  start_at: number;
  end_at: number;
  status: OfferStatus;
}

/** A fixed-total stream with a worker. */
export interface FlowStream {
  id: string;
  offer_id: string;
  organization_id: string;
  organization_name: string;
  worker_name: string;
  recipient_user_id: string;
  total_zats: number;
  total_zec: string;
  start_at: number;
  end_at: number;
  status: StreamStatus;
  ended_at: number | null;
  balance: BalanceView;
  milestone_zats: number;
  /** What "Withdraw" would pay now; null when it is not enabled. */
  withdrawable_zats: number | null;
  withdrawable_zec: string | null;
  address_locked: boolean;
  /** The locked address — present only for the stream's worker. */
  address: string | null;
  /** Newest first. */
  payouts: PayoutView[];
  payouts_enabled: boolean;
}

export interface EmployerHome {
  offers: OfferView[];
  streams: FlowStream[];
  payouts_enabled: boolean;
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
  payoutWallet: (actor: string, org: string) => call<PayoutWalletSummary>("GET", `/v1/orgs/${enc(org)}/payout-wallet`, actor),

  // ---- Phase 5 flow ----
  createOffer: (
    actor: string,
    org: string,
    req: { worker_name: string; total_zec: string; start_at: number | null; end_at: number; link_hash: string },
  ) => call<OfferView>("POST", `/v1/orgs/${enc(org)}/offers`, actor, req),
  employerHome: (actor: string, org: string) => call<EmployerHome>("GET", `/v1/orgs/${enc(org)}/home`, actor),
  cancelOffer: (actor: string, offer: string) => call<OfferView>("POST", `/v1/offers/${enc(offer)}/cancel`, actor),
  relinkOffer: (actor: string, offer: string, linkHash: string) =>
    call<OfferView>("POST", `/v1/offers/${enc(offer)}/relink`, actor, { link_hash: linkHash }),
  linkPreview: (linkHash: string) => call<LinkPreview>("GET", `/v1/links/${enc(linkHash)}`, null),
  bindLink: (actor: string, linkHash: string) =>
    call<{ stream_id: string; duplicate: boolean }>("POST", `/v1/links/${enc(linkHash)}/bind`, actor),
  workerStreams: (actor: string) => call<FlowStream[]>("GET", "/v1/me/flow-streams", actor),
  flowStream: (actor: string, id: string) => call<FlowStream>("GET", `/v1/flow-streams/${enc(id)}`, actor),
  endStream: (actor: string, id: string) => call<unknown>("POST", `/v1/streams/${enc(id)}/end`, actor),
  setAddress: (actor: string, id: string, address: string) =>
    call<FlowStream>("POST", `/v1/flow-streams/${enc(id)}/address`, actor, { address }),
  // Exactly-once: a repeat, or any request while a withdrawal is unfinished,
  // returns that withdrawal (duplicate: true).
  withdraw: (actor: string, id: string, idempotencyKey: string) =>
    call<{ payout: PayoutView; duplicate: boolean }>("POST", `/v1/flow-streams/${enc(id)}/withdraw`, actor, {
      idempotency_key: idempotencyKey,
    }),
};

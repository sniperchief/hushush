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

export type CompensationType = "stream" | "vesting";

export interface PayoutView {
  id: string;
  /** What `claim_id` refers to: a stream claim or a vesting claim. */
  compensation_type: CompensationType;
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

/** Testnet top-ups of a company's payout wallet from the shared treasury. */
export interface FaucetSummary {
  amount_zats: number;
  amount_zec: string;
  available: boolean;
  /** When the next top-up is allowed (unix seconds), if waiting. */
  next_at: number | null;
  remaining: number;
}

/** The company's own payout wallet (Phase 6): withdrawals are paid from it. */
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
  faucet: FaucetSummary;
}

export interface FaucetResult {
  payment_id: string;
  state: "pending" | "failed" | "built" | "broadcast";
  amount_zats: number;
  amount_zec: string;
  txid: string | null;
  failure_reason: string | null;
}

export interface Member {
  user_id: string;
  email: string;
  display_name: string;
  role: Role;
  joined_at: number;
}

// ---- vesting ----

export type VestingStatus = "draft" | "active" | "cancelled";
/** Where an agreement is at `balance.as_of` (derived by the server). */
export type VestingPhase = "draft" | "not_started" | "in_cliff" | "vesting" | "fully_vested" | "cancelled";
export type ClaimStatus = "reserved" | "released" | "settled";

export interface VestingBalanceView {
  as_of: number;
  total_zats: number;
  total_zec: string;
  vested_zats: number;
  vested_zec: string;
  unvested_zats: number;
  unvested_zec: string;
  /** Held for a withdrawal in progress; not paid. */
  reserved_zats: number;
  reserved_zec: string;
  /** Paid: confirmed on chain. */
  paid_zats: number;
  paid_zec: string;
  available_zats: number;
  available_zec: string;
}

export interface VestingView {
  id: string;
  organization_id: string;
  organization_name: string;
  employee_user_id: string;
  employee_display_name: string;
  created_by: string;
  method: "linear";
  status: VestingStatus;
  phase: VestingPhase;
  total_zats: number;
  total_zec: string;
  start_at: number;
  end_at: number;
  duration_secs: number;
  cliff_secs: number;
  cliff_at: number;
  /** What vests at once at the cliff (the cliff period's linear share); 0 without a cliff. */
  cliff_vests_zats: number;
  cliff_vests_zec: string;
  created_at: number;
  activated_at: number | null;
  cancelled_at: number | null;
  activatable: boolean;
  cancellable: boolean;
  balance: VestingBalanceView;
}

export interface VestingClaimView {
  id: string;
  compensation_type: "vesting";
  amount_zats: number;
  amount_zec: string;
  status: ClaimStatus;
  created_at: number;
  resolved_at: number | null;
  payout: PayoutView | null;
}

export interface VestingEvent {
  kind: "created" | "activated" | "cancelled";
  actor_user_id: string;
  occurred_at: number;
  vested_zats_after: number;
}

export interface VestingDetail {
  agreement: VestingView;
  viewer: "employee" | "manager";
  events: VestingEvent[];
  /** Newest first. */
  claims: VestingClaimView[];
  /** Newest first. */
  payouts: PayoutView[];
  address_locked: boolean;
  /** The locked address — present only for the agreement's employee. */
  address: string | null;
  payouts_enabled: boolean;
  /** Up to how much may be withdrawn now; null when nothing can be. */
  withdrawable_zats: number | null;
  withdrawable_zec: string | null;
}

/** A fee quote for a withdrawal. Nothing is reserved by quoting. */
export interface QuoteView {
  amount_zats: number;
  amount_zec: string;
  fee_zats: number | null;
  fee_zec: string | null;
  /** Debited from the organization's payout wallet (amount + fee). */
  wallet_debit_zats: number | null;
  valid_recipient: boolean;
  sufficient: boolean;
  reason: string | null;
  network: "testnet";
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
  /** Testnet: tops the company's payout wallet up from the treasury (idempotent per key). */
  faucet: (actor: string, org: string, idempotencyKey: string) =>
    call<FaucetResult>("POST", `/v1/orgs/${enc(org)}/payout-wallet/faucet`, actor, { idempotency_key: idempotencyKey }),

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

  /** Members of an organization (owners/admins only). */
  listMembers: (actor: string, org: string) => call<Member[]>("GET", `/v1/orgs/${enc(org)}/members`, actor),

  // ---- vesting ----
  orgVesting: (actor: string, org: string) => call<VestingView[]>("GET", `/v1/orgs/${enc(org)}/vesting`, actor),
  /** Creates a draft (linear vesting). */
  createVesting: (
    actor: string,
    org: string,
    req: { employee_user_id: string; total_zec: string; start_at: number; duration_secs: number; cliff_secs: number | null },
  ) => call<VestingView>("POST", `/v1/orgs/${enc(org)}/vesting`, actor, req),
  myVesting: (actor: string) => call<VestingView[]>("GET", "/v1/me/vesting", actor),
  vesting: (actor: string, id: string) => call<VestingDetail>("GET", `/v1/vesting/${enc(id)}`, actor),
  activateVesting: (actor: string, id: string) => call<VestingView>("POST", `/v1/vesting/${enc(id)}/activate`, actor),
  cancelVesting: (actor: string, id: string) => call<VestingView>("POST", `/v1/vesting/${enc(id)}/cancel`, actor),
  setVestingAddress: (actor: string, id: string, address: string) =>
    call<VestingDetail>("POST", `/v1/vesting/${enc(id)}/address`, actor, { address }),
  vestingQuote: (actor: string, id: string, amountZec: string) =>
    call<QuoteView>("POST", `/v1/vesting/${enc(id)}/quote`, actor, { amount_zec: amountZec }),
  // Exactly-once: a repeat (same key and amount), or any request while a
  // withdrawal is unfinished, returns that withdrawal (duplicate: true).
  withdrawVesting: (actor: string, id: string, amountZec: string, idempotencyKey: string) =>
    call<{ payout: PayoutView; duplicate: boolean }>("POST", `/v1/vesting/${enc(id)}/withdraw`, actor, {
      amount_zec: amountZec,
      idempotency_key: idempotencyKey,
    }),
};

//! Domain types shared by every storage backend and the HTTP API.

use serde::Serialize;

use crate::accrual::AccrualState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Admin,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "owner" => Some(Role::Owner),
            "admin" => Some(Role::Admin),
            "member" => Some(Role::Member),
            _ => None,
        }
    }
    pub fn manages_payroll(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamStatus {
    Active,
    Paused,
    Ended,
}

impl StreamStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StreamStatus::Active => "active",
            StreamStatus::Paused => "paused",
            StreamStatus::Ended => "ended",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(StreamStatus::Active),
            "paused" => Some(StreamStatus::Paused),
            "ended" => Some(StreamStatus::Ended),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimStatus {
    /// Funds held for the claimant. NOT paid (even while a payout is in flight).
    Reserved,
    /// Reservation cancelled, or its payout failed; funds available again.
    Released,
    /// Paid: its payout was confirmed on chain (or recorded by `settle_claim`).
    Settled,
}

impl ClaimStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimStatus::Reserved => "reserved",
            ClaimStatus::Released => "released",
            ClaimStatus::Settled => "settled",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "reserved" => Some(ClaimStatus::Reserved),
            "released" => Some(ClaimStatus::Released),
            "settled" => Some(ClaimStatus::Settled),
            _ => None,
        }
    }
}

/// Request to create a stream. Times are unix seconds.
#[derive(Clone, Debug)]
pub struct NewStream {
    pub organization_id: String,
    pub recipient_user_id: String,
    /// Zatoshis paid per `rate_period_secs` of active time.
    pub rate_zats: i64,
    pub rate_period_secs: i64,
    /// Defaults to now. May not be in the past (no retroactive accrual).
    pub start_at: Option<i64>,
    pub end_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    pub id: String,
    pub organization_id: String,
    pub recipient_user_id: String,
    pub created_by: String,
    pub status: StreamStatus,
    pub accrual: AccrualState,
    pub ended_at: Option<i64>,
    pub created_at: i64,
    pub last_event_at: i64,
}

impl Stream {
    /// Status as seen at `t`: an active stream past its end date is ended.
    pub fn effective_status_at(&self, t: i64) -> StreamStatus {
        if self.status == StreamStatus::Active && self.accrual.schedule_finished_at(t) {
            StreamStatus::Ended
        } else {
            self.status
        }
    }
}

/// Balances of one stream at `as_of`, all in zatoshis.
///
/// Invariant: `available = earned - reserved - claimed >= 0`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Balance {
    pub stream_id: String,
    pub as_of: i64,
    pub status: StreamStatus,
    pub earned: i64,
    /// Sum of claims in `reserved` state (held, not yet paid).
    pub reserved: i64,
    /// Sum of claims in `settled` state (paid out by a later phase).
    pub claimed: i64,
    pub available: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Claim {
    pub id: String,
    pub stream_id: String,
    pub requested_by: String,
    pub idempotency_key: String,
    pub amount_zats: i64,
    pub status: ClaimStatus,
    pub earned_at_reservation_zats: i64,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    pub settlement_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClaimOutcome {
    pub claim: Claim,
    /// True when this request repeated an earlier one (same idempotency key)
    /// and the original claim was returned instead of creating a new one.
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StreamEvent {
    pub kind: String,
    pub actor_user_id: String,
    pub occurred_at: i64,
    pub banked_active_secs_after: i64,
    pub earned_zats_after: i64,
}

/// An organization the actor belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Membership {
    pub organization_id: String,
    pub organization_name: String,
    pub role: Role,
    pub joined_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Member {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub role: Role,
    pub joined_at: i64,
}

/// A stream with display context and its server-computed balance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamListing {
    pub stream: Stream,
    pub organization_name: String,
    pub recipient_display_name: String,
    pub recipient_email: String,
    pub balance: Balance,
}

/// Raw listing row as loaded by a backend (sums not yet turned into a balance).
#[derive(Clone, Debug)]
pub struct ListedRow {
    pub stream: Stream,
    pub organization_name: String,
    pub recipient_display_name: String,
    pub recipient_email: String,
    pub reserved: i64,
    pub claimed: i64,
}

#[derive(Clone, Copy, Debug)]
pub enum StreamFilter<'a> {
    Organization(&'a str),
    Recipient(&'a str),
}

/// Payout of one reserved claim through the shielded payout wallet.
///
/// ```text
/// reserved ──▶ submitted ──▶ confirmed          (claim → settled)
///    │  ╲          │ ╲
///    │   ╲         │  ╲──▶ failed               (claim → released)
///    │    ╲        ▼
///    │     ╲──▶ requires_reconciliation ──▶ submitted | confirmed | failed
///    ╰──────────────────────────────────────▶ failed
/// ```
///
/// `confirmed` and `failed` are final. A payout is only `failed` once it is
/// certain that no transaction for it can ever be mined; anything ambiguous
/// is `requires_reconciliation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutState {
    /// Payout recorded and claim locked to it; the wallet may or may not have
    /// been asked to pay yet.
    Reserved,
    /// A transaction exists and the network accepted it. NOT yet paid.
    Submitted,
    /// Mined under the confirmation policy and verified. The claim is settled.
    Confirmed,
    /// Established that no transaction was or can be mined. Claim released.
    Failed,
    /// Outcome unknown or inconsistent; resolved by the reconciler (or a human).
    RequiresReconciliation,
}

impl PayoutState {
    pub fn as_str(self) -> &'static str {
        match self {
            PayoutState::Reserved => "reserved",
            PayoutState::Submitted => "submitted",
            PayoutState::Confirmed => "confirmed",
            PayoutState::Failed => "failed",
            PayoutState::RequiresReconciliation => "requires_reconciliation",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "reserved" => Some(PayoutState::Reserved),
            "submitted" => Some(PayoutState::Submitted),
            "confirmed" => Some(PayoutState::Confirmed),
            "failed" => Some(PayoutState::Failed),
            "requires_reconciliation" => Some(PayoutState::RequiresReconciliation),
            _ => None,
        }
    }
    pub fn is_final(self) -> bool {
        matches!(self, PayoutState::Confirmed | PayoutState::Failed)
    }
    /// The allowed state changes (mirrored by the schema triggers).
    pub fn can_become(self, to: PayoutState) -> bool {
        use PayoutState::*;
        matches!(
            (self, to),
            (Reserved, Submitted | Failed | RequiresReconciliation)
                | (Submitted, Confirmed | Failed | RequiresReconciliation)
                | (RequiresReconciliation, Submitted | Confirmed | Failed)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Payout {
    /// Also the wallet's payment id: the wallet builds at most one transaction per id.
    pub id: String,
    /// What this payout pays: a stream claim or a vesting claim.
    pub compensation_type: CompensationType,
    /// The paid claim: a row of `claims` (stream) or of `vesting_claims`
    /// (vesting), per `compensation_type`. One payout per claim, ever.
    pub claim_id: String,
    /// The claim's source: a payroll stream or a vesting agreement, per
    /// `compensation_type`.
    pub source_id: String,
    pub requested_by: String,
    pub amount_zats: i64,
    /// Salted hash of the destination address. The address itself is not
    /// stored in the payroll database (see `payout::recipient_fingerprint`).
    pub recipient_fingerprint: String,
    pub fee_estimate_zats: i64,
    pub max_fee_zats: i64,
    pub state: PayoutState,
    pub txid: Option<String>,
    pub fee_zats: Option<i64>,
    pub expiry_height: Option<i64>,
    pub mined_height: Option<i64>,
    pub confirmations: i64,
    /// Failure or reconciliation reason, for display and operators.
    pub detail: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub submitted_at: Option<i64>,
    pub resolved_at: Option<i64>,
    #[serde(skip)]
    pub lease_owner: Option<String>,
    #[serde(skip)]
    pub lease_until: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PayoutOutcome {
    pub payout: Payout,
    /// True when the claim already had a payout and it was returned instead.
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PayoutEvent {
    pub from_state: Option<PayoutState>,
    pub to_state: PayoutState,
    pub occurred_at: i64,
    pub detail: Option<String>,
}

// ---- Phase 5 flow: stream offers, locked payout addresses ---------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OfferStatus {
    /// Waiting for the worker to open the link and sign in.
    Open,
    /// A worker opened the link; the payroll stream exists.
    Bound,
    /// Cancelled by the employer before anyone bound it.
    Cancelled,
}

/// What an employer creates: a worker's display name and the stream's terms.
/// Bound to a worker (and turned into a payroll stream) through the worker link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StreamOffer {
    pub id: String,
    pub organization_id: String,
    pub organization_name: String,
    pub created_by: String,
    pub worker_name: String,
    /// The stream pays exactly this over `end_at - start_at`.
    pub total_zats: i64,
    pub start_at: i64,
    pub end_at: i64,
    pub created_at: i64,
    pub stream_id: Option<String>,
    pub bound_user_id: Option<String>,
    pub bound_at: Option<i64>,
    pub cancelled_at: Option<i64>,
}

impl StreamOffer {
    pub fn status(&self) -> OfferStatus {
        if self.stream_id.is_some() {
            OfferStatus::Bound
        } else if self.cancelled_at.is_some() {
            OfferStatus::Cancelled
        } else {
            OfferStatus::Open
        }
    }
}

// ---- vesting compensation -----------------------------------------------------------

/// Which kind of compensation a claim draws on. Stream claims live in
/// `claims`, vesting claims in `vesting_claims`; the two never share a table
/// or an id space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompensationType {
    Stream,
    Vesting,
}

impl CompensationType {
    pub fn as_str(self) -> &'static str {
        match self {
            CompensationType::Stream => "stream",
            CompensationType::Vesting => "vesting",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "stream" => Some(CompensationType::Stream),
            "vesting" => Some(CompensationType::Vesting),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VestingMethod {
    /// Linear over the duration, with an optional cliff (see [`crate::vesting`]).
    Linear,
}

impl VestingMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            VestingMethod::Linear => "linear",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "linear" => Some(VestingMethod::Linear),
            _ => None,
        }
    }
}

/// Stored status of a vesting agreement.
///
/// ```text
/// draft ──activate──▶ active ──cancel──▶ cancelled
///   ╰──────────────cancel (void)───────────▲
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VestingStatus {
    /// Created by the employer; nothing vests and the employee can't see it.
    Draft,
    /// Vesting by its schedule.
    Active,
    /// Vesting stopped at `cancelled_at`; what had vested stays owed. Final.
    Cancelled,
}

impl VestingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            VestingStatus::Draft => "draft",
            VestingStatus::Active => "active",
            VestingStatus::Cancelled => "cancelled",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(VestingStatus::Draft),
            "active" => Some(VestingStatus::Active),
            "cancelled" => Some(VestingStatus::Cancelled),
            _ => None,
        }
    }
}

/// Where an agreement is at a given time (derived, never stored).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VestingPhase {
    Draft,
    /// Active, before `start_at`.
    NotStarted,
    /// Active, started, before the cliff: nothing vested yet.
    InCliff,
    /// Active and partly vested.
    Vesting,
    /// Active and at or past the end: the whole allocation is vested.
    FullyVested,
    Cancelled,
}

/// The fixed terms of a vesting agreement. Immutable once created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct VestingTerms {
    pub total_zats: i64,
    pub start_at: i64,
    pub duration_secs: i64,
    /// 0 = no cliff. At most `duration_secs`.
    pub cliff_secs: i64,
    pub method: VestingMethod,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VestingAgreement {
    pub id: String,
    pub organization_id: String,
    pub employee_user_id: String,
    pub created_by: String,
    pub terms: VestingTerms,
    pub status: VestingStatus,
    pub created_at: i64,
    pub updated_at: i64,
    pub activated_at: Option<i64>,
    pub activated_by: Option<String>,
    pub cancelled_at: Option<i64>,
    pub cancelled_by: Option<String>,
    /// Vested amount frozen at cancellation (0 for a voided draft).
    pub vested_at_cancel_zats: Option<i64>,
}

/// Balances of one agreement at `as_of`, all in zatoshis.
///
/// Invariants: `vested + unvested = total`;
/// `available = vested - reserved - paid >= 0`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VestingBalance {
    pub agreement_id: String,
    pub as_of: i64,
    pub phase: VestingPhase,
    pub total: i64,
    pub vested: i64,
    /// Not (yet) vested. After cancellation this is forfeited for good.
    pub unvested: i64,
    /// Sum of claims in `reserved` state (held, not yet paid).
    pub reserved: i64,
    /// Sum of claims in `settled` state.
    pub paid: i64,
    pub available: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VestingClaim {
    pub id: String,
    /// Always [`CompensationType::Vesting`]; carried so API consumers never
    /// have to infer a claim's kind from where it came from.
    pub compensation_type: CompensationType,
    pub agreement_id: String,
    pub requested_by: String,
    pub idempotency_key: String,
    pub amount_zats: i64,
    pub status: ClaimStatus,
    pub vested_at_reservation_zats: i64,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    pub settlement_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VestingClaimOutcome {
    pub claim: VestingClaim,
    /// True when this request repeated an earlier one (same idempotency key).
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VestingEvent {
    pub kind: String,
    pub actor_user_id: String,
    pub occurred_at: i64,
    pub vested_zats_after: i64,
}

/// An agreement with display context and its server-computed balance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VestingListing {
    pub agreement: VestingAgreement,
    pub organization_name: String,
    pub employee_display_name: String,
    pub balance: VestingBalance,
}

/// Raw listing row as loaded by a backend (sums not yet turned into a balance).
#[derive(Clone, Debug)]
pub struct VestingListedRow {
    pub agreement: VestingAgreement,
    pub organization_name: String,
    pub employee_display_name: String,
    pub reserved: i64,
    pub paid: i64,
}

#[derive(Clone, Copy, Debug)]
pub enum VestingFilter<'a> {
    Organization(&'a str),
    /// The employee's agreements that were ever activated.
    Employee(&'a str),
}

/// A vesting agreement's payout address: set once by its employee, then
/// locked. Same sealing and fingerprinting as [`PayoutAddress`], keyed by the
/// agreement id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VestingPayoutAddress {
    pub agreement_id: String,
    pub set_by: String,
    pub address_fingerprint: String,
    pub address_sealed: String,
    pub set_at: i64,
}

/// A stream's payout address, set once by its recipient and locked for the
/// life of the stream. `address_sealed` is opaque to the engine (payroll-api
/// encrypts it); `address_fingerprint` is
/// [`recipient_fingerprint`](crate::payout::recipient_fingerprint)`(stream_id, address)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayoutAddress {
    pub stream_id: String,
    pub set_by: String,
    pub address_fingerprint: String,
    pub address_sealed: String,
    pub set_at: i64,
}

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
    /// Funds held for the claimant. NOT paid.
    Reserved,
    /// Reservation cancelled; funds available again.
    Released,
    /// A payout was recorded by the (future) payout worker.
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

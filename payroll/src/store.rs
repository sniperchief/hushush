//! Storage abstraction. Business rules live once in [`crate::engine`]; a
//! backend only supplies transactions and plain data access.
//!
//! Contract every backend must honour:
//! * `begin(true)` returns a transaction that excludes other writers to the
//!   rows it locks for its duration (SQLite: `BEGIN IMMEDIATE`; PostgreSQL:
//!   `SELECT … FOR UPDATE` on the stream/claim row via `lock = true`).
//! * Constraint violations that represent a duplicate (unique key) are
//!   returned as [`PayrollError::Conflict`](crate::PayrollError::Conflict).

use crate::accrual::AccrualState;
use crate::error::Result;
use crate::model::*;

pub trait Backend {
    type Tx<'a>: StoreTx
    where
        Self: 'a;
    /// Starts a transaction; `write` requests writer exclusion.
    fn begin(&mut self, write: bool) -> Result<Self::Tx<'_>>;
}

#[allow(clippy::too_many_arguments)]
pub trait StoreTx {
    fn commit(self) -> Result<()>
    where
        Self: Sized;

    fn user_exists(&mut self, id: &str) -> Result<bool>;
    fn insert_user(&mut self, id: &str, email: &str, display_name: &str, now: i64) -> Result<()>;
    fn insert_org(&mut self, id: &str, name: &str, created_by: &str, now: i64) -> Result<()>;
    fn role_of(&mut self, org: &str, user: &str) -> Result<Option<Role>>;
    fn insert_member(&mut self, org: &str, user: &str, role: Role, now: i64) -> Result<()>;
    fn update_member_role(&mut self, org: &str, user: &str, role: Role) -> Result<()>;
    fn delete_member(&mut self, org: &str, user: &str) -> Result<()>;
    fn count_owners(&mut self, org: &str) -> Result<i64>;
    fn count_streams_for_member(&mut self, org: &str, user: &str) -> Result<i64>;
    fn memberships_of(&mut self, user: &str) -> Result<Vec<Membership>>;
    fn members_of(&mut self, org: &str) -> Result<Vec<Member>>;

    fn insert_stream(
        &mut self,
        id: &str,
        created_by: &str,
        req: &NewStream,
        start_at: i64,
        now: i64,
    ) -> Result<()>;
    /// `lock = true` must lock the row against concurrent writers.
    fn load_stream(&mut self, id: &str, lock: bool) -> Result<Option<Stream>>;
    /// Compare-and-set on `from`; returns rows updated (0 or 1).
    fn update_stream_cas(
        &mut self,
        id: &str,
        from: StreamStatus,
        to: StreamStatus,
        next: &AccrualState,
        ended_at: Option<i64>,
        now: i64,
    ) -> Result<usize>;
    fn insert_event(
        &mut self,
        stream_id: &str,
        kind: &str,
        actor: &str,
        now: i64,
        banked_after: i64,
        earned_after: i64,
    ) -> Result<()>;
    fn events_of(&mut self, stream_id: &str) -> Result<Vec<StreamEvent>>;
    fn list_streams(&mut self, filter: StreamFilter<'_>) -> Result<Vec<ListedRow>>;

    /// (reserved, settled) sums for the stream.
    fn claim_sums(&mut self, stream_id: &str) -> Result<(i64, i64)>;
    fn claim_by_key(&mut self, stream_id: &str, key: &str) -> Result<Option<Claim>>;
    fn insert_claim(
        &mut self,
        id: &str,
        stream_id: &str,
        requested_by: &str,
        key: &str,
        amount: i64,
        earned: i64,
        now: i64,
    ) -> Result<()>;
    fn load_claim(&mut self, id: &str, lock: bool) -> Result<Option<Claim>>;
    /// Moves a `reserved` claim to `to`; returns rows updated (0 or 1).
    fn update_claim_cas(
        &mut self,
        id: &str,
        to: ClaimStatus,
        now: i64,
        actor: &str,
        settlement_ref: Option<&str>,
    ) -> Result<usize>;
    fn claims_of(&mut self, stream_id: &str) -> Result<Vec<Claim>>;
}

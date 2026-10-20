//! SQLite storage backend (the Phase 1 tested storage layer).
//!
//! Writes use `BEGIN IMMEDIATE`, so SQLite admits one writer at a time across
//! all connections and processes; row locks (`lock = true`) are therefore
//! implicit.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Row, Transaction, TransactionBehavior, params,
};

use crate::accrual::AccrualState;
use crate::clock::Clock;
use crate::engine::Payroll;
use crate::error::{PayrollError, Result};
use crate::migrations;
use crate::model::*;
use crate::store::{Backend, StoreTx};

pub struct SqliteBackend {
    conn: Connection,
}

/// SQLite-backed payroll engine. Open one per thread/request worker.
pub type PayrollDb = Payroll<SqliteBackend>;

impl Payroll<SqliteBackend> {
    /// Opens (creating if needed) the database at `path` and applies migrations.
    pub fn open(path: impl AsRef<Path>, clock: Arc<dyn Clock>) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(30))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        migrations::migrate(&mut conn, clock.now())?;
        Ok(Payroll::from_backend(SqliteBackend { conn }, clock))
    }

    /// Escape hatch for tests and admin tooling.
    pub fn connection(&self) -> &Connection {
        &self.backend.conn
    }
}

impl Backend for SqliteBackend {
    type Tx<'a> = SqliteTx<'a>;
    fn begin(&mut self, write: bool) -> Result<SqliteTx<'_>> {
        let behavior = if write {
            TransactionBehavior::Immediate
        } else {
            TransactionBehavior::Deferred
        };
        Ok(SqliteTx(self.conn.transaction_with_behavior(behavior)?))
    }
}

pub struct SqliteTx<'a>(Transaction<'a>);

const STREAM_COLS: &str = "s.id, s.organization_id, s.recipient_user_id, s.created_by, s.rate_zats, \
     s.rate_period_secs, s.start_at, s.end_at, s.status, s.banked_active_secs, s.active_since, \
     s.ended_at, s.created_at, s.last_event_at";

const CLAIM_COLS: &str = "id, stream_id, requested_by, idempotency_key, amount_zats, status, \
     earned_at_reservation_zats, created_at, resolved_at, settlement_ref";

fn bad_enum(s: String) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(0, s, rusqlite::types::Type::Text)
}

fn stream_from_row(r: &Row<'_>) -> rusqlite::Result<Stream> {
    let status: String = r.get(8)?;
    Ok(Stream {
        id: r.get(0)?,
        organization_id: r.get(1)?,
        recipient_user_id: r.get(2)?,
        created_by: r.get(3)?,
        accrual: AccrualState {
            rate_zats: r.get(4)?,
            rate_period_secs: r.get(5)?,
            start_at: r.get(6)?,
            end_at: r.get(7)?,
            banked_active_secs: r.get(9)?,
            active_since: r.get(10)?,
        },
        status: StreamStatus::parse(&status).ok_or_else(|| bad_enum(status))?,
        ended_at: r.get(11)?,
        created_at: r.get(12)?,
        last_event_at: r.get(13)?,
    })
}

fn claim_from_row(r: &Row<'_>) -> rusqlite::Result<Claim> {
    let status: String = r.get(5)?;
    Ok(Claim {
        id: r.get(0)?,
        stream_id: r.get(1)?,
        requested_by: r.get(2)?,
        idempotency_key: r.get(3)?,
        amount_zats: r.get(4)?,
        status: ClaimStatus::parse(&status).ok_or_else(|| bad_enum(status))?,
        earned_at_reservation_zats: r.get(6)?,
        created_at: r.get(7)?,
        resolved_at: r.get(8)?,
        settlement_ref: r.get(9)?,
    })
}

fn role_col(r: &Row<'_>, i: usize) -> rusqlite::Result<Role> {
    let s: String = r.get(i)?;
    Role::parse(&s).ok_or_else(|| bad_enum(s))
}

fn unique_to_conflict(e: rusqlite::Error, msg: &str) -> PayrollError {
    match &e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == ErrorCode::ConstraintViolation
                && (f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                    || f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY) =>
        {
            PayrollError::Conflict(msg.into())
        }
        _ => e.into(),
    }
}

impl StoreTx for SqliteTx<'_> {
    fn commit(self) -> Result<()> {
        Ok(self.0.commit()?)
    }

    fn user_exists(&mut self, id: &str) -> Result<bool> {
        Ok(self.0.query_row(
            "SELECT EXISTS (SELECT 1 FROM users WHERE id = ?1)",
            [id],
            |r| r.get(0),
        )?)
    }

    fn insert_user(&mut self, id: &str, email: &str, display_name: &str, now: i64) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO users (id, email, display_name, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![id, email, display_name, now],
            )
            .map_err(|e| unique_to_conflict(e, "email already registered"))?;
        Ok(())
    }

    fn insert_org(&mut self, id: &str, name: &str, created_by: &str, now: i64) -> Result<()> {
        self.0.execute(
            "INSERT INTO organizations (id, name, created_by, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, created_by, now],
        )?;
        Ok(())
    }

    fn role_of(&mut self, org: &str, user: &str) -> Result<Option<Role>> {
        Ok(self
            .0
            .query_row(
                "SELECT role FROM organization_members WHERE organization_id = ?1 AND user_id = ?2",
                params![org, user],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| Role::parse(&s)))
    }

    fn insert_member(&mut self, org: &str, user: &str, role: Role, now: i64) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO organization_members (organization_id, user_id, role, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![org, user, role.as_str(), now],
            )
            .map_err(|e| unique_to_conflict(e, "user is already a member"))?;
        Ok(())
    }

    fn update_member_role(&mut self, org: &str, user: &str, role: Role) -> Result<()> {
        self.0.execute(
            "UPDATE organization_members SET role = ?3 WHERE organization_id = ?1 AND user_id = ?2",
            params![org, user, role.as_str()],
        )?;
        Ok(())
    }

    fn delete_member(&mut self, org: &str, user: &str) -> Result<()> {
        self.0.execute(
            "DELETE FROM organization_members WHERE organization_id = ?1 AND user_id = ?2",
            params![org, user],
        )?;
        Ok(())
    }

    fn count_owners(&mut self, org: &str) -> Result<i64> {
        Ok(self.0.query_row(
            "SELECT COUNT(*) FROM organization_members WHERE organization_id = ?1 AND role = 'owner'",
            [org],
            |r| r.get(0),
        )?)
    }

    fn count_streams_for_member(&mut self, org: &str, user: &str) -> Result<i64> {
        Ok(self.0.query_row(
            "SELECT COUNT(*) FROM payroll_streams WHERE organization_id = ?1 AND recipient_user_id = ?2",
            params![org, user],
            |r| r.get(0),
        )?)
    }

    fn memberships_of(&mut self, user: &str) -> Result<Vec<Membership>> {
        let mut q = self.0.prepare(
            "SELECT o.id, o.name, m.role, m.created_at
             FROM organization_members m JOIN organizations o ON o.id = m.organization_id
             WHERE m.user_id = ?1 ORDER BY o.name, o.id",
        )?;
        let rows = q.query_map([user], |r| {
            Ok(Membership {
                organization_id: r.get(0)?,
                organization_name: r.get(1)?,
                role: role_col(r, 2)?,
                joined_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn members_of(&mut self, org: &str) -> Result<Vec<Member>> {
        let mut q = self.0.prepare(
            "SELECT u.id, u.email, u.display_name, m.role, m.created_at
             FROM organization_members m JOIN users u ON u.id = m.user_id
             WHERE m.organization_id = ?1 ORDER BY m.created_at, u.id",
        )?;
        let rows = q.query_map([org], |r| {
            Ok(Member {
                user_id: r.get(0)?,
                email: r.get(1)?,
                display_name: r.get(2)?,
                role: role_col(r, 3)?,
                joined_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn insert_stream(
        &mut self,
        id: &str,
        created_by: &str,
        req: &NewStream,
        start_at: i64,
        now: i64,
    ) -> Result<()> {
        self.0.execute(
            "INSERT INTO payroll_streams (id, organization_id, recipient_user_id, created_by,
                 rate_zats, rate_period_secs, start_at, end_at, status, banked_active_secs,
                 active_since, ended_at, created_at, last_event_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', 0, ?7, NULL, ?9, ?9)",
            params![
                id,
                req.organization_id,
                req.recipient_user_id,
                created_by,
                req.rate_zats,
                req.rate_period_secs,
                start_at,
                req.end_at,
                now
            ],
        )?;
        Ok(())
    }

    fn load_stream(&mut self, id: &str, _lock: bool) -> Result<Option<Stream>> {
        Ok(self
            .0
            .query_row(
                &format!("SELECT {STREAM_COLS} FROM payroll_streams s WHERE s.id = ?1"),
                [id],
                stream_from_row,
            )
            .optional()?)
    }

    fn update_stream_cas(
        &mut self,
        id: &str,
        from: StreamStatus,
        to: StreamStatus,
        next: &AccrualState,
        ended_at: Option<i64>,
        now: i64,
    ) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE payroll_streams
             SET status = ?1, banked_active_secs = ?2, active_since = ?3, ended_at = ?4, last_event_at = ?5
             WHERE id = ?6 AND status = ?7",
            params![
                to.as_str(),
                next.banked_active_secs,
                next.active_since,
                ended_at,
                now,
                id,
                from.as_str()
            ],
        )?)
    }

    fn insert_event(
        &mut self,
        stream_id: &str,
        kind: &str,
        actor: &str,
        now: i64,
        banked_after: i64,
        earned_after: i64,
    ) -> Result<()> {
        self.0.execute(
            "INSERT INTO stream_events (stream_id, kind, actor_user_id, occurred_at,
                 banked_active_secs_after, earned_zats_after)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![stream_id, kind, actor, now, banked_after, earned_after],
        )?;
        Ok(())
    }

    fn events_of(&mut self, stream_id: &str) -> Result<Vec<StreamEvent>> {
        let mut q = self.0.prepare(
            "SELECT kind, actor_user_id, occurred_at, banked_active_secs_after, earned_zats_after
             FROM stream_events WHERE stream_id = ?1 ORDER BY id",
        )?;
        let rows = q.query_map([stream_id], |r| {
            Ok(StreamEvent {
                kind: r.get(0)?,
                actor_user_id: r.get(1)?,
                occurred_at: r.get(2)?,
                banked_active_secs_after: r.get(3)?,
                earned_zats_after: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn list_streams(&mut self, filter: StreamFilter<'_>) -> Result<Vec<ListedRow>> {
        let (cond, arg) = match filter {
            StreamFilter::Organization(o) => ("s.organization_id = ?1", o),
            StreamFilter::Recipient(u) => ("s.recipient_user_id = ?1", u),
        };
        let mut q = self.0.prepare(&format!(
            "SELECT {STREAM_COLS}, o.name, u.display_name, u.email,
                    COALESCE((SELECT SUM(amount_zats) FROM claims c WHERE c.stream_id = s.id AND c.status = 'reserved'), 0),
                    COALESCE((SELECT SUM(amount_zats) FROM claims c WHERE c.stream_id = s.id AND c.status = 'settled'), 0)
             FROM payroll_streams s
             JOIN organizations o ON o.id = s.organization_id
             JOIN users u ON u.id = s.recipient_user_id
             WHERE {cond} ORDER BY s.created_at DESC, s.id"
        ))?;
        let rows = q.query_map([arg], |r| {
            Ok(ListedRow {
                stream: stream_from_row(r)?,
                organization_name: r.get(14)?,
                recipient_display_name: r.get(15)?,
                recipient_email: r.get(16)?,
                reserved: r.get(17)?,
                claimed: r.get(18)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn claim_sums(&mut self, stream_id: &str) -> Result<(i64, i64)> {
        Ok(self.0.query_row(
            "SELECT COALESCE(SUM(CASE WHEN status = 'reserved' THEN amount_zats END), 0),
                    COALESCE(SUM(CASE WHEN status = 'settled'  THEN amount_zats END), 0)
             FROM claims WHERE stream_id = ?1",
            [stream_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    fn claim_by_key(&mut self, stream_id: &str, key: &str) -> Result<Option<Claim>> {
        Ok(self
            .0
            .query_row(
                &format!(
                    "SELECT {CLAIM_COLS} FROM claims WHERE stream_id = ?1 AND idempotency_key = ?2"
                ),
                params![stream_id, key],
                claim_from_row,
            )
            .optional()?)
    }

    fn insert_claim(
        &mut self,
        id: &str,
        stream_id: &str,
        requested_by: &str,
        key: &str,
        amount: i64,
        earned: i64,
        now: i64,
    ) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO claims (id, stream_id, requested_by, idempotency_key, amount_zats, status,
                     earned_at_reservation_zats, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'reserved', ?6, ?7)",
                params![id, stream_id, requested_by, key, amount, earned, now],
            )
            .map_err(|e| unique_to_conflict(e, "duplicate claim request"))?;
        Ok(())
    }

    fn load_claim(&mut self, id: &str, _lock: bool) -> Result<Option<Claim>> {
        Ok(self
            .0
            .query_row(
                &format!("SELECT {CLAIM_COLS} FROM claims WHERE id = ?1"),
                [id],
                claim_from_row,
            )
            .optional()?)
    }

    fn update_claim_cas(
        &mut self,
        id: &str,
        to: ClaimStatus,
        now: i64,
        actor: &str,
        settlement_ref: Option<&str>,
    ) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE claims SET status = ?1, resolved_at = ?2, resolved_by = ?3, settlement_ref = ?4
             WHERE id = ?5 AND status = 'reserved'",
            params![to.as_str(), now, actor, settlement_ref, id],
        )?)
    }

    fn claims_of(&mut self, stream_id: &str) -> Result<Vec<Claim>> {
        let mut q = self.0.prepare(&format!(
            "SELECT {CLAIM_COLS} FROM claims WHERE stream_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows = q.query_map([stream_id], claim_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn insert_payout(&mut self, p: &Payout) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO payouts (id, compensation_type, claim_id, stream_id, vesting_claim_id,
                     vesting_agreement_id, requested_by, amount_zats, recipient_fingerprint,
                     fee_estimate_zats, max_fee_zats, state, confirmations, lease_owner,
                     lease_until, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13, ?14, ?15, ?15)",
                params![
                    p.id,
                    p.compensation_type.as_str(),
                    stream_ref(p).0,
                    stream_ref(p).1,
                    vesting_ref(p).0,
                    vesting_ref(p).1,
                    p.requested_by,
                    p.amount_zats,
                    p.recipient_fingerprint,
                    p.fee_estimate_zats,
                    p.max_fee_zats,
                    p.state.as_str(),
                    p.lease_owner,
                    p.lease_until,
                    p.created_at
                ],
            )
            .map_err(|e| unique_to_conflict(e, "claim already has a payout"))?;
        Ok(())
    }

    fn load_payout(&mut self, id: &str, _lock: bool) -> Result<Option<Payout>> {
        Ok(self
            .0
            .query_row(
                &format!("SELECT {PAYOUT_COLS} FROM payouts WHERE id = ?1"),
                [id],
                payout_from_row,
            )
            .optional()?)
    }

    fn payout_by_claim(&mut self, claim_id: &str, _lock: bool) -> Result<Option<Payout>> {
        Ok(self
            .0
            .query_row(
                &format!("SELECT {PAYOUT_COLS} FROM payouts WHERE claim_id = ?1"),
                [claim_id],
                payout_from_row,
            )
            .optional()?)
    }

    fn payouts_of_stream(&mut self, stream_id: &str) -> Result<Vec<Payout>> {
        let mut q = self.0.prepare(&format!(
            "SELECT {PAYOUT_COLS} FROM payouts WHERE stream_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows = q.query_map([stream_id], payout_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn payout_by_vesting_claim(&mut self, claim_id: &str, _lock: bool) -> Result<Option<Payout>> {
        Ok(self
            .0
            .query_row(
                &format!("SELECT {PAYOUT_COLS} FROM payouts WHERE vesting_claim_id = ?1"),
                [claim_id],
                payout_from_row,
            )
            .optional()?)
    }

    fn payouts_of_vesting(&mut self, agreement_id: &str) -> Result<Vec<Payout>> {
        let mut q = self.0.prepare(&format!(
            "SELECT {PAYOUT_COLS} FROM payouts WHERE vesting_agreement_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows = q.query_map([agreement_id], payout_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn update_payout_cas(
        &mut self,
        n: &Payout,
        expected_state: PayoutState,
        expected_lease_owner: Option<&str>,
    ) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE payouts SET state = ?1, txid = ?2, fee_zats = ?3, expiry_height = ?4,
                 mined_height = ?5, confirmations = ?6, detail = ?7, lease_owner = ?8,
                 lease_until = ?9, updated_at = ?10, submitted_at = ?11, resolved_at = ?12
             WHERE id = ?13 AND state = ?14 AND lease_owner IS ?15",
            params![
                n.state.as_str(),
                n.txid,
                n.fee_zats,
                n.expiry_height,
                n.mined_height,
                n.confirmations,
                n.detail,
                n.lease_owner,
                n.lease_until,
                n.updated_at,
                n.submitted_at,
                n.resolved_at,
                n.id,
                expected_state.as_str(),
                expected_lease_owner
            ],
        )?)
    }

    fn lease_due_payouts(
        &mut self,
        owner: &str,
        now: i64,
        lease_until: i64,
        limit: i64,
    ) -> Result<Vec<Payout>> {
        // BEGIN IMMEDIATE already excludes every other writer.
        let mut q = self.0.prepare(&format!(
            "UPDATE payouts SET lease_owner = ?1, lease_until = ?2
             WHERE id IN (
                 SELECT id FROM payouts
                 WHERE state IN ('reserved', 'submitted', 'requires_reconciliation')
                   AND (lease_until IS NULL OR lease_until <= ?3)
                 ORDER BY updated_at, id LIMIT ?4)
             RETURNING {PAYOUT_COLS}"
        ))?;
        let rows = q.query_map(params![owner, lease_until, now, limit], payout_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn insert_payout_event(
        &mut self,
        payout_id: &str,
        from: Option<PayoutState>,
        to: PayoutState,
        now: i64,
        detail: Option<&str>,
    ) -> Result<()> {
        self.0.execute(
            "INSERT INTO payout_events (payout_id, from_state, to_state, occurred_at, detail)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                payout_id,
                from.map(PayoutState::as_str),
                to.as_str(),
                now,
                detail
            ],
        )?;
        Ok(())
    }

    fn payout_events_of(&mut self, payout_id: &str) -> Result<Vec<PayoutEvent>> {
        let mut q = self.0.prepare(
            "SELECT from_state, to_state, occurred_at, detail
             FROM payout_events WHERE payout_id = ?1 ORDER BY id",
        )?;
        let rows = q.query_map([payout_id], |r| {
            let from: Option<String> = r.get(0)?;
            let to: String = r.get(1)?;
            Ok(PayoutEvent {
                from_state: match from {
                    Some(s) => Some(PayoutState::parse(&s).ok_or_else(|| bad_enum(s))?),
                    None => None,
                },
                to_state: PayoutState::parse(&to).ok_or_else(|| bad_enum(to))?,
                occurred_at: r.get(2)?,
                detail: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn insert_offer(&mut self, o: &StreamOffer, link_hash: &str) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO stream_offers (id, organization_id, created_by, worker_name, total_zats,
                     start_at, end_at, link_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    o.id,
                    o.organization_id,
                    o.created_by,
                    o.worker_name,
                    o.total_zats,
                    o.start_at,
                    o.end_at,
                    link_hash,
                    o.created_at
                ],
            )
            .map_err(|e| unique_to_conflict(e, "worker link already in use"))?;
        Ok(())
    }

    fn load_offer(&mut self, id: &str, _lock: bool) -> Result<Option<StreamOffer>> {
        self.offer_where("f.id = ?1", id)
    }

    fn offer_by_link(&mut self, link_hash: &str, _lock: bool) -> Result<Option<StreamOffer>> {
        self.offer_where("f.link_hash = ?1", link_hash)
    }

    fn offer_by_stream(&mut self, stream_id: &str) -> Result<Option<StreamOffer>> {
        self.offer_where("f.stream_id = ?1", stream_id)
    }

    fn offers_of_org(&mut self, org: &str) -> Result<Vec<StreamOffer>> {
        let mut q = self.0.prepare(&format!(
            "SELECT {OFFER_COLS} FROM stream_offers f JOIN organizations o ON o.id = f.organization_id
             WHERE f.organization_id = ?1 ORDER BY f.created_at DESC, f.id"
        ))?;
        let rows = q.query_map([org], offer_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn bind_offer_cas(&mut self, id: &str, stream_id: &str, user: &str, now: i64) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE stream_offers SET stream_id = ?1, bound_user_id = ?2, bound_at = ?3
             WHERE id = ?4 AND stream_id IS NULL AND cancelled_at IS NULL",
            params![stream_id, user, now, id],
        )?)
    }

    fn cancel_offer_cas(&mut self, id: &str, now: i64) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE stream_offers SET cancelled_at = ?1
             WHERE id = ?2 AND stream_id IS NULL AND cancelled_at IS NULL",
            params![now, id],
        )?)
    }

    fn relink_offer_cas(&mut self, id: &str, link_hash: &str) -> Result<usize> {
        self.0
            .execute(
                "UPDATE stream_offers SET link_hash = ?1
                 WHERE id = ?2 AND stream_id IS NULL AND cancelled_at IS NULL",
                params![link_hash, id],
            )
            .map_err(|e| unique_to_conflict(e, "worker link already in use"))
    }

    fn insert_payout_address(&mut self, a: &PayoutAddress) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO stream_payout_addresses (stream_id, set_by, address_fingerprint,
                     address_sealed, set_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    a.stream_id,
                    a.set_by,
                    a.address_fingerprint,
                    a.address_sealed,
                    a.set_at
                ],
            )
            .map_err(|e| {
                unique_to_conflict(e, "the payout address for this stream is already set")
            })?;
        Ok(())
    }

    fn payout_address(&mut self, stream_id: &str) -> Result<Option<PayoutAddress>> {
        Ok(self
            .0
            .query_row(
                "SELECT stream_id, set_by, address_fingerprint, address_sealed, set_at
                 FROM stream_payout_addresses WHERE stream_id = ?1",
                [stream_id],
                |r| {
                    Ok(PayoutAddress {
                        stream_id: r.get(0)?,
                        set_by: r.get(1)?,
                        address_fingerprint: r.get(2)?,
                        address_sealed: r.get(3)?,
                        set_at: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    fn count_vesting_for_member(&mut self, org: &str, user: &str) -> Result<i64> {
        Ok(self.0.query_row(
            "SELECT COUNT(*) FROM vesting_agreements WHERE organization_id = ?1 AND employee_user_id = ?2",
            params![org, user],
            |r| r.get(0),
        )?)
    }

    fn insert_vesting(&mut self, a: &VestingAgreement) -> Result<()> {
        self.0.execute(
            "INSERT INTO vesting_agreements (id, organization_id, employee_user_id, created_by,
                 total_zats, start_at, duration_secs, cliff_secs, method, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                a.id,
                a.organization_id,
                a.employee_user_id,
                a.created_by,
                a.terms.total_zats,
                a.terms.start_at,
                a.terms.duration_secs,
                a.terms.cliff_secs,
                a.terms.method.as_str(),
                a.status.as_str(),
                a.created_at
            ],
        )?;
        Ok(())
    }

    fn load_vesting(&mut self, id: &str, _lock: bool) -> Result<Option<VestingAgreement>> {
        Ok(self
            .0
            .query_row(
                &format!("SELECT {VESTING_COLS} FROM vesting_agreements v WHERE v.id = ?1"),
                [id],
                vesting_from_row,
            )
            .optional()?)
    }

    fn activate_vesting_cas(&mut self, id: &str, actor: &str, now: i64) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE vesting_agreements
             SET status = 'active', activated_at = ?1, activated_by = ?2, updated_at = ?1
             WHERE id = ?3 AND status = 'draft'",
            params![now, actor, id],
        )?)
    }

    fn cancel_vesting_cas(
        &mut self,
        id: &str,
        from: VestingStatus,
        actor: &str,
        now: i64,
        vested: i64,
    ) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE vesting_agreements
             SET status = 'cancelled', cancelled_at = ?1, cancelled_by = ?2,
                 vested_at_cancel_zats = ?3, updated_at = ?1
             WHERE id = ?4 AND status = ?5",
            params![now, actor, vested, id, from.as_str()],
        )?)
    }

    fn insert_vesting_event(
        &mut self,
        agreement_id: &str,
        kind: &str,
        actor: &str,
        now: i64,
        vested_after: i64,
    ) -> Result<()> {
        self.0.execute(
            "INSERT INTO vesting_events (agreement_id, kind, actor_user_id, occurred_at, vested_zats_after)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![agreement_id, kind, actor, now, vested_after],
        )?;
        Ok(())
    }

    fn vesting_events_of(&mut self, agreement_id: &str) -> Result<Vec<VestingEvent>> {
        let mut q = self.0.prepare(
            "SELECT kind, actor_user_id, occurred_at, vested_zats_after
             FROM vesting_events WHERE agreement_id = ?1 ORDER BY id",
        )?;
        let rows = q.query_map([agreement_id], |r| {
            Ok(VestingEvent {
                kind: r.get(0)?,
                actor_user_id: r.get(1)?,
                occurred_at: r.get(2)?,
                vested_zats_after: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn list_vesting(&mut self, filter: VestingFilter<'_>) -> Result<Vec<VestingListedRow>> {
        let (cond, arg) = match filter {
            VestingFilter::Organization(o) => ("v.organization_id = ?1", o),
            VestingFilter::Employee(u) => {
                ("v.employee_user_id = ?1 AND v.activated_at IS NOT NULL", u)
            }
        };
        let mut q = self.0.prepare(&format!(
            "SELECT {VESTING_COLS}, o.name, u.display_name,
                    COALESCE((SELECT SUM(amount_zats) FROM vesting_claims c WHERE c.agreement_id = v.id AND c.status = 'reserved'), 0),
                    COALESCE((SELECT SUM(amount_zats) FROM vesting_claims c WHERE c.agreement_id = v.id AND c.status = 'settled'), 0)
             FROM vesting_agreements v
             JOIN organizations o ON o.id = v.organization_id
             JOIN users u ON u.id = v.employee_user_id
             WHERE {cond} ORDER BY v.created_at DESC, v.id"
        ))?;
        let rows = q.query_map([arg], |r| {
            Ok(VestingListedRow {
                agreement: vesting_from_row(r)?,
                organization_name: r.get(17)?,
                employee_display_name: r.get(18)?,
                reserved: r.get(19)?,
                paid: r.get(20)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn vesting_claim_sums(&mut self, agreement_id: &str) -> Result<(i64, i64)> {
        Ok(self.0.query_row(
            "SELECT COALESCE(SUM(CASE WHEN status = 'reserved' THEN amount_zats END), 0),
                    COALESCE(SUM(CASE WHEN status = 'settled'  THEN amount_zats END), 0)
             FROM vesting_claims WHERE agreement_id = ?1",
            [agreement_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    fn vesting_claim_by_key(
        &mut self,
        agreement_id: &str,
        key: &str,
    ) -> Result<Option<VestingClaim>> {
        Ok(self
            .0
            .query_row(
                &format!(
                    "SELECT {VCLAIM_COLS} FROM vesting_claims WHERE agreement_id = ?1 AND idempotency_key = ?2"
                ),
                params![agreement_id, key],
                vesting_claim_from_row,
            )
            .optional()?)
    }

    fn insert_vesting_claim(
        &mut self,
        id: &str,
        agreement_id: &str,
        requested_by: &str,
        key: &str,
        amount: i64,
        vested: i64,
        now: i64,
    ) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO vesting_claims (id, agreement_id, requested_by, idempotency_key,
                     amount_zats, status, vested_at_reservation_zats, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'reserved', ?6, ?7)",
                params![id, agreement_id, requested_by, key, amount, vested, now],
            )
            .map_err(|e| unique_to_conflict(e, "duplicate claim request"))?;
        Ok(())
    }

    fn load_vesting_claim(&mut self, id: &str, _lock: bool) -> Result<Option<VestingClaim>> {
        Ok(self
            .0
            .query_row(
                &format!("SELECT {VCLAIM_COLS} FROM vesting_claims WHERE id = ?1"),
                [id],
                vesting_claim_from_row,
            )
            .optional()?)
    }

    fn update_vesting_claim_cas(
        &mut self,
        id: &str,
        to: ClaimStatus,
        now: i64,
        actor: &str,
        settlement_ref: Option<&str>,
    ) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE vesting_claims SET status = ?1, resolved_at = ?2, resolved_by = ?3, settlement_ref = ?4
             WHERE id = ?5 AND status = 'reserved'",
            params![to.as_str(), now, actor, settlement_ref, id],
        )?)
    }

    fn vesting_claims_of(&mut self, agreement_id: &str) -> Result<Vec<VestingClaim>> {
        let mut q = self.0.prepare(&format!(
            "SELECT {VCLAIM_COLS} FROM vesting_claims WHERE agreement_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows = q.query_map([agreement_id], vesting_claim_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn insert_vesting_address(&mut self, a: &VestingPayoutAddress) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO vesting_payout_addresses (agreement_id, set_by, address_fingerprint,
                     address_sealed, set_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    a.agreement_id,
                    a.set_by,
                    a.address_fingerprint,
                    a.address_sealed,
                    a.set_at
                ],
            )
            .map_err(|e| {
                unique_to_conflict(e, "the payout address for this agreement is already set")
            })?;
        Ok(())
    }

    fn vesting_address(&mut self, agreement_id: &str) -> Result<Option<VestingPayoutAddress>> {
        Ok(self
            .0
            .query_row(
                "SELECT agreement_id, set_by, address_fingerprint, address_sealed, set_at
                 FROM vesting_payout_addresses WHERE agreement_id = ?1",
                [agreement_id],
                |r| {
                    Ok(VestingPayoutAddress {
                        agreement_id: r.get(0)?,
                        set_by: r.get(1)?,
                        address_fingerprint: r.get(2)?,
                        address_sealed: r.get(3)?,
                        set_at: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }
}

const VESTING_COLS: &str = "v.id, v.organization_id, v.employee_user_id, v.created_by, v.total_zats, \
     v.start_at, v.duration_secs, v.cliff_secs, v.method, v.status, v.created_at, v.updated_at, \
     v.activated_at, v.activated_by, v.cancelled_at, v.cancelled_by, v.vested_at_cancel_zats";

fn vesting_from_row(r: &Row<'_>) -> rusqlite::Result<VestingAgreement> {
    let method: String = r.get(8)?;
    let status: String = r.get(9)?;
    Ok(VestingAgreement {
        id: r.get(0)?,
        organization_id: r.get(1)?,
        employee_user_id: r.get(2)?,
        created_by: r.get(3)?,
        terms: VestingTerms {
            total_zats: r.get(4)?,
            start_at: r.get(5)?,
            duration_secs: r.get(6)?,
            cliff_secs: r.get(7)?,
            method: VestingMethod::parse(&method).ok_or_else(|| bad_enum(method))?,
        },
        status: VestingStatus::parse(&status).ok_or_else(|| bad_enum(status))?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
        activated_at: r.get(12)?,
        activated_by: r.get(13)?,
        cancelled_at: r.get(14)?,
        cancelled_by: r.get(15)?,
        vested_at_cancel_zats: r.get(16)?,
    })
}

const VCLAIM_COLS: &str = "id, agreement_id, requested_by, idempotency_key, amount_zats, status, \
     vested_at_reservation_zats, created_at, resolved_at, settlement_ref";

fn vesting_claim_from_row(r: &Row<'_>) -> rusqlite::Result<VestingClaim> {
    let status: String = r.get(5)?;
    Ok(VestingClaim {
        id: r.get(0)?,
        compensation_type: CompensationType::Vesting,
        agreement_id: r.get(1)?,
        requested_by: r.get(2)?,
        idempotency_key: r.get(3)?,
        amount_zats: r.get(4)?,
        status: ClaimStatus::parse(&status).ok_or_else(|| bad_enum(status))?,
        vested_at_reservation_zats: r.get(6)?,
        created_at: r.get(7)?,
        resolved_at: r.get(8)?,
        settlement_ref: r.get(9)?,
    })
}

impl SqliteTx<'_> {
    fn offer_where(&mut self, cond: &str, arg: &str) -> Result<Option<StreamOffer>> {
        Ok(self
            .0
            .query_row(
                &format!(
                    "SELECT {OFFER_COLS} FROM stream_offers f
                     JOIN organizations o ON o.id = f.organization_id WHERE {cond}"
                ),
                [arg],
                offer_from_row,
            )
            .optional()?)
    }
}

const OFFER_COLS: &str = "f.id, f.organization_id, o.name, f.created_by, f.worker_name, f.total_zats,      f.start_at, f.end_at, f.created_at, f.stream_id, f.bound_user_id, f.bound_at, f.cancelled_at";

fn offer_from_row(r: &Row<'_>) -> rusqlite::Result<StreamOffer> {
    Ok(StreamOffer {
        id: r.get(0)?,
        organization_id: r.get(1)?,
        organization_name: r.get(2)?,
        created_by: r.get(3)?,
        worker_name: r.get(4)?,
        total_zats: r.get(5)?,
        start_at: r.get(6)?,
        end_at: r.get(7)?,
        created_at: r.get(8)?,
        stream_id: r.get(9)?,
        bound_user_id: r.get(10)?,
        bound_at: r.get(11)?,
        cancelled_at: r.get(12)?,
    })
}

const PAYOUT_COLS: &str = "id, compensation_type, claim_id, stream_id, vesting_claim_id, \
     vesting_agreement_id, requested_by, amount_zats, recipient_fingerprint, fee_estimate_zats, \
     max_fee_zats, state, txid, fee_zats, expiry_height, mined_height, confirmations, detail, \
     created_at, updated_at, submitted_at, resolved_at, lease_owner, lease_until";

/// (claim_id, stream_id) columns of a stream payout; NULLs otherwise.
fn stream_ref(p: &Payout) -> (Option<&str>, Option<&str>) {
    match p.compensation_type {
        CompensationType::Stream => (Some(&p.claim_id), Some(&p.source_id)),
        CompensationType::Vesting => (None, None),
    }
}

/// (vesting_claim_id, vesting_agreement_id) columns of a vesting payout; NULLs otherwise.
fn vesting_ref(p: &Payout) -> (Option<&str>, Option<&str>) {
    match p.compensation_type {
        CompensationType::Vesting => (Some(&p.claim_id), Some(&p.source_id)),
        CompensationType::Stream => (None, None),
    }
}

fn payout_from_row(r: &Row<'_>) -> rusqlite::Result<Payout> {
    let kind: String = r.get(1)?;
    let kind = CompensationType::parse(&kind).ok_or_else(|| bad_enum(kind))?;
    let (claim, source): (Option<String>, Option<String>) = match kind {
        CompensationType::Stream => (r.get(2)?, r.get(3)?),
        CompensationType::Vesting => (r.get(4)?, r.get(5)?),
    };
    let (Some(claim_id), Some(source_id)) = (claim, source) else {
        return Err(bad_enum("payout without its claim reference".into()));
    };
    let state: String = r.get(11)?;
    Ok(Payout {
        id: r.get(0)?,
        compensation_type: kind,
        claim_id,
        source_id,
        requested_by: r.get(6)?,
        amount_zats: r.get(7)?,
        recipient_fingerprint: r.get(8)?,
        fee_estimate_zats: r.get(9)?,
        max_fee_zats: r.get(10)?,
        state: PayoutState::parse(&state).ok_or_else(|| bad_enum(state))?,
        txid: r.get(12)?,
        fee_zats: r.get(13)?,
        expiry_height: r.get(14)?,
        mined_height: r.get(15)?,
        confirmations: r.get(16)?,
        detail: r.get(17)?,
        created_at: r.get(18)?,
        updated_at: r.get(19)?,
        submitted_at: r.get(20)?,
        resolved_at: r.get(21)?,
        lease_owner: r.get(22)?,
        lease_until: r.get(23)?,
    })
}

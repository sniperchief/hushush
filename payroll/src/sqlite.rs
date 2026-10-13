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
}

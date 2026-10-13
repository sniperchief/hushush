//! PostgreSQL storage backend, used by the deployed web app (via `payroll-api`).
//!
//! Transactions run at READ COMMITTED. Every write path that must not
//! interleave locks the relevant row with `SELECT … FOR UPDATE` (stream for
//! lifecycle changes and claim reservations, claim for release/settle), which
//! serializes concurrent requests per stream exactly like SQLite's single
//! writer does.

use std::borrow::BorrowMut;
use std::sync::Arc;

use postgres::error::SqlState;
use postgres::{Client, IsolationLevel, Row, Transaction};

use crate::accrual::AccrualState;
use crate::clock::Clock;
use crate::engine::Payroll;
use crate::error::{PayrollError, Result};
use crate::migrations;
use crate::model::*;
use crate::store::{Backend, StoreTx};

pub struct PgBackend<C: BorrowMut<Client> = Client> {
    client: C,
}

/// PostgreSQL-backed payroll engine.
pub type PgPayrollDb<C = Client> = Payroll<PgBackend<C>>;

/// Opens a client for `url`. `sslmode=require` (or `verify-full`) in the URL
/// enables TLS with the Mozilla root store; otherwise the connection is plain
/// (suitable only for localhost / private networks).
pub fn connect_client(url: &str) -> Result<Client> {
    let wants_tls = url.contains("sslmode=require") || url.contains("sslmode=verify");
    if wants_tls {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tls = tokio_postgres_rustls::MakeRustlsConnect::new(config);
        Ok(Client::connect(url, tls)?)
    } else {
        Ok(Client::connect(url, postgres::NoTls)?)
    }
}

/// Sets the schema search path (used to isolate tests). The schema is created
/// if missing.
pub fn use_schema(client: &mut Client, schema: &str) -> Result<()> {
    if schema.is_empty()
        || !schema
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(PayrollError::Validation(
            "schema name must be [a-z0-9_]+".into(),
        ));
    }
    client.batch_execute(&format!(
        "CREATE SCHEMA IF NOT EXISTS {schema}; SET search_path TO {schema}"
    ))?;
    Ok(())
}

impl<C: BorrowMut<Client>> Payroll<PgBackend<C>> {
    /// Wraps an existing client (e.g. from a pool). Migrations are NOT run;
    /// call [`migrations::migrate_pg`] once at service start-up.
    pub fn with_client(client: C, clock: Arc<dyn Clock>) -> Self {
        Payroll::from_backend(PgBackend { client }, clock)
    }

    pub fn client(&mut self) -> &mut Client {
        self.backend.client.borrow_mut()
    }
}

impl Payroll<PgBackend<Client>> {
    /// Connects, optionally switches schema, and applies pending migrations.
    pub fn connect(url: &str, schema: Option<&str>, clock: Arc<dyn Clock>) -> Result<Self> {
        let mut client = connect_client(url)?;
        if let Some(s) = schema {
            use_schema(&mut client, s)?;
        }
        migrations::migrate_pg(&mut client, clock.now())?;
        Ok(Self::with_client(client, clock))
    }
}

impl<C: BorrowMut<Client>> Backend for PgBackend<C> {
    type Tx<'a>
        = PgTx<'a>
    where
        C: 'a;
    fn begin(&mut self, write: bool) -> Result<PgTx<'_>> {
        let tx = self
            .client
            .borrow_mut()
            .build_transaction()
            .isolation_level(if write {
                IsolationLevel::ReadCommitted
            } else {
                // One consistent snapshot for multi-statement reads.
                IsolationLevel::RepeatableRead
            })
            .read_only(!write)
            .start()?;
        Ok(PgTx(tx))
    }
}

pub struct PgTx<'a>(Transaction<'a>);

const STREAM_COLS: &str = "s.id, s.organization_id, s.recipient_user_id, s.created_by, s.rate_zats, \
     s.rate_period_secs, s.start_at, s.end_at, s.status, s.banked_active_secs, s.active_since, \
     s.ended_at, s.created_at, s.last_event_at";

const CLAIM_COLS: &str = "id, stream_id, requested_by, idempotency_key, amount_zats, status, \
     earned_at_reservation_zats, created_at, resolved_at, settlement_ref";

fn corrupt(what: &str) -> PayrollError {
    PayrollError::InvalidState(format!("unrecognised {what} value in database"))
}

fn stream_from_row(r: &Row) -> Result<Stream> {
    let status: String = r.try_get(8)?;
    Ok(Stream {
        id: r.try_get(0)?,
        organization_id: r.try_get(1)?,
        recipient_user_id: r.try_get(2)?,
        created_by: r.try_get(3)?,
        accrual: AccrualState {
            rate_zats: r.try_get(4)?,
            rate_period_secs: r.try_get(5)?,
            start_at: r.try_get(6)?,
            end_at: r.try_get(7)?,
            banked_active_secs: r.try_get(9)?,
            active_since: r.try_get(10)?,
        },
        status: StreamStatus::parse(&status).ok_or_else(|| corrupt("stream status"))?,
        ended_at: r.try_get(11)?,
        created_at: r.try_get(12)?,
        last_event_at: r.try_get(13)?,
    })
}

fn claim_from_row(r: &Row) -> Result<Claim> {
    let status: String = r.try_get(5)?;
    Ok(Claim {
        id: r.try_get(0)?,
        stream_id: r.try_get(1)?,
        requested_by: r.try_get(2)?,
        idempotency_key: r.try_get(3)?,
        amount_zats: r.try_get(4)?,
        status: ClaimStatus::parse(&status).ok_or_else(|| corrupt("claim status"))?,
        earned_at_reservation_zats: r.try_get(6)?,
        created_at: r.try_get(7)?,
        resolved_at: r.try_get(8)?,
        settlement_ref: r.try_get(9)?,
    })
}

fn role_col(r: &Row, i: usize) -> Result<Role> {
    let s: String = r.try_get(i)?;
    Role::parse(&s).ok_or_else(|| corrupt("role"))
}

fn unique_to_conflict(e: postgres::Error, msg: &str) -> PayrollError {
    if e.code() == Some(&SqlState::UNIQUE_VIOLATION) {
        PayrollError::Conflict(msg.into())
    } else {
        e.into()
    }
}

impl StoreTx for PgTx<'_> {
    fn commit(self) -> Result<()> {
        Ok(self.0.commit()?)
    }

    fn user_exists(&mut self, id: &str) -> Result<bool> {
        Ok(self
            .0
            .query_one("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)", &[&id])?
            .try_get(0)?)
    }

    fn insert_user(&mut self, id: &str, email: &str, display_name: &str, now: i64) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO users (id, email, display_name, created_at) VALUES ($1, $2, $3, $4)",
                &[&id, &email, &display_name, &now],
            )
            .map_err(|e| unique_to_conflict(e, "email already registered"))?;
        Ok(())
    }

    fn insert_org(&mut self, id: &str, name: &str, created_by: &str, now: i64) -> Result<()> {
        self.0.execute(
            "INSERT INTO organizations (id, name, created_by, created_at) VALUES ($1, $2, $3, $4)",
            &[&id, &name, &created_by, &now],
        )?;
        Ok(())
    }

    fn role_of(&mut self, org: &str, user: &str) -> Result<Option<Role>> {
        let row = self.0.query_opt(
            "SELECT role FROM organization_members WHERE organization_id = $1 AND user_id = $2",
            &[&org, &user],
        )?;
        Ok(match row {
            Some(r) => Some(role_col(&r, 0)?),
            None => None,
        })
    }

    fn insert_member(&mut self, org: &str, user: &str, role: Role, now: i64) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO organization_members (organization_id, user_id, role, created_at)
                 VALUES ($1, $2, $3, $4)",
                &[&org, &user, &role.as_str(), &now],
            )
            .map_err(|e| unique_to_conflict(e, "user is already a member"))?;
        Ok(())
    }

    fn update_member_role(&mut self, org: &str, user: &str, role: Role) -> Result<()> {
        self.0.execute(
            "UPDATE organization_members SET role = $3 WHERE organization_id = $1 AND user_id = $2",
            &[&org, &user, &role.as_str()],
        )?;
        Ok(())
    }

    fn delete_member(&mut self, org: &str, user: &str) -> Result<()> {
        self.0.execute(
            "DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2",
            &[&org, &user],
        )?;
        Ok(())
    }

    fn count_owners(&mut self, org: &str) -> Result<i64> {
        Ok(self
            .0
            .query_one(
                "SELECT COUNT(*) FROM organization_members WHERE organization_id = $1 AND role = 'owner'",
                &[&org],
            )?
            .try_get(0)?)
    }

    fn count_streams_for_member(&mut self, org: &str, user: &str) -> Result<i64> {
        Ok(self
            .0
            .query_one(
                "SELECT COUNT(*) FROM payroll_streams WHERE organization_id = $1 AND recipient_user_id = $2",
                &[&org, &user],
            )?
            .try_get(0)?)
    }

    fn memberships_of(&mut self, user: &str) -> Result<Vec<Membership>> {
        self.0
            .query(
                "SELECT o.id, o.name, m.role, m.created_at
                 FROM organization_members m JOIN organizations o ON o.id = m.organization_id
                 WHERE m.user_id = $1 ORDER BY o.name, o.id",
                &[&user],
            )?
            .iter()
            .map(|r| {
                Ok(Membership {
                    organization_id: r.try_get(0)?,
                    organization_name: r.try_get(1)?,
                    role: role_col(r, 2)?,
                    joined_at: r.try_get(3)?,
                })
            })
            .collect()
    }

    fn members_of(&mut self, org: &str) -> Result<Vec<Member>> {
        self.0
            .query(
                "SELECT u.id, u.email, u.display_name, m.role, m.created_at
                 FROM organization_members m JOIN users u ON u.id = m.user_id
                 WHERE m.organization_id = $1 ORDER BY m.created_at, u.id",
                &[&org],
            )?
            .iter()
            .map(|r| {
                Ok(Member {
                    user_id: r.try_get(0)?,
                    email: r.try_get(1)?,
                    display_name: r.try_get(2)?,
                    role: role_col(r, 3)?,
                    joined_at: r.try_get(4)?,
                })
            })
            .collect()
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
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', 0, $7, NULL, $9, $9)",
            &[
                &id,
                &req.organization_id,
                &req.recipient_user_id,
                &created_by,
                &req.rate_zats,
                &req.rate_period_secs,
                &start_at,
                &req.end_at,
                &now,
            ],
        )?;
        Ok(())
    }

    fn load_stream(&mut self, id: &str, lock: bool) -> Result<Option<Stream>> {
        let sql = format!(
            "SELECT {STREAM_COLS} FROM payroll_streams s WHERE s.id = $1{}",
            if lock { " FOR UPDATE" } else { "" }
        );
        match self.0.query_opt(&sql, &[&id])? {
            Some(r) => Ok(Some(stream_from_row(&r)?)),
            None => Ok(None),
        }
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
             SET status = $1, banked_active_secs = $2, active_since = $3, ended_at = $4, last_event_at = $5
             WHERE id = $6 AND status = $7",
            &[
                &to.as_str(),
                &next.banked_active_secs,
                &next.active_since,
                &ended_at,
                &now,
                &id,
                &from.as_str(),
            ],
        )? as usize)
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
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &stream_id,
                &kind,
                &actor,
                &now,
                &banked_after,
                &earned_after,
            ],
        )?;
        Ok(())
    }

    fn events_of(&mut self, stream_id: &str) -> Result<Vec<StreamEvent>> {
        self.0
            .query(
                "SELECT kind, actor_user_id, occurred_at, banked_active_secs_after, earned_zats_after
                 FROM stream_events WHERE stream_id = $1 ORDER BY id",
                &[&stream_id],
            )?
            .iter()
            .map(|r| {
                Ok(StreamEvent {
                    kind: r.try_get(0)?,
                    actor_user_id: r.try_get(1)?,
                    occurred_at: r.try_get(2)?,
                    banked_active_secs_after: r.try_get(3)?,
                    earned_zats_after: r.try_get(4)?,
                })
            })
            .collect()
    }

    fn list_streams(&mut self, filter: StreamFilter<'_>) -> Result<Vec<ListedRow>> {
        let (cond, arg) = match filter {
            StreamFilter::Organization(o) => ("s.organization_id = $1", o),
            StreamFilter::Recipient(u) => ("s.recipient_user_id = $1", u),
        };
        self.0
            .query(
                &format!(
                    "SELECT {STREAM_COLS}, o.name, u.display_name, u.email,
                        COALESCE((SELECT SUM(amount_zats) FROM claims c WHERE c.stream_id = s.id AND c.status = 'reserved'), 0)::BIGINT,
                        COALESCE((SELECT SUM(amount_zats) FROM claims c WHERE c.stream_id = s.id AND c.status = 'settled'), 0)::BIGINT
                     FROM payroll_streams s
                     JOIN organizations o ON o.id = s.organization_id
                     JOIN users u ON u.id = s.recipient_user_id
                     WHERE {cond} ORDER BY s.created_at DESC, s.id"
                ),
                &[&arg],
            )?
            .iter()
            .map(|r| {
                Ok(ListedRow {
                    stream: stream_from_row(r)?,
                    organization_name: r.try_get(14)?,
                    recipient_display_name: r.try_get(15)?,
                    recipient_email: r.try_get(16)?,
                    reserved: r.try_get(17)?,
                    claimed: r.try_get(18)?,
                })
            })
            .collect()
    }

    fn claim_sums(&mut self, stream_id: &str) -> Result<(i64, i64)> {
        let r = self.0.query_one(
            "SELECT COALESCE(SUM(CASE WHEN status = 'reserved' THEN amount_zats END), 0)::BIGINT,
                    COALESCE(SUM(CASE WHEN status = 'settled'  THEN amount_zats END), 0)::BIGINT
             FROM claims WHERE stream_id = $1",
            &[&stream_id],
        )?;
        Ok((r.try_get(0)?, r.try_get(1)?))
    }

    fn claim_by_key(&mut self, stream_id: &str, key: &str) -> Result<Option<Claim>> {
        match self.0.query_opt(
            &format!(
                "SELECT {CLAIM_COLS} FROM claims WHERE stream_id = $1 AND idempotency_key = $2"
            ),
            &[&stream_id, &key],
        )? {
            Some(r) => Ok(Some(claim_from_row(&r)?)),
            None => Ok(None),
        }
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
                 VALUES ($1, $2, $3, $4, $5, 'reserved', $6, $7)",
                &[&id, &stream_id, &requested_by, &key, &amount, &earned, &now],
            )
            .map_err(|e| unique_to_conflict(e, "duplicate claim request"))?;
        Ok(())
    }

    fn load_claim(&mut self, id: &str, lock: bool) -> Result<Option<Claim>> {
        let sql = format!(
            "SELECT {CLAIM_COLS} FROM claims WHERE id = $1{}",
            if lock { " FOR UPDATE" } else { "" }
        );
        match self.0.query_opt(&sql, &[&id])? {
            Some(r) => Ok(Some(claim_from_row(&r)?)),
            None => Ok(None),
        }
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
            "UPDATE claims SET status = $1, resolved_at = $2, resolved_by = $3, settlement_ref = $4
             WHERE id = $5 AND status = 'reserved'",
            &[&to.as_str(), &now, &actor, &settlement_ref, &id],
        )? as usize)
    }

    fn claims_of(&mut self, stream_id: &str) -> Result<Vec<Claim>> {
        self.0
            .query(
                &format!(
                    "SELECT {CLAIM_COLS} FROM claims WHERE stream_id = $1 ORDER BY created_at, id"
                ),
                &[&stream_id],
            )?
            .iter()
            .map(claim_from_row)
            .collect()
    }
}

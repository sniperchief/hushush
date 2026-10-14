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

/// Extra trusted CA certificates for the database's TLS certificate — e.g.
/// Supabase signs its database certificates with its own root CA, which is
/// not in the public root store. From `PAYROLL_DATABASE_CA_PEM` (the PEM text;
/// literal `\n` sequences are accepted for single-line secret stores) or
/// `PAYROLL_DATABASE_CA_FILE` (a path).
fn extra_ca_certs() -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    use rustls::pki_types::{CertificateDer, pem::PemObject};
    let pem = match (
        std::env::var("PAYROLL_DATABASE_CA_PEM"),
        std::env::var("PAYROLL_DATABASE_CA_FILE"),
    ) {
        (Ok(p), _) if !p.trim().is_empty() => p.replace("\\n", "\n").into_bytes(),
        (_, Ok(f)) if !f.trim().is_empty() => std::fs::read(f.trim()).map_err(|e| {
            PayrollError::Validation(format!("cannot read PAYROLL_DATABASE_CA_FILE: {e}"))
        })?,
        _ => return Ok(vec![]),
    };
    let certs = CertificateDer::pem_slice_iter(&pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| PayrollError::Validation(format!("invalid database CA PEM: {e}")))?;
    if certs.is_empty() {
        return Err(PayrollError::Validation(
            "database CA PEM contains no certificate".into(),
        ));
    }
    Ok(certs)
}

/// Opens a client for `url`. TLS (always verified: certificate chain and host
/// name) is used when the URL has `sslmode=require`/`verify-*` or when an
/// extra CA is configured (see [`extra_ca_certs`]); trusted roots are the
/// Mozilla store plus that CA. Otherwise the connection is plain (suitable
/// only for localhost / private networks).
pub fn connect_client(url: &str) -> Result<Client> {
    let extra = extra_ca_certs()?;
    let wants_tls =
        url.contains("sslmode=require") || url.contains("sslmode=verify") || !extra.is_empty();
    if wants_tls {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for cert in extra {
            roots
                .add(cert)
                .map_err(|e| PayrollError::Validation(format!("unusable database CA: {e}")))?;
        }
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tls = tokio_postgres_rustls::MakeRustlsConnect::new(config);
        // Never let the driver's default `sslmode=prefer` fall back to plain text.
        let url = if url.contains("sslmode=") {
            url.to_string()
        } else {
            format!(
                "{url}{}sslmode=require",
                if url.contains('?') { '&' } else { '?' }
            )
        };
        Ok(Client::connect(&url, tls)?)
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

    fn insert_payout(&mut self, p: &Payout) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO payouts (id, claim_id, stream_id, requested_by, amount_zats,
                     recipient_fingerprint, fee_estimate_zats, max_fee_zats, state, confirmations,
                     lease_owner, lease_until, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, $10, $11, $12, $12)",
                &[
                    &p.id,
                    &p.claim_id,
                    &p.stream_id,
                    &p.requested_by,
                    &p.amount_zats,
                    &p.recipient_fingerprint,
                    &p.fee_estimate_zats,
                    &p.max_fee_zats,
                    &p.state.as_str(),
                    &p.lease_owner,
                    &p.lease_until,
                    &p.created_at,
                ],
            )
            .map_err(|e| unique_to_conflict(e, "claim already has a payout"))?;
        Ok(())
    }

    fn load_payout(&mut self, id: &str, lock: bool) -> Result<Option<Payout>> {
        let sql = format!(
            "SELECT {PAYOUT_COLS} FROM payouts WHERE id = $1{}",
            if lock { " FOR UPDATE" } else { "" }
        );
        match self.0.query_opt(&sql, &[&id])? {
            Some(r) => Ok(Some(payout_from_row(&r)?)),
            None => Ok(None),
        }
    }

    fn payout_by_claim(&mut self, claim_id: &str, lock: bool) -> Result<Option<Payout>> {
        let sql = format!(
            "SELECT {PAYOUT_COLS} FROM payouts WHERE claim_id = $1{}",
            if lock { " FOR UPDATE" } else { "" }
        );
        match self.0.query_opt(&sql, &[&claim_id])? {
            Some(r) => Ok(Some(payout_from_row(&r)?)),
            None => Ok(None),
        }
    }

    fn payouts_of_stream(&mut self, stream_id: &str) -> Result<Vec<Payout>> {
        self.0
            .query(
                &format!(
                    "SELECT {PAYOUT_COLS} FROM payouts WHERE stream_id = $1 ORDER BY created_at, id"
                ),
                &[&stream_id],
            )?
            .iter()
            .map(payout_from_row)
            .collect()
    }

    fn update_payout_cas(
        &mut self,
        n: &Payout,
        expected_state: PayoutState,
        expected_lease_owner: Option<&str>,
    ) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE payouts SET state = $1, txid = $2, fee_zats = $3, expiry_height = $4,
                 mined_height = $5, confirmations = $6, detail = $7, lease_owner = $8,
                 lease_until = $9, updated_at = $10, submitted_at = $11, resolved_at = $12
             WHERE id = $13 AND state = $14 AND lease_owner IS NOT DISTINCT FROM $15",
            &[
                &n.state.as_str(),
                &n.txid,
                &n.fee_zats,
                &n.expiry_height,
                &n.mined_height,
                &n.confirmations,
                &n.detail,
                &n.lease_owner,
                &n.lease_until,
                &n.updated_at,
                &n.submitted_at,
                &n.resolved_at,
                &n.id,
                &expected_state.as_str(),
                &expected_lease_owner,
            ],
        )? as usize)
    }

    fn lease_due_payouts(
        &mut self,
        owner: &str,
        now: i64,
        lease_until: i64,
        limit: i64,
    ) -> Result<Vec<Payout>> {
        // SKIP LOCKED: concurrent workers never wait on (or double-lease) a row.
        self.0
            .query(
                &format!(
                    "UPDATE payouts SET lease_owner = $1, lease_until = $2
                     WHERE id IN (
                         SELECT id FROM payouts
                         WHERE state IN ('reserved', 'submitted', 'requires_reconciliation')
                           AND (lease_until IS NULL OR lease_until <= $3)
                         ORDER BY updated_at, id LIMIT $4
                         FOR UPDATE SKIP LOCKED)
                     RETURNING {PAYOUT_COLS}"
                ),
                &[&owner, &lease_until, &now, &limit],
            )?
            .iter()
            .map(payout_from_row)
            .collect()
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
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &payout_id,
                &from.map(PayoutState::as_str),
                &to.as_str(),
                &now,
                &detail,
            ],
        )?;
        Ok(())
    }

    fn payout_events_of(&mut self, payout_id: &str) -> Result<Vec<PayoutEvent>> {
        self.0
            .query(
                "SELECT from_state, to_state, occurred_at, detail
                 FROM payout_events WHERE payout_id = $1 ORDER BY id",
                &[&payout_id],
            )?
            .iter()
            .map(|r| {
                let from: Option<String> = r.try_get(0)?;
                let to: String = r.try_get(1)?;
                Ok(PayoutEvent {
                    from_state: match from {
                        Some(s) => {
                            Some(PayoutState::parse(&s).ok_or_else(|| corrupt("payout state"))?)
                        }
                        None => None,
                    },
                    to_state: PayoutState::parse(&to).ok_or_else(|| corrupt("payout state"))?,
                    occurred_at: r.try_get(2)?,
                    detail: r.try_get(3)?,
                })
            })
            .collect()
    }

    fn insert_offer(&mut self, o: &StreamOffer, link_hash: &str) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO stream_offers (id, organization_id, created_by, worker_name, total_zats,
                     start_at, end_at, link_hash, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                &[
                    &o.id,
                    &o.organization_id,
                    &o.created_by,
                    &o.worker_name,
                    &o.total_zats,
                    &o.start_at,
                    &o.end_at,
                    &link_hash,
                    &o.created_at,
                ],
            )
            .map_err(|e| unique_to_conflict(e, "worker link already in use"))?;
        Ok(())
    }

    fn load_offer(&mut self, id: &str, lock: bool) -> Result<Option<StreamOffer>> {
        self.offer_where("f.id = $1", id, lock)
    }

    fn offer_by_link(&mut self, link_hash: &str, lock: bool) -> Result<Option<StreamOffer>> {
        self.offer_where("f.link_hash = $1", link_hash, lock)
    }

    fn offer_by_stream(&mut self, stream_id: &str) -> Result<Option<StreamOffer>> {
        self.offer_where("f.stream_id = $1", stream_id, false)
    }

    fn offers_of_org(&mut self, org: &str) -> Result<Vec<StreamOffer>> {
        self.0
            .query(
                &format!(
                    "SELECT {OFFER_COLS} FROM stream_offers f
                     JOIN organizations o ON o.id = f.organization_id
                     WHERE f.organization_id = $1 ORDER BY f.created_at DESC, f.id"
                ),
                &[&org],
            )?
            .iter()
            .map(offer_from_row)
            .collect()
    }

    fn bind_offer_cas(&mut self, id: &str, stream_id: &str, user: &str, now: i64) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE stream_offers SET stream_id = $1, bound_user_id = $2, bound_at = $3
             WHERE id = $4 AND stream_id IS NULL AND cancelled_at IS NULL",
            &[&stream_id, &user, &now, &id],
        )? as usize)
    }

    fn cancel_offer_cas(&mut self, id: &str, now: i64) -> Result<usize> {
        Ok(self.0.execute(
            "UPDATE stream_offers SET cancelled_at = $1
             WHERE id = $2 AND stream_id IS NULL AND cancelled_at IS NULL",
            &[&now, &id],
        )? as usize)
    }

    fn relink_offer_cas(&mut self, id: &str, link_hash: &str) -> Result<usize> {
        Ok(self
            .0
            .execute(
                "UPDATE stream_offers SET link_hash = $1
                 WHERE id = $2 AND stream_id IS NULL AND cancelled_at IS NULL",
                &[&link_hash, &id],
            )
            .map_err(|e| unique_to_conflict(e, "worker link already in use"))? as usize)
    }

    fn insert_payout_address(&mut self, a: &PayoutAddress) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO stream_payout_addresses (stream_id, set_by, address_fingerprint,
                     address_sealed, set_at)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &a.stream_id,
                    &a.set_by,
                    &a.address_fingerprint,
                    &a.address_sealed,
                    &a.set_at,
                ],
            )
            .map_err(|e| {
                unique_to_conflict(e, "the payout address for this stream is already set")
            })?;
        Ok(())
    }

    fn payout_address(&mut self, stream_id: &str) -> Result<Option<PayoutAddress>> {
        match self.0.query_opt(
            "SELECT stream_id, set_by, address_fingerprint, address_sealed, set_at
             FROM stream_payout_addresses WHERE stream_id = $1",
            &[&stream_id],
        )? {
            Some(r) => Ok(Some(PayoutAddress {
                stream_id: r.try_get(0)?,
                set_by: r.try_get(1)?,
                address_fingerprint: r.try_get(2)?,
                address_sealed: r.try_get(3)?,
                set_at: r.try_get(4)?,
            })),
            None => Ok(None),
        }
    }
}

impl PgTx<'_> {
    fn offer_where(&mut self, cond: &str, arg: &str, lock: bool) -> Result<Option<StreamOffer>> {
        let sql = format!(
            "SELECT {OFFER_COLS} FROM stream_offers f
             JOIN organizations o ON o.id = f.organization_id WHERE {cond}{}",
            if lock { " FOR UPDATE OF f" } else { "" }
        );
        match self.0.query_opt(&sql, &[&arg])? {
            Some(r) => Ok(Some(offer_from_row(&r)?)),
            None => Ok(None),
        }
    }
}

const OFFER_COLS: &str = "f.id, f.organization_id, o.name, f.created_by, f.worker_name, f.total_zats,      f.start_at, f.end_at, f.created_at, f.stream_id, f.bound_user_id, f.bound_at, f.cancelled_at";

fn offer_from_row(r: &Row) -> Result<StreamOffer> {
    Ok(StreamOffer {
        id: r.try_get(0)?,
        organization_id: r.try_get(1)?,
        organization_name: r.try_get(2)?,
        created_by: r.try_get(3)?,
        worker_name: r.try_get(4)?,
        total_zats: r.try_get(5)?,
        start_at: r.try_get(6)?,
        end_at: r.try_get(7)?,
        created_at: r.try_get(8)?,
        stream_id: r.try_get(9)?,
        bound_user_id: r.try_get(10)?,
        bound_at: r.try_get(11)?,
        cancelled_at: r.try_get(12)?,
    })
}

const PAYOUT_COLS: &str = "id, claim_id, stream_id, requested_by, amount_zats, recipient_fingerprint, \
     fee_estimate_zats, max_fee_zats, state, txid, fee_zats, expiry_height, mined_height, \
     confirmations, detail, created_at, updated_at, submitted_at, resolved_at, lease_owner, lease_until";

fn payout_from_row(r: &Row) -> Result<Payout> {
    let state: String = r.try_get(8)?;
    Ok(Payout {
        id: r.try_get(0)?,
        claim_id: r.try_get(1)?,
        stream_id: r.try_get(2)?,
        requested_by: r.try_get(3)?,
        amount_zats: r.try_get(4)?,
        recipient_fingerprint: r.try_get(5)?,
        fee_estimate_zats: r.try_get(6)?,
        max_fee_zats: r.try_get(7)?,
        state: PayoutState::parse(&state).ok_or_else(|| corrupt("payout state"))?,
        txid: r.try_get(9)?,
        fee_zats: r.try_get(10)?,
        expiry_height: r.try_get(11)?,
        mined_height: r.try_get(12)?,
        confirmations: r.try_get(13)?,
        detail: r.try_get(14)?,
        created_at: r.try_get(15)?,
        updated_at: r.try_get(16)?,
        submitted_at: r.try_get(17)?,
        resolved_at: r.try_get(18)?,
        lease_owner: r.try_get(19)?,
        lease_until: r.try_get(20)?,
    })
}

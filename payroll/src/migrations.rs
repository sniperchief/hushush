//! Forward-only, versioned schema migrations. Each migration runs in its own
//! transaction and is recorded in `schema_migrations`; `PRAGMA user_version`
//! mirrors the latest applied version.

use rusqlite::{Connection, params};

use crate::error::PayrollError;

pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "0002_payouts",
        sql: include_str!("../migrations/0002_payouts.sql"),
    },
    Migration {
        version: 3,
        name: "0003_flow",
        sql: include_str!("../migrations/0003_flow.sql"),
    },
];

pub fn latest_version() -> i64 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

/// Applies all pending migrations. Returns the versions applied.
pub fn migrate(conn: &mut Connection, now: i64) -> Result<Vec<i64>, PayrollError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             name       TEXT    NOT NULL,
             applied_at INTEGER NOT NULL
         ) STRICT;",
    )?;
    let current: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |r| r.get(0),
    )?;
    if current > latest_version() {
        return Err(PayrollError::SchemaTooNew {
            found: current,
            supported: latest_version(),
        });
    }
    let mut applied = vec![];
    for m in MIGRATIONS.iter().filter(|m| m.version > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![m.version, m.name, now],
        )?;
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
        applied.push(m.version);
    }
    Ok(applied)
}

/// PostgreSQL migrations (`migrations_pg/`), mirroring [`MIGRATIONS`].
pub const PG_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_init",
        sql: include_str!("../migrations_pg/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "0002_payouts",
        sql: include_str!("../migrations_pg/0002_payouts.sql"),
    },
    Migration {
        version: 3,
        name: "0003_rls",
        sql: include_str!("../migrations_pg/0003_rls.sql"),
    },
    Migration {
        version: 4,
        name: "0004_flow",
        sql: include_str!("../migrations_pg/0004_flow.sql"),
    },
];

/// Applies pending PostgreSQL migrations in the client's current schema.
/// Concurrent migrators (several service instances starting at once) are
/// serialized with a transaction-scoped advisory lock.
pub fn migrate_pg(client: &mut postgres::Client, now: i64) -> Result<Vec<i64>, PayrollError> {
    let latest = PG_MIGRATIONS.last().map_or(0, |m| m.version);
    let mut tx = client.transaction()?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext('hushush_payroll_migrations'))",
        &[],
    )?;
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS payroll_schema_migrations (
             version    BIGINT PRIMARY KEY,
             name       TEXT   NOT NULL,
             applied_at BIGINT NOT NULL
         )",
    )?;
    let current: i64 = tx
        .query_one(
            "SELECT COALESCE(MAX(version), 0) FROM payroll_schema_migrations",
            &[],
        )?
        .get(0);
    if current > latest {
        return Err(PayrollError::SchemaTooNew {
            found: current,
            supported: latest,
        });
    }
    let mut applied = vec![];
    for m in PG_MIGRATIONS.iter().filter(|m| m.version > current) {
        tx.batch_execute(m.sql)?;
        tx.execute(
            "INSERT INTO payroll_schema_migrations (version, name, applied_at) VALUES ($1, $2, $3)",
            &[&m.version, &m.name, &now],
        )?;
        applied.push(m.version);
    }
    tx.commit()?;
    Ok(applied)
}

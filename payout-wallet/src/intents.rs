//! Persistent payment intents: the wallet side of payout idempotency.
//!
//! Every payment id gets exactly one row, written BEFORE any wallet work and
//! updated only forward:
//!
//! ```text
//! pending ──▶ built ──▶ broadcast
//!    ╰──────▶ failed            (no transaction exists, and none ever will)
//! ```
//!
//! A `failed` row with no address is a *fence*: the payroll side cancelled a
//! payment id the wallet had never seen, so a late request for it is refused.
//! The raw transaction is stored at `built`, before broadcast, so a crash can
//! never lose a transaction that might already be on the network.
//!
//! This database lives next to the wallet DB, outside the repository. It holds
//! destination addresses (needed to verify payments); it holds no keys.
//!
//! Phase 6 adds per-company payout accounts: `wallet_accounts` maps an opaque
//! account key (the payroll organization id) to its account in the wallet DB,
//! and each intent records which account pays it (`account`, NULL = the
//! treasury, account 0). `faucet_grants` logs testnet top-ups from the treasury.

use std::path::Path;

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    Pending,
    Failed,
    Built,
    Broadcast,
}

impl IntentState {
    pub fn as_str(self) -> &'static str {
        match self {
            IntentState::Pending => "pending",
            IntentState::Failed => "failed",
            IntentState::Built => "built",
            IntentState::Broadcast => "broadcast",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(IntentState::Pending),
            "failed" => Some(IntentState::Failed),
            "built" => Some(IntentState::Built),
            "broadcast" => Some(IntentState::Broadcast),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Intent {
    pub payment_id: String,
    /// Account key that pays this payment; `None` = the treasury.
    pub account: Option<String>,
    /// `None` only for fences (cancelled before the wallet saw a request).
    pub address: Option<String>,
    pub amount_zats: i64,
    pub max_fee_zats: i64,
    pub memo: String,
    pub state: IntentState,
    pub failure_reason: Option<String>,
    pub txid: Option<String>,
    pub raw_tx: Option<Vec<u8>>,
    pub fee_zats: Option<i64>,
    pub expiry_height: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub broadcast_attempts: i64,
    pub last_broadcast_error: Option<String>,
}

impl Intent {
    pub fn recipient_fingerprint(&self) -> String {
        self.address
            .as_deref()
            .map(|a| recipient_fingerprint(&self.payment_id, a))
            .unwrap_or_default()
    }
}

/// The memo of every payout transaction. It is encrypted to the recipient
/// (and recoverable by the wallet's own viewing key). It lets the recipient
/// match a payment to its payout, and lets this service find a transaction it
/// built if it crashed before recording it.
pub fn memo_for(payment_id: &str) -> String {
    format!("hushush payout {payment_id}")
}

/// Same definition as `hushush_payroll::payout::recipient_fingerprint`
/// (shared test vector below): salted per payment, so fingerprints in the
/// payroll database can't be linked to each other.
pub fn recipient_fingerprint(payment_id: &str, address: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"hushush-payout-recipient-v1\0");
    h.update(payment_id.as_bytes());
    h.update(b"\0");
    h.update(address.trim().as_bytes());
    hex::encode(h.finalize())
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS payment_intents (
    payment_id           TEXT    PRIMARY KEY CHECK (length(payment_id) = 36),
    address              TEXT,
    amount_zats          INTEGER NOT NULL CHECK (amount_zats >= 0),
    max_fee_zats         INTEGER NOT NULL CHECK (max_fee_zats >= 0),
    memo                 TEXT    NOT NULL,
    state                TEXT    NOT NULL CHECK (state IN ('pending', 'failed', 'built', 'broadcast')),
    failure_reason       TEXT,
    txid                 TEXT    CHECK (txid IS NULL OR length(txid) = 64),
    raw_tx               BLOB,
    fee_zats             INTEGER,
    expiry_height        INTEGER,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    broadcast_attempts   INTEGER NOT NULL DEFAULT 0,
    last_broadcast_error TEXT,
    CHECK ((state IN ('built', 'broadcast')) = (txid IS NOT NULL AND raw_tx IS NOT NULL)),
    CHECK ((state = 'failed') = (failure_reason IS NOT NULL)),
    CHECK (address IS NOT NULL OR state = 'failed')
) STRICT;

CREATE TRIGGER IF NOT EXISTS payment_intents_forward_only
BEFORE UPDATE OF state ON payment_intents
WHEN NEW.state <> OLD.state AND NOT (
       (OLD.state = 'pending' AND NEW.state IN ('failed', 'built'))
    OR (OLD.state = 'built' AND NEW.state = 'broadcast'))
BEGIN
    SELECT RAISE(ABORT, 'invalid payment intent transition');
END;

CREATE TRIGGER IF NOT EXISTS payment_intents_immutable
BEFORE UPDATE OF payment_id, address, amount_zats, max_fee_zats, memo, created_at ON payment_intents
BEGIN
    SELECT RAISE(ABORT, 'payment intent terms are immutable');
END;

CREATE TRIGGER IF NOT EXISTS payment_intents_tx_write_once
BEFORE UPDATE OF txid, raw_tx ON payment_intents
WHEN OLD.txid IS NOT NULL AND (NEW.txid IS NOT OLD.txid OR NEW.raw_tx IS NOT OLD.raw_tx)
BEGIN
    SELECT RAISE(ABORT, 'payment intent transaction is write-once');
END;

CREATE TRIGGER IF NOT EXISTS payment_intents_no_delete
BEFORE DELETE ON payment_intents
BEGIN
    SELECT RAISE(ABORT, 'payment intents cannot be deleted');
END;

CREATE TABLE IF NOT EXISTS wallet_accounts (
    key          TEXT    PRIMARY KEY CHECK (length(key) BETWEEN 1 AND 64),
    account_uuid TEXT    NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL
) STRICT;

CREATE TRIGGER IF NOT EXISTS wallet_accounts_immutable
BEFORE UPDATE ON wallet_accounts
BEGIN
    SELECT RAISE(ABORT, 'wallet accounts are immutable');
END;

CREATE TABLE IF NOT EXISTS faucet_grants (
    payment_id  TEXT    PRIMARY KEY CHECK (length(payment_id) = 36),
    key         TEXT    NOT NULL REFERENCES wallet_accounts (key),
    amount_zats INTEGER NOT NULL CHECK (amount_zats > 0),
    created_at  INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS faucet_grants_key ON faucet_grants (key, created_at);
";

const COLS: &str = "payment_id, address, amount_zats, max_fee_zats, memo, state, failure_reason, \
     txid, raw_tx, fee_zats, expiry_height, created_at, updated_at, broadcast_attempts, \
     last_broadcast_error, account";

fn from_row(r: &Row<'_>) -> rusqlite::Result<Intent> {
    let state: String = r.get(5)?;
    Ok(Intent {
        payment_id: r.get(0)?,
        address: r.get(1)?,
        amount_zats: r.get(2)?,
        max_fee_zats: r.get(3)?,
        memo: r.get(4)?,
        state: IntentState::parse(&state).ok_or(rusqlite::Error::InvalidColumnType(
            5,
            state,
            rusqlite::types::Type::Text,
        ))?,
        failure_reason: r.get(6)?,
        txid: r.get(7)?,
        raw_tx: r.get(8)?,
        fee_zats: r.get(9)?,
        expiry_height: r.get(10)?,
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
        broadcast_attempts: r.get(13)?,
        last_broadcast_error: r.get(14)?,
        account: r.get(15)?,
    })
}

fn truncate(s: &str) -> String {
    s.chars().take(500).collect()
}

pub struct IntentStore {
    conn: Connection,
}

impl IntentStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening intent store {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(SCHEMA)?;
        // Phase 6: which account pays each intent (older rows: NULL = treasury).
        let has_account = conn
            .prepare("SELECT 1 FROM pragma_table_info('payment_intents') WHERE name = 'account'")?
            .exists([])?;
        if !has_account {
            conn.execute_batch(
                "ALTER TABLE payment_intents ADD COLUMN account TEXT;
                 CREATE TRIGGER IF NOT EXISTS payment_intents_account_immutable
                 BEFORE UPDATE OF account ON payment_intents
                 BEGIN SELECT RAISE(ABORT, 'payment intent terms are immutable'); END;",
            )?;
        }
        Ok(Self { conn })
    }

    // ---- accounts and faucet grants -------------------------------------------------

    /// The wallet account (UUID string) registered for `key`.
    pub fn account_for(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT account_uuid FROM wallet_accounts WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn insert_account(&self, key: &str, account_uuid: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO wallet_accounts (key, account_uuid, created_at) VALUES (?1, ?2, ?3)",
            params![key, account_uuid, now()],
        )?;
        Ok(())
    }

    /// Creation times of `key`'s faucet grants that did not fail (a failed
    /// grant sent nothing, so it doesn't count against the limits).
    pub fn live_faucet_grants(&self, key: &str) -> anyhow::Result<Vec<i64>> {
        let mut q = self.conn.prepare(
            "SELECT g.created_at FROM faucet_grants g
             LEFT JOIN payment_intents i ON i.payment_id = g.payment_id
             WHERE g.key = ?1 AND (i.state IS NULL OR i.state <> 'failed')
             ORDER BY g.created_at",
        )?;
        let rows = q.query_map([key], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The account key a faucet payment id was granted to, if it is one.
    pub fn faucet_grant(&self, payment_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT key FROM faucet_grants WHERE payment_id = ?1",
                [payment_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn insert_faucet_grant(
        &self,
        payment_id: &str,
        key: &str,
        amount: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO faucet_grants (payment_id, key, amount_zats, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![payment_id, key, amount, now()],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> anyhow::Result<Option<Intent>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM payment_intents WHERE payment_id = ?1"),
                [id],
                from_row,
            )
            .optional()?)
    }

    pub fn in_state(&self, state: IntentState) -> anyhow::Result<Vec<Intent>> {
        let mut q = self.conn.prepare(&format!(
            "SELECT {COLS} FROM payment_intents WHERE state = ?1 ORDER BY created_at"
        ))?;
        let rows = q.query_map([state.as_str()], from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count_unfinished(&self) -> anyhow::Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM payment_intents WHERE state IN ('pending', 'built')",
            [],
            |r| r.get(0),
        )?)
    }

    /// Records a new payment the wallet is about to work on. Must be durable
    /// before any transaction is built.
    pub fn insert_pending(
        &self,
        id: &str,
        account: Option<&str>,
        address: &str,
        amount: i64,
        max_fee: i64,
        memo: &str,
    ) -> anyhow::Result<()> {
        let t = now();
        self.conn.execute(
            "INSERT INTO payment_intents (payment_id, address, amount_zats, max_fee_zats, memo,
                 state, created_at, updated_at, account)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?6, ?7)",
            params![id, address, amount, max_fee, memo, t, account],
        )?;
        Ok(())
    }

    /// Records a payment id as failed without ever building anything. With
    /// `address = None` this is a fence.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_failed(
        &self,
        id: &str,
        account: Option<&str>,
        address: Option<&str>,
        amount: i64,
        max_fee: i64,
        memo: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        let t = now();
        self.conn.execute(
            "INSERT INTO payment_intents (payment_id, address, amount_zats, max_fee_zats, memo,
                 state, failure_reason, created_at, updated_at, account)
             VALUES (?1, ?2, ?3, ?4, ?5, 'failed', ?6, ?7, ?7, ?8)",
            params![
                id,
                address,
                amount.max(0),
                max_fee.max(0),
                memo,
                truncate(reason),
                t,
                account
            ],
        )?;
        Ok(())
    }

    /// Records a payment id whose transaction already exists in the wallet DB
    /// but which has no intent row (the intent store was lost or restored
    /// from an older backup). The existing transaction is adopted; nothing is
    /// ever built for this id again.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_recovered(
        &self,
        id: &str,
        address: &str,
        amount: i64,
        memo: &str,
        txid: &str,
        raw: &[u8],
        fee: i64,
        expiry: Option<i64>,
    ) -> anyhow::Result<()> {
        let t = now();
        self.conn.execute(
            "INSERT INTO payment_intents (payment_id, address, amount_zats, max_fee_zats, memo,
                 state, txid, raw_tx, fee_zats, expiry_height, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'built', ?6, ?7, ?4, ?8, ?9, ?9)",
            params![id, address, amount, fee, memo, txid, raw, expiry, t],
        )?;
        Ok(())
    }

    pub fn mark_built(
        &self,
        id: &str,
        txid: &str,
        raw: &[u8],
        fee: Option<i64>,
        expiry: Option<i64>,
    ) -> anyhow::Result<()> {
        let n = self.conn.execute(
            "UPDATE payment_intents SET state = 'built', txid = ?2, raw_tx = ?3, fee_zats = ?4,
                 expiry_height = ?5, updated_at = ?6
             WHERE payment_id = ?1 AND state = 'pending'",
            params![id, txid, raw, fee, expiry, now()],
        )?;
        anyhow::ensure!(n == 1, "intent {id} was not pending");
        Ok(())
    }

    pub fn mark_failed(&self, id: &str, reason: &str) -> anyhow::Result<()> {
        let n = self.conn.execute(
            "UPDATE payment_intents SET state = 'failed', failure_reason = ?2, updated_at = ?3
             WHERE payment_id = ?1 AND state = 'pending'",
            params![id, truncate(reason), now()],
        )?;
        anyhow::ensure!(n == 1, "intent {id} was not pending");
        Ok(())
    }

    /// Records a broadcast attempt; `error = None` means the network accepted it.
    pub fn record_broadcast(&self, id: &str, error: Option<&str>) -> anyhow::Result<()> {
        let n = match error {
            None => self.conn.execute(
                "UPDATE payment_intents SET state = 'broadcast', broadcast_attempts = broadcast_attempts + 1,
                     last_broadcast_error = NULL, updated_at = ?2
                 WHERE payment_id = ?1 AND state IN ('built', 'broadcast')",
                params![id, now()],
            )?,
            Some(e) => self.conn.execute(
                "UPDATE payment_intents SET broadcast_attempts = broadcast_attempts + 1,
                     last_broadcast_error = ?2, updated_at = ?3
                 WHERE payment_id = ?1 AND state IN ('built', 'broadcast')",
                params![id, truncate(e), now()],
            )?,
        };
        anyhow::ensure!(n == 1, "intent {id} has no transaction");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "11111111-1111-1111-1111-111111111111";

    fn store() -> (tempfile::TempDir, IntentStore) {
        let d = tempfile::tempdir().unwrap();
        let s = IntentStore::open(&d.path().join("i.sqlite")).unwrap();
        (d, s)
    }

    #[test]
    fn fingerprint_matches_payroll_test_vector() {
        assert_eq!(
            recipient_fingerprint("00000000-0000-0000-0000-000000000000", "utest1example"),
            "9d5ce522bd2347c13521b7a002f9b114cd4ee5bdeb9321608dd9f038109e950f"
        );
    }

    #[test]
    fn lifecycle_pending_built_broadcast() {
        let (_d, s) = store();
        s.insert_pending(ID, None, "utest1x", 5, 10, &memo_for(ID))
            .unwrap();
        assert_eq!(s.count_unfinished().unwrap(), 1);
        s.mark_built(ID, &"a".repeat(64), b"raw", Some(10), Some(100))
            .unwrap();
        s.record_broadcast(ID, Some("connection reset")).unwrap();
        let i = s.get(ID).unwrap().unwrap();
        assert_eq!((i.state, i.broadcast_attempts), (IntentState::Built, 1));
        s.record_broadcast(ID, None).unwrap();
        let i = s.get(ID).unwrap().unwrap();
        assert_eq!((i.state, i.broadcast_attempts), (IntentState::Broadcast, 2));
        assert_eq!(i.raw_tx.as_deref(), Some(&b"raw"[..]));
        assert_eq!(s.count_unfinished().unwrap(), 0);
    }

    #[test]
    fn one_row_per_payment_id_and_forward_only() {
        let (_d, s) = store();
        s.insert_pending(ID, None, "utest1x", 5, 10, "m").unwrap();
        assert!(s.insert_pending(ID, None, "utest1x", 5, 10, "m").is_err());
        assert!(s.insert_failed(ID, None, None, 0, 0, "m", "fence").is_err());
        s.mark_failed(ID, "insufficient funds").unwrap();
        // Failed is final: it can never be built afterwards.
        assert!(
            s.mark_built(ID, &"a".repeat(64), b"raw", None, None)
                .is_err()
        );
        assert!(s.record_broadcast(ID, None).is_err());
        let raw = |sql: &str| s.conn.execute_batch(sql).unwrap_err().to_string();
        assert!(
            raw(&format!(
                "UPDATE payment_intents SET state = 'pending' WHERE payment_id = '{ID}'"
            ))
            .contains("transition")
        );
        assert!(
            raw(&format!(
                "UPDATE payment_intents SET amount_zats = 9 WHERE payment_id = '{ID}'"
            ))
            .contains("immutable")
        );
        assert!(
            raw(&format!(
                "DELETE FROM payment_intents WHERE payment_id = '{ID}'"
            ))
            .contains("cannot be deleted")
        );
    }

    #[test]
    fn fences_and_write_once_transactions() {
        let (_d, s) = store();
        s.insert_failed(ID, None, None, 0, 0, "m", "cancelled").unwrap();
        let i = s.get(ID).unwrap().unwrap();
        assert_eq!(
            (i.state, i.recipient_fingerprint()),
            (IntentState::Failed, String::new())
        );
        let id2 = "22222222-2222-2222-2222-222222222222";
        s.insert_pending(id2, None, "utest1x", 5, 10, "m").unwrap();
        s.mark_built(id2, &"a".repeat(64), b"raw", None, None)
            .unwrap();
        let e = s
            .conn
            .execute_batch(&format!(
                "UPDATE payment_intents SET txid = '{}' WHERE payment_id = '{id2}'",
                "b".repeat(64)
            ))
            .unwrap_err()
            .to_string();
        assert!(e.contains("write-once"), "{e}");
    }

    #[test]
    fn survives_reopen() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("i.sqlite");
        IntentStore::open(&p)
            .unwrap()
            .insert_pending(ID, None, "utest1x", 5, 10, "m")
            .unwrap();
        let s = IntentStore::open(&p).unwrap();
        assert_eq!(s.in_state(IntentState::Pending).unwrap().len(), 1);
    }

    #[test]
    fn accounts_are_registered_once_and_immutable() {
        let (_d, s) = store();
        assert_eq!(s.account_for("org-1").unwrap(), None);
        s.insert_account("org-1", "uuid-a").unwrap();
        assert_eq!(s.account_for("org-1").unwrap().as_deref(), Some("uuid-a"));
        assert!(s.insert_account("org-1", "uuid-b").is_err());
        assert!(s.insert_account("org-2", "uuid-a").is_err());
        let e = s
            .conn
            .execute_batch("UPDATE wallet_accounts SET account_uuid = 'x'")
            .unwrap_err()
            .to_string();
        assert!(e.contains("immutable"), "{e}");
    }

    #[test]
    fn intents_record_their_paying_account() {
        let (_d, s) = store();
        s.insert_pending(ID, Some("org-1"), "utest1x", 5, 10, "m")
            .unwrap();
        assert_eq!(
            s.get(ID).unwrap().unwrap().account.as_deref(),
            Some("org-1")
        );
        let id2 = "22222222-2222-2222-2222-222222222222";
        s.insert_pending(id2, None, "utest1x", 5, 10, "m").unwrap();
        assert_eq!(s.get(id2).unwrap().unwrap().account, None);
        let e = s
            .conn
            .execute_batch(&format!(
                "UPDATE payment_intents SET account = 'org-2' WHERE payment_id = '{ID}'"
            ))
            .unwrap_err()
            .to_string();
        assert!(e.contains("immutable"), "{e}");
    }

    #[test]
    fn an_older_intent_store_gains_the_account_column() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("i.sqlite");
        {
            // A Phase 3–5 store: the original table without `account`.
            let c = Connection::open(&p).unwrap();
            let old = SCHEMA.split("CREATE TABLE IF NOT EXISTS wallet_accounts").next().unwrap();
            c.execute_batch(old).unwrap();
            c.execute(
                "INSERT INTO payment_intents (payment_id, address, amount_zats, max_fee_zats, memo, state, created_at, updated_at)
                 VALUES (?1, 'utest1x', 5, 10, 'm', 'pending', 1, 1)",
                [ID],
            )
            .unwrap();
        }
        let s = IntentStore::open(&p).unwrap();
        assert_eq!(s.get(ID).unwrap().unwrap().account, None);
        IntentStore::open(&p).unwrap(); // reopening is a no-op
    }

    #[test]
    fn failed_faucet_grants_do_not_count() {
        let (_d, s) = store();
        s.insert_account("org-1", "uuid-a").unwrap();
        let a = "33333333-3333-3333-3333-333333333333";
        let b = "44444444-4444-4444-4444-444444444444";
        for id in [a, b] {
            s.insert_faucet_grant(id, "org-1", 100).unwrap();
            s.insert_pending(id, None, "utest1x", 100, 10, "m").unwrap();
        }
        assert_eq!(s.live_faucet_grants("org-1").unwrap().len(), 2);
        s.mark_failed(b, "insufficient funds").unwrap();
        assert_eq!(s.live_faucet_grants("org-1").unwrap().len(), 1);
        assert_eq!(s.faucet_grant(a).unwrap().as_deref(), Some("org-1"));
        assert_eq!(s.faucet_grant(ID).unwrap(), None);
    }
}

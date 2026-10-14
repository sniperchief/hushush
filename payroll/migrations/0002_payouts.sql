-- hushush payroll schema v2: shielded payouts of reserved claims.
--
--   reserved -> submitted -> confirmed            (claim -> settled)
--   reserved | submitted -> requires_reconciliation -> submitted | confirmed | failed
--   reserved | submitted -> failed                (claim -> released)
--
-- `confirmed` and `failed` are final. One payout per claim, ever: a failed
-- payout releases its claim and a new claim is needed to try again.
-- The destination address is NOT stored here; only a salted fingerprint.

CREATE TABLE payouts (
    id                    TEXT    PRIMARY KEY CHECK (length(id) = 36),
    claim_id              TEXT    NOT NULL UNIQUE REFERENCES claims (id),
    stream_id             TEXT    NOT NULL REFERENCES payroll_streams (id),
    requested_by          TEXT    NOT NULL REFERENCES users (id),
    amount_zats           INTEGER NOT NULL CHECK (amount_zats > 0 AND amount_zats <= 2100000000000000),
    recipient_fingerprint TEXT    NOT NULL CHECK (length(recipient_fingerprint) = 64),
    fee_estimate_zats     INTEGER NOT NULL CHECK (fee_estimate_zats >= 0),
    max_fee_zats          INTEGER NOT NULL CHECK (max_fee_zats >= fee_estimate_zats),
    state                 TEXT    NOT NULL CHECK (state IN ('reserved', 'submitted', 'confirmed', 'failed', 'requires_reconciliation')),
    txid                  TEXT    CHECK (txid IS NULL OR length(txid) = 64),
    fee_zats              INTEGER CHECK (fee_zats IS NULL OR fee_zats >= 0),
    expiry_height         INTEGER CHECK (expiry_height IS NULL OR expiry_height > 0),
    mined_height          INTEGER CHECK (mined_height IS NULL OR mined_height > 0),
    confirmations         INTEGER NOT NULL DEFAULT 0 CHECK (confirmations >= 0),
    detail                TEXT    CHECK (detail IS NULL OR length(detail) <= 500),
    lease_owner           TEXT,
    lease_until           INTEGER,
    created_at            INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at            INTEGER NOT NULL CHECK (updated_at >= created_at),
    submitted_at          INTEGER,
    resolved_at           INTEGER,
    CHECK (state NOT IN ('submitted', 'confirmed') OR txid IS NOT NULL),
    CHECK (state <> 'confirmed' OR (mined_height IS NOT NULL AND confirmations > 0)),
    CHECK ((state IN ('confirmed', 'failed')) = (resolved_at IS NOT NULL)),
    CHECK ((lease_owner IS NULL) = (lease_until IS NULL))
) STRICT;

CREATE INDEX payouts_stream ON payouts (stream_id);
CREATE INDEX payouts_due ON payouts (state, lease_until);

-- A payout is created reserved, for a reserved claim, by its claimant, for
-- exactly the claimed amount.
CREATE TRIGGER payouts_valid_insert
BEFORE INSERT ON payouts
WHEN NEW.state <> 'reserved'
  OR NOT EXISTS (
        SELECT 1 FROM claims c
        WHERE c.id = NEW.claim_id AND c.status = 'reserved' AND c.stream_id = NEW.stream_id
          AND c.requested_by = NEW.requested_by AND c.amount_zats = NEW.amount_zats)
BEGIN
    SELECT RAISE(ABORT, 'payout must be created reserved, for a reserved claim, by its claimant, for its amount');
END;

CREATE TRIGGER payouts_immutable_terms
BEFORE UPDATE OF id, claim_id, stream_id, requested_by, amount_zats, recipient_fingerprint,
                 fee_estimate_zats, max_fee_zats, created_at
ON payouts
BEGIN
    SELECT RAISE(ABORT, 'payout terms are immutable');
END;

-- Once a transaction is associated with a payout it can never be replaced.
CREATE TRIGGER payouts_txid_write_once
BEFORE UPDATE OF txid ON payouts
WHEN OLD.txid IS NOT NULL AND NEW.txid IS NOT OLD.txid
BEGIN
    SELECT RAISE(ABORT, 'payout txid is write-once');
END;

CREATE TRIGGER payouts_final
BEFORE UPDATE ON payouts
WHEN OLD.state IN ('confirmed', 'failed')
BEGIN
    SELECT RAISE(ABORT, 'payout is final');
END;

CREATE TRIGGER payouts_valid_transition
BEFORE UPDATE OF state ON payouts
WHEN NEW.state <> OLD.state AND NOT (
       (OLD.state = 'reserved' AND NEW.state IN ('submitted', 'failed', 'requires_reconciliation'))
    OR (OLD.state = 'submitted' AND NEW.state IN ('confirmed', 'failed', 'requires_reconciliation'))
    OR (OLD.state = 'requires_reconciliation' AND NEW.state IN ('submitted', 'confirmed', 'failed')))
BEGIN
    SELECT RAISE(ABORT, 'invalid payout state transition');
END;

CREATE TRIGGER payouts_no_delete
BEFORE DELETE ON payouts
BEGIN
    SELECT RAISE(ABORT, 'payouts cannot be deleted');
END;

-- A claim with a payout can only be resolved by that payout: settled with its
-- txid once confirmed, or released once it has failed.
CREATE TRIGGER claims_resolved_by_payout
BEFORE UPDATE OF status ON claims
WHEN EXISTS (
        SELECT 1 FROM payouts p WHERE p.claim_id = OLD.id AND NOT (
               (NEW.status = 'settled' AND p.state = 'confirmed' AND NEW.settlement_ref = p.txid)
            OR (NEW.status = 'released' AND p.state = 'failed')))
BEGIN
    SELECT RAISE(ABORT, 'claim has a payout; only its confirmation or failure can resolve it');
END;

-- Append-only audit log of payout state changes.
CREATE TABLE payout_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    payout_id   TEXT    NOT NULL REFERENCES payouts (id),
    from_state  TEXT    CHECK (from_state IS NULL OR from_state IN ('reserved', 'submitted', 'confirmed', 'failed', 'requires_reconciliation')),
    to_state    TEXT    NOT NULL CHECK (to_state IN ('reserved', 'submitted', 'confirmed', 'failed', 'requires_reconciliation')),
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    detail      TEXT    CHECK (detail IS NULL OR length(detail) <= 500)
) STRICT;

CREATE INDEX payout_events_payout ON payout_events (payout_id, id);

CREATE TRIGGER payout_events_no_update
BEFORE UPDATE ON payout_events
BEGIN
    SELECT RAISE(ABORT, 'payout_events is append-only');
END;

CREATE TRIGGER payout_events_no_delete
BEFORE DELETE ON payout_events
BEGIN
    SELECT RAISE(ABORT, 'payout_events is append-only');
END;

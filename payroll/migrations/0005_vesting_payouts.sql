-- hushush payroll schema v5: vesting claims are paid through the existing
-- shielded payout state machine.
--
-- * `payouts` gains an explicit `compensation_type` and a vesting reference
--   (`vesting_claim_id`, `vesting_agreement_id`) next to the stream reference
--   (`claim_id`, `stream_id`). Exactly one reference is set, matching the type.
--   Each reference is UNIQUE per claim: one payout per claim, ever.
-- * A vesting claim with a payout is resolved only by that payout (settled
--   with its txid once confirmed, released once it has failed).
-- * An agreement's payout address is set once by its employee and locked.
-- * An agreement may only be activated at or before its start (no
--   retroactive vesting).
--
-- SQLite cannot relax the NOT NULL on `payouts.claim_id`/`stream_id`, so
-- `payouts` (and `payout_events`, which references it) are rebuilt with every
-- row copied. The trigger on `claims` that reads `payouts` is recreated around
-- the rebuild. The whole migration runs in one transaction.

DROP TRIGGER claims_resolved_by_payout;

CREATE TABLE payouts_v5 (
    id                    TEXT    PRIMARY KEY CHECK (length(id) = 36),
    compensation_type     TEXT    NOT NULL DEFAULT 'stream' CHECK (compensation_type IN ('stream', 'vesting')),
    claim_id              TEXT    UNIQUE REFERENCES claims (id),
    stream_id             TEXT    REFERENCES payroll_streams (id),
    vesting_claim_id      TEXT    UNIQUE REFERENCES vesting_claims (id),
    vesting_agreement_id  TEXT    REFERENCES vesting_agreements (id),
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
    -- Exactly one claim reference, of the declared type.
    CHECK ((compensation_type = 'stream'
            AND claim_id IS NOT NULL AND stream_id IS NOT NULL
            AND vesting_claim_id IS NULL AND vesting_agreement_id IS NULL)
        OR (compensation_type = 'vesting'
            AND claim_id IS NULL AND stream_id IS NULL
            AND vesting_claim_id IS NOT NULL AND vesting_agreement_id IS NOT NULL)),
    CHECK (state NOT IN ('submitted', 'confirmed') OR txid IS NOT NULL),
    CHECK (state <> 'confirmed' OR (mined_height IS NOT NULL AND confirmations > 0)),
    CHECK ((state IN ('confirmed', 'failed')) = (resolved_at IS NOT NULL)),
    CHECK ((lease_owner IS NULL) = (lease_until IS NULL))
) STRICT;

INSERT INTO payouts_v5 (id, compensation_type, claim_id, stream_id, requested_by, amount_zats,
        recipient_fingerprint, fee_estimate_zats, max_fee_zats, state, txid, fee_zats,
        expiry_height, mined_height, confirmations, detail, lease_owner, lease_until,
        created_at, updated_at, submitted_at, resolved_at)
    SELECT id, 'stream', claim_id, stream_id, requested_by, amount_zats,
        recipient_fingerprint, fee_estimate_zats, max_fee_zats, state, txid, fee_zats,
        expiry_height, mined_height, confirmations, detail, lease_owner, lease_until,
        created_at, updated_at, submitted_at, resolved_at
    FROM payouts;

CREATE TABLE payout_events_v5 (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Renaming payouts_v5 to payouts below rewrites this reference.
    payout_id   TEXT    NOT NULL REFERENCES payouts_v5 (id),
    from_state  TEXT    CHECK (from_state IS NULL OR from_state IN ('reserved', 'submitted', 'confirmed', 'failed', 'requires_reconciliation')),
    to_state    TEXT    NOT NULL CHECK (to_state IN ('reserved', 'submitted', 'confirmed', 'failed', 'requires_reconciliation')),
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    detail      TEXT    CHECK (detail IS NULL OR length(detail) <= 500)
) STRICT;

INSERT INTO payout_events_v5 (id, payout_id, from_state, to_state, occurred_at, detail)
    SELECT id, payout_id, from_state, to_state, occurred_at, detail FROM payout_events ORDER BY id;

-- Children first. Dropping a table runs no triggers.
DROP TABLE payout_events;
DROP TABLE payouts;
ALTER TABLE payouts_v5 RENAME TO payouts;
ALTER TABLE payout_events_v5 RENAME TO payout_events;

CREATE INDEX payouts_stream ON payouts (stream_id);
CREATE INDEX payouts_vesting_agreement ON payouts (vesting_agreement_id);
CREATE INDEX payouts_due ON payouts (state, lease_until);
CREATE INDEX payout_events_payout ON payout_events (payout_id, id);

-- A payout is created reserved, for a reserved claim of its type, by that
-- claim's claimant, for exactly the claimed amount, from the claim's source.
CREATE TRIGGER payouts_valid_insert
BEFORE INSERT ON payouts
WHEN NEW.state <> 'reserved'
  OR (NEW.compensation_type = 'stream' AND NOT EXISTS (
        SELECT 1 FROM claims c
        WHERE c.id = NEW.claim_id AND c.status = 'reserved' AND c.stream_id = NEW.stream_id
          AND c.requested_by = NEW.requested_by AND c.amount_zats = NEW.amount_zats))
  OR (NEW.compensation_type = 'vesting' AND NOT EXISTS (
        SELECT 1 FROM vesting_claims c
        WHERE c.id = NEW.vesting_claim_id AND c.status = 'reserved'
          AND c.agreement_id = NEW.vesting_agreement_id
          AND c.requested_by = NEW.requested_by AND c.amount_zats = NEW.amount_zats))
BEGIN
    SELECT RAISE(ABORT, 'payout must be created reserved, for a reserved claim, by its claimant, for its amount');
END;

CREATE TRIGGER payouts_immutable_terms
BEFORE UPDATE OF id, compensation_type, claim_id, stream_id, vesting_claim_id, vesting_agreement_id,
                 requested_by, amount_zats, recipient_fingerprint, fee_estimate_zats, max_fee_zats,
                 created_at
ON payouts
BEGIN
    SELECT RAISE(ABORT, 'payout terms are immutable');
END;

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

-- A claim with a payout can only be resolved by that payout: settled with its
-- txid once confirmed, or released once it has failed. (Recreated as in v2.)
CREATE TRIGGER claims_resolved_by_payout
BEFORE UPDATE OF status ON claims
WHEN EXISTS (
        SELECT 1 FROM payouts p WHERE p.claim_id = OLD.id AND NOT (
               (NEW.status = 'settled' AND p.state = 'confirmed' AND NEW.settlement_ref = p.txid)
            OR (NEW.status = 'released' AND p.state = 'failed')))
BEGIN
    SELECT RAISE(ABORT, 'claim has a payout; only its confirmation or failure can resolve it');
END;

-- The same rule for vesting claims.
CREATE TRIGGER vesting_claims_resolved_by_payout
BEFORE UPDATE OF status ON vesting_claims
WHEN EXISTS (
        SELECT 1 FROM payouts p WHERE p.vesting_claim_id = OLD.id AND NOT (
               (NEW.status = 'settled' AND p.state = 'confirmed' AND NEW.settlement_ref = p.txid)
            OR (NEW.status = 'released' AND p.state = 'failed')))
BEGIN
    SELECT RAISE(ABORT, 'claim has a payout; only its confirmation or failure can resolve it');
END;

-- No retroactive vesting: activation happens at or before the start.
CREATE TRIGGER vesting_agreements_activate_before_start
BEFORE UPDATE OF activated_at ON vesting_agreements
WHEN NEW.activated_at IS NOT NULL AND NEW.activated_at > NEW.start_at
BEGIN
    SELECT RAISE(ABORT, 'an agreement must be activated before its start');
END;

-- An agreement's payout address: set once by its employee, then locked.
-- Stored sealed by payroll-api (the database never sees the key), next to a
-- salted fingerprint used for equality checks.
CREATE TABLE vesting_payout_addresses (
    agreement_id        TEXT    PRIMARY KEY REFERENCES vesting_agreements (id),
    set_by              TEXT    NOT NULL REFERENCES users (id),
    address_fingerprint TEXT    NOT NULL CHECK (length(address_fingerprint) = 64),
    address_sealed      TEXT    NOT NULL CHECK (length(address_sealed) BETWEEN 1 AND 4000),
    set_at              INTEGER NOT NULL CHECK (set_at >= 0)
) STRICT;

CREATE TRIGGER vesting_payout_addresses_employee
BEFORE INSERT ON vesting_payout_addresses
WHEN NOT EXISTS (
        SELECT 1 FROM vesting_agreements a
        WHERE a.id = NEW.agreement_id AND a.employee_user_id = NEW.set_by AND a.status <> 'draft')
BEGIN
    SELECT RAISE(ABORT, 'only the employee of an activated agreement can set its payout address');
END;

CREATE TRIGGER vesting_payout_addresses_locked
BEFORE UPDATE ON vesting_payout_addresses
BEGIN
    SELECT RAISE(ABORT, 'payout address is locked for the life of the agreement');
END;

CREATE TRIGGER vesting_payout_addresses_no_delete
BEFORE DELETE ON vesting_payout_addresses
BEGIN
    SELECT RAISE(ABORT, 'payout address is locked for the life of the agreement');
END;

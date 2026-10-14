-- hushush payroll schema v2 (PostgreSQL): shielded payouts of reserved claims.
-- Mirrors migrations/0002_payouts.sql (SQLite) rule for rule.

CREATE TABLE payouts (
    id                    TEXT   PRIMARY KEY CHECK (length(id) = 36),
    claim_id              TEXT   NOT NULL UNIQUE REFERENCES claims (id),
    stream_id             TEXT   NOT NULL REFERENCES payroll_streams (id),
    requested_by          TEXT   NOT NULL REFERENCES users (id),
    amount_zats           BIGINT NOT NULL CHECK (amount_zats > 0 AND amount_zats <= 2100000000000000),
    recipient_fingerprint TEXT   NOT NULL CHECK (length(recipient_fingerprint) = 64),
    fee_estimate_zats     BIGINT NOT NULL CHECK (fee_estimate_zats >= 0),
    max_fee_zats          BIGINT NOT NULL CHECK (max_fee_zats >= fee_estimate_zats),
    state                 TEXT   NOT NULL CHECK (state IN ('reserved', 'submitted', 'confirmed', 'failed', 'requires_reconciliation')),
    txid                  TEXT   CHECK (txid IS NULL OR length(txid) = 64),
    fee_zats              BIGINT CHECK (fee_zats IS NULL OR fee_zats >= 0),
    expiry_height         BIGINT CHECK (expiry_height IS NULL OR expiry_height > 0),
    mined_height          BIGINT CHECK (mined_height IS NULL OR mined_height > 0),
    confirmations         BIGINT NOT NULL DEFAULT 0 CHECK (confirmations >= 0),
    detail                TEXT   CHECK (detail IS NULL OR length(detail) <= 500),
    lease_owner           TEXT,
    lease_until           BIGINT,
    created_at            BIGINT NOT NULL CHECK (created_at >= 0),
    updated_at            BIGINT NOT NULL CHECK (updated_at >= created_at),
    submitted_at          BIGINT,
    resolved_at           BIGINT,
    CHECK (state NOT IN ('submitted', 'confirmed') OR txid IS NOT NULL),
    CHECK (state <> 'confirmed' OR (mined_height IS NOT NULL AND confirmations > 0)),
    CHECK ((state IN ('confirmed', 'failed')) = (resolved_at IS NOT NULL)),
    CHECK ((lease_owner IS NULL) = (lease_until IS NULL))
);
CREATE INDEX payouts_stream ON payouts (stream_id);
CREATE INDEX payouts_due ON payouts (state, lease_until);

CREATE FUNCTION payouts_guard_insert() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.state <> 'reserved' OR NOT EXISTS (
           SELECT 1 FROM claims c
           WHERE c.id = NEW.claim_id AND c.status = 'reserved' AND c.stream_id = NEW.stream_id
             AND c.requested_by = NEW.requested_by AND c.amount_zats = NEW.amount_zats) THEN
        RAISE EXCEPTION 'payout must be created reserved, for a reserved claim, by its claimant, for its amount';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER payouts_guard_insert
BEFORE INSERT ON payouts
FOR EACH ROW EXECUTE FUNCTION payouts_guard_insert();

CREATE FUNCTION payouts_guard_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'payouts cannot be deleted';
    END IF;
    IF OLD.state IN ('confirmed', 'failed') THEN
        RAISE EXCEPTION 'payout is final';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.claim_id IS DISTINCT FROM OLD.claim_id
       OR NEW.stream_id IS DISTINCT FROM OLD.stream_id
       OR NEW.requested_by IS DISTINCT FROM OLD.requested_by
       OR NEW.amount_zats IS DISTINCT FROM OLD.amount_zats
       OR NEW.recipient_fingerprint IS DISTINCT FROM OLD.recipient_fingerprint
       OR NEW.fee_estimate_zats IS DISTINCT FROM OLD.fee_estimate_zats
       OR NEW.max_fee_zats IS DISTINCT FROM OLD.max_fee_zats
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'payout terms are immutable';
    END IF;
    IF OLD.txid IS NOT NULL AND NEW.txid IS DISTINCT FROM OLD.txid THEN
        RAISE EXCEPTION 'payout txid is write-once';
    END IF;
    IF NEW.state IS DISTINCT FROM OLD.state AND NOT (
           (OLD.state = 'reserved' AND NEW.state IN ('submitted', 'failed', 'requires_reconciliation'))
        OR (OLD.state = 'submitted' AND NEW.state IN ('confirmed', 'failed', 'requires_reconciliation'))
        OR (OLD.state = 'requires_reconciliation' AND NEW.state IN ('submitted', 'confirmed', 'failed'))) THEN
        RAISE EXCEPTION 'invalid payout state transition';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER payouts_guard_change
BEFORE UPDATE OR DELETE ON payouts
FOR EACH ROW EXECUTE FUNCTION payouts_guard_change();

-- A claim with a payout can only be resolved by that payout.
CREATE FUNCTION claims_resolved_by_payout() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.status IS DISTINCT FROM OLD.status AND EXISTS (
           SELECT 1 FROM payouts p WHERE p.claim_id = OLD.id AND NOT (
                  (NEW.status = 'settled' AND p.state = 'confirmed' AND NEW.settlement_ref = p.txid)
               OR (NEW.status = 'released' AND p.state = 'failed'))) THEN
        RAISE EXCEPTION 'claim has a payout; only its confirmation or failure can resolve it';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER claims_resolved_by_payout
BEFORE UPDATE ON claims
FOR EACH ROW EXECUTE FUNCTION claims_resolved_by_payout();

CREATE TABLE payout_events (
    id          BIGSERIAL PRIMARY KEY,
    payout_id   TEXT   NOT NULL REFERENCES payouts (id),
    from_state  TEXT   CHECK (from_state IS NULL OR from_state IN ('reserved', 'submitted', 'confirmed', 'failed', 'requires_reconciliation')),
    to_state    TEXT   NOT NULL CHECK (to_state IN ('reserved', 'submitted', 'confirmed', 'failed', 'requires_reconciliation')),
    occurred_at BIGINT NOT NULL CHECK (occurred_at >= 0),
    detail      TEXT   CHECK (detail IS NULL OR length(detail) <= 500)
);
CREATE INDEX payout_events_payout ON payout_events (payout_id, id);

CREATE TRIGGER payout_events_append_only
BEFORE UPDATE OR DELETE ON payout_events
FOR EACH ROW EXECUTE FUNCTION append_only();

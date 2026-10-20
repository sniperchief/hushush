-- hushush payroll schema v6 — PostgreSQL edition of migrations/0005_vesting_payouts.sql
-- (vesting claims paid through the existing payout state machine). See that
-- file for the model; rules here are identical. PostgreSQL alters `payouts`
-- in place: every existing row is a stream payout.

ALTER TABLE payouts
    ADD COLUMN compensation_type    TEXT NOT NULL DEFAULT 'stream' CHECK (compensation_type IN ('stream', 'vesting')),
    ADD COLUMN vesting_claim_id     TEXT UNIQUE REFERENCES vesting_claims (id),
    ADD COLUMN vesting_agreement_id TEXT REFERENCES vesting_agreements (id),
    ALTER COLUMN claim_id DROP NOT NULL,
    ALTER COLUMN stream_id DROP NOT NULL,
    ADD CONSTRAINT payouts_one_claim_reference CHECK (
           (compensation_type = 'stream'
            AND claim_id IS NOT NULL AND stream_id IS NOT NULL
            AND vesting_claim_id IS NULL AND vesting_agreement_id IS NULL)
        OR (compensation_type = 'vesting'
            AND claim_id IS NULL AND stream_id IS NULL
            AND vesting_claim_id IS NOT NULL AND vesting_agreement_id IS NOT NULL));
CREATE INDEX payouts_vesting_agreement ON payouts (vesting_agreement_id);

CREATE OR REPLACE FUNCTION payouts_guard_insert() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.state <> 'reserved'
       OR (NEW.compensation_type = 'stream' AND NOT EXISTS (
           SELECT 1 FROM claims c
           WHERE c.id = NEW.claim_id AND c.status = 'reserved' AND c.stream_id = NEW.stream_id
             AND c.requested_by = NEW.requested_by AND c.amount_zats = NEW.amount_zats))
       OR (NEW.compensation_type = 'vesting' AND NOT EXISTS (
           SELECT 1 FROM vesting_claims c
           WHERE c.id = NEW.vesting_claim_id AND c.status = 'reserved'
             AND c.agreement_id = NEW.vesting_agreement_id
             AND c.requested_by = NEW.requested_by AND c.amount_zats = NEW.amount_zats)) THEN
        RAISE EXCEPTION 'payout must be created reserved, for a reserved claim, by its claimant, for its amount';
    END IF;
    RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION payouts_guard_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'payouts cannot be deleted';
    END IF;
    IF OLD.state IN ('confirmed', 'failed') THEN
        RAISE EXCEPTION 'payout is final';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.compensation_type IS DISTINCT FROM OLD.compensation_type
       OR NEW.claim_id IS DISTINCT FROM OLD.claim_id
       OR NEW.stream_id IS DISTINCT FROM OLD.stream_id
       OR NEW.vesting_claim_id IS DISTINCT FROM OLD.vesting_claim_id
       OR NEW.vesting_agreement_id IS DISTINCT FROM OLD.vesting_agreement_id
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

-- A vesting claim with a payout can only be resolved by that payout.
CREATE FUNCTION vesting_claims_resolved_by_payout() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.status IS DISTINCT FROM OLD.status AND EXISTS (
           SELECT 1 FROM payouts p WHERE p.vesting_claim_id = OLD.id AND NOT (
                  (NEW.status = 'settled' AND p.state = 'confirmed' AND NEW.settlement_ref = p.txid)
               OR (NEW.status = 'released' AND p.state = 'failed'))) THEN
        RAISE EXCEPTION 'claim has a payout; only its confirmation or failure can resolve it';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER vesting_claims_resolved_by_payout
BEFORE UPDATE ON vesting_claims
FOR EACH ROW EXECUTE FUNCTION vesting_claims_resolved_by_payout();

-- No retroactive vesting: activation happens at or before the start.
CREATE FUNCTION vesting_agreements_activate_before_start() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.activated_at IS NOT NULL AND NEW.activated_at IS DISTINCT FROM OLD.activated_at
       AND NEW.activated_at > NEW.start_at THEN
        RAISE EXCEPTION 'an agreement must be activated before its start';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER vesting_agreements_activate_before_start
BEFORE UPDATE ON vesting_agreements
FOR EACH ROW EXECUTE FUNCTION vesting_agreements_activate_before_start();

CREATE TABLE vesting_payout_addresses (
    agreement_id        TEXT   PRIMARY KEY REFERENCES vesting_agreements (id),
    set_by              TEXT   NOT NULL REFERENCES users (id),
    address_fingerprint TEXT   NOT NULL CHECK (length(address_fingerprint) = 64),
    address_sealed      TEXT   NOT NULL CHECK (length(address_sealed) BETWEEN 1 AND 4000),
    set_at              BIGINT NOT NULL CHECK (set_at >= 0)
);

CREATE FUNCTION vesting_payout_addresses_insert() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM vesting_agreements a
                   WHERE a.id = NEW.agreement_id AND a.employee_user_id = NEW.set_by
                     AND a.status <> 'draft') THEN
        RAISE EXCEPTION 'only the employee of an activated agreement can set its payout address';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER vesting_payout_addresses_insert
BEFORE INSERT ON vesting_payout_addresses
FOR EACH ROW EXECUTE FUNCTION vesting_payout_addresses_insert();

CREATE FUNCTION vesting_payout_addresses_locked() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'payout address is locked for the life of the agreement';
END $$;

CREATE TRIGGER vesting_payout_addresses_locked
BEFORE UPDATE OR DELETE ON vesting_payout_addresses
FOR EACH ROW EXECUTE FUNCTION vesting_payout_addresses_locked();

-- Same row-level security as 0003_rls.sql.
ALTER TABLE vesting_payout_addresses ENABLE ROW LEVEL SECURITY;

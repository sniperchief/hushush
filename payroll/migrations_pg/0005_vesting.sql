-- hushush payroll schema v5 — PostgreSQL edition of migrations/0004_vesting.sql
-- (vesting compensation). See that file for the model; rules here are identical.

CREATE TABLE vesting_agreements (
    id                    TEXT   PRIMARY KEY CHECK (length(id) = 36),
    organization_id       TEXT   NOT NULL REFERENCES organizations (id),
    employee_user_id      TEXT   NOT NULL,
    created_by            TEXT   NOT NULL REFERENCES users (id),
    total_zats            BIGINT NOT NULL CHECK (total_zats > 0 AND total_zats <= 2100000000000000),
    start_at              BIGINT NOT NULL CHECK (start_at >= 0),
    duration_secs         BIGINT NOT NULL CHECK (duration_secs > 0 AND duration_secs <= 3153600000),
    cliff_secs            BIGINT NOT NULL DEFAULT 0 CHECK (cliff_secs >= 0 AND cliff_secs <= duration_secs),
    method                TEXT   NOT NULL CHECK (method IN ('linear')),
    status                TEXT   NOT NULL CHECK (status IN ('draft', 'active', 'cancelled')),
    created_at            BIGINT NOT NULL CHECK (created_at >= 0),
    updated_at            BIGINT NOT NULL CHECK (updated_at >= created_at),
    activated_at          BIGINT CHECK (activated_at IS NULL OR activated_at >= created_at),
    activated_by          TEXT   REFERENCES users (id),
    cancelled_at          BIGINT CHECK (cancelled_at IS NULL OR cancelled_at >= created_at),
    cancelled_by          TEXT   REFERENCES users (id),
    vested_at_cancel_zats BIGINT CHECK (vested_at_cancel_zats IS NULL
                                        OR (vested_at_cancel_zats >= 0 AND vested_at_cancel_zats <= total_zats)),
    CHECK (start_at >= created_at),
    CHECK (employee_user_id <> created_by),
    CHECK ((activated_at IS NULL) = (activated_by IS NULL)),
    CHECK ((status = 'draft') = (activated_at IS NULL AND cancelled_at IS NULL)),
    CHECK ((status = 'cancelled') = (cancelled_at IS NOT NULL)),
    CHECK ((cancelled_at IS NULL) = (cancelled_by IS NULL)),
    CHECK ((cancelled_at IS NULL) = (vested_at_cancel_zats IS NULL)),
    CHECK (activated_at IS NOT NULL OR COALESCE(vested_at_cancel_zats, 0) = 0),
    FOREIGN KEY (organization_id, employee_user_id)
        REFERENCES organization_members (organization_id, user_id)
);
CREATE INDEX vesting_agreements_org ON vesting_agreements (organization_id);
CREATE INDEX vesting_agreements_employee ON vesting_agreements (employee_user_id);

CREATE TABLE vesting_events (
    id                BIGSERIAL PRIMARY KEY,
    agreement_id      TEXT   NOT NULL REFERENCES vesting_agreements (id),
    kind              TEXT   NOT NULL CHECK (kind IN ('created', 'activated', 'cancelled')),
    actor_user_id     TEXT   NOT NULL REFERENCES users (id),
    occurred_at       BIGINT NOT NULL CHECK (occurred_at >= 0),
    vested_zats_after BIGINT NOT NULL CHECK (vested_zats_after >= 0)
);
CREATE INDEX vesting_events_agreement ON vesting_events (agreement_id, id);

CREATE TRIGGER vesting_events_append_only
BEFORE UPDATE OR DELETE ON vesting_events
FOR EACH ROW EXECUTE FUNCTION append_only();

CREATE TABLE vesting_claims (
    id                         TEXT   PRIMARY KEY CHECK (length(id) = 36),
    agreement_id               TEXT   NOT NULL REFERENCES vesting_agreements (id),
    requested_by               TEXT   NOT NULL REFERENCES users (id),
    idempotency_key            TEXT   NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    amount_zats                BIGINT NOT NULL CHECK (amount_zats > 0 AND amount_zats <= 2100000000000000),
    status                     TEXT   NOT NULL CHECK (status IN ('reserved', 'released', 'settled')),
    vested_at_reservation_zats BIGINT NOT NULL CHECK (vested_at_reservation_zats >= amount_zats),
    created_at                 BIGINT NOT NULL CHECK (created_at >= 0),
    resolved_at                BIGINT CHECK (resolved_at IS NULL OR resolved_at >= created_at),
    resolved_by                TEXT   REFERENCES users (id),
    settlement_ref             TEXT   CHECK (settlement_ref IS NULL OR length(settlement_ref) BETWEEN 1 AND 256),
    CHECK ((status = 'reserved') = (resolved_at IS NULL)),
    CHECK ((status = 'reserved') = (resolved_by IS NULL)),
    CHECK ((status = 'settled') = (settlement_ref IS NOT NULL)),
    UNIQUE (agreement_id, idempotency_key)
);
CREATE INDEX vesting_claims_agreement_status ON vesting_claims (agreement_id, status);

CREATE FUNCTION vesting_agreements_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'vesting agreements cannot be deleted';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.status <> 'draft' THEN
            RAISE EXCEPTION 'vesting agreements must be created as drafts';
        END IF;
        RETURN NEW;
    END IF;
    IF OLD.status = 'cancelled' THEN
        RAISE EXCEPTION 'cancelled vesting agreements are final';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
       OR NEW.employee_user_id IS DISTINCT FROM OLD.employee_user_id
       OR NEW.created_by IS DISTINCT FROM OLD.created_by
       OR NEW.total_zats IS DISTINCT FROM OLD.total_zats
       OR NEW.start_at IS DISTINCT FROM OLD.start_at
       OR NEW.duration_secs IS DISTINCT FROM OLD.duration_secs
       OR NEW.cliff_secs IS DISTINCT FROM OLD.cliff_secs
       OR NEW.method IS DISTINCT FROM OLD.method
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'vesting agreement terms are immutable';
    END IF;
    IF NEW.updated_at < OLD.updated_at
       OR (OLD.activated_at IS NOT NULL AND (NEW.activated_at IS DISTINCT FROM OLD.activated_at
                                             OR NEW.activated_by IS DISTINCT FROM OLD.activated_by))
       OR (NEW.status IS DISTINCT FROM OLD.status AND NOT (
              (OLD.status = 'draft' AND NEW.status IN ('active', 'cancelled'))
           OR (OLD.status = 'active' AND NEW.status = 'cancelled'))) THEN
        RAISE EXCEPTION 'invalid vesting agreement transition';
    END IF;
    IF NEW.status = 'cancelled' AND NEW.vested_at_cancel_zats < (
           SELECT COALESCE(SUM(amount_zats), 0) FROM vesting_claims
           WHERE agreement_id = NEW.id AND status IN ('reserved', 'settled')) THEN
        RAISE EXCEPTION 'cancellation would freeze less than is already claimed';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER vesting_agreements_guard
BEFORE INSERT OR UPDATE OR DELETE ON vesting_agreements
FOR EACH ROW EXECUTE FUNCTION vesting_agreements_guard();

CREATE FUNCTION vesting_claims_guard_insert() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.status <> 'reserved' OR NOT EXISTS (
           SELECT 1 FROM vesting_agreements a
           WHERE a.id = NEW.agreement_id AND a.status <> 'draft'
             AND a.employee_user_id = NEW.requested_by
             AND NEW.vested_at_reservation_zats <= COALESCE(a.vested_at_cancel_zats, a.total_zats)) THEN
        RAISE EXCEPTION 'vesting claim must be reserved, by the employee, of an activated agreement, within what can vest';
    END IF;
    IF NEW.amount_zats + (
           SELECT COALESCE(SUM(amount_zats), 0) FROM vesting_claims
           WHERE agreement_id = NEW.agreement_id AND status IN ('reserved', 'settled')
       ) > NEW.vested_at_reservation_zats THEN
        RAISE EXCEPTION 'claim would exceed vested compensation';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER vesting_claims_guard_insert
BEFORE INSERT ON vesting_claims
FOR EACH ROW EXECUTE FUNCTION vesting_claims_guard_insert();

CREATE FUNCTION vesting_claims_guard_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'claims cannot be deleted';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.agreement_id IS DISTINCT FROM OLD.agreement_id
       OR NEW.requested_by IS DISTINCT FROM OLD.requested_by
       OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
       OR NEW.amount_zats IS DISTINCT FROM OLD.amount_zats
       OR NEW.vested_at_reservation_zats IS DISTINCT FROM OLD.vested_at_reservation_zats
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'claim terms are immutable';
    END IF;
    IF NOT (OLD.status = 'reserved' AND NEW.status IN ('released', 'settled')) THEN
        RAISE EXCEPTION 'invalid claim status transition';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER vesting_claims_guard_change
BEFORE UPDATE OR DELETE ON vesting_claims
FOR EACH ROW EXECUTE FUNCTION vesting_claims_guard_change();

-- Same row-level security as 0003_rls.sql.
ALTER TABLE vesting_agreements ENABLE ROW LEVEL SECURITY;
ALTER TABLE vesting_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE vesting_claims ENABLE ROW LEVEL SECURITY;

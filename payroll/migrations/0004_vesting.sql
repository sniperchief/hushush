-- hushush payroll schema v4: vesting compensation.
--
-- A vesting agreement grants one employee a fixed ZEC allocation that vests
-- linearly over `duration_secs` from `start_at`, with an optional cliff. The
-- vested amount is computed by the engine from the server clock (see
-- payroll/src/vesting.rs); the schema stores the terms, the lifecycle, and the
-- amount frozen at cancellation.
--
--   draft -> active -> cancelled      (cancelled is final)
--   draft -> cancelled                (voided; nothing was ever owed)
--
-- Vesting claims are reservations against vested compensation, separate from
-- stream claims (`claims`) and with the same rules:
--   reserved -> released  (cancelled; funds become available again)
--   reserved -> settled   (paid; `settlement_ref` set; final)
-- Nothing here holds keys, addresses or wallet credentials.

CREATE TABLE vesting_agreements (
    id                    TEXT    PRIMARY KEY CHECK (length(id) = 36),
    organization_id       TEXT    NOT NULL REFERENCES organizations (id),
    employee_user_id      TEXT    NOT NULL,
    created_by            TEXT    NOT NULL REFERENCES users (id),
    total_zats            INTEGER NOT NULL CHECK (total_zats > 0 AND total_zats <= 2100000000000000),
    start_at              INTEGER NOT NULL CHECK (start_at >= 0),
    duration_secs         INTEGER NOT NULL CHECK (duration_secs > 0 AND duration_secs <= 3153600000),
    cliff_secs            INTEGER NOT NULL DEFAULT 0 CHECK (cliff_secs >= 0 AND cliff_secs <= duration_secs),
    method                TEXT    NOT NULL CHECK (method IN ('linear')),
    status                TEXT    NOT NULL CHECK (status IN ('draft', 'active', 'cancelled')),
    created_at            INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at            INTEGER NOT NULL CHECK (updated_at >= created_at),
    activated_at          INTEGER CHECK (activated_at IS NULL OR activated_at >= created_at),
    activated_by          TEXT    REFERENCES users (id),
    cancelled_at          INTEGER CHECK (cancelled_at IS NULL OR cancelled_at >= created_at),
    cancelled_by          TEXT    REFERENCES users (id),
    vested_at_cancel_zats INTEGER CHECK (vested_at_cancel_zats IS NULL
                                         OR (vested_at_cancel_zats >= 0 AND vested_at_cancel_zats <= total_zats)),
    -- No vesting from before the agreement existed.
    CHECK (start_at >= created_at),
    CHECK (employee_user_id <> created_by),
    CHECK ((activated_at IS NULL) = (activated_by IS NULL)),
    CHECK ((status = 'draft') = (activated_at IS NULL AND cancelled_at IS NULL)),
    CHECK ((status = 'cancelled') = (cancelled_at IS NOT NULL)),
    CHECK ((cancelled_at IS NULL) = (cancelled_by IS NULL)),
    CHECK ((cancelled_at IS NULL) = (vested_at_cancel_zats IS NULL)),
    -- A voided draft owes nothing.
    CHECK (activated_at IS NOT NULL OR COALESCE(vested_at_cancel_zats, 0) = 0),
    -- The employee must be a member of the granting organization.
    FOREIGN KEY (organization_id, employee_user_id)
        REFERENCES organization_members (organization_id, user_id)
) STRICT;

CREATE INDEX vesting_agreements_org ON vesting_agreements (organization_id);
CREATE INDEX vesting_agreements_employee ON vesting_agreements (employee_user_id);

CREATE TRIGGER vesting_agreements_immutable_terms
BEFORE UPDATE OF id, organization_id, employee_user_id, created_by, total_zats, start_at,
                 duration_secs, cliff_secs, method, created_at
ON vesting_agreements
BEGIN
    SELECT RAISE(ABORT, 'vesting agreement terms are immutable');
END;

CREATE TRIGGER vesting_agreements_final
BEFORE UPDATE ON vesting_agreements
WHEN OLD.status = 'cancelled'
BEGIN
    SELECT RAISE(ABORT, 'cancelled vesting agreements are final');
END;

CREATE TRIGGER vesting_agreements_valid_transition
BEFORE UPDATE ON vesting_agreements
WHEN NEW.updated_at < OLD.updated_at
  OR (OLD.activated_at IS NOT NULL AND (NEW.activated_at IS NOT OLD.activated_at
                                        OR NEW.activated_by IS NOT OLD.activated_by))
  OR (NEW.status <> OLD.status AND NOT (
         (OLD.status = 'draft' AND NEW.status IN ('active', 'cancelled'))
      OR (OLD.status = 'active' AND NEW.status = 'cancelled')))
BEGIN
    SELECT RAISE(ABORT, 'invalid vesting agreement transition');
END;

CREATE TRIGGER vesting_agreements_new_is_draft
BEFORE INSERT ON vesting_agreements
WHEN NEW.status <> 'draft'
BEGIN
    SELECT RAISE(ABORT, 'vesting agreements must be created as drafts');
END;

CREATE TRIGGER vesting_agreements_no_delete
BEFORE DELETE ON vesting_agreements
BEGIN
    SELECT RAISE(ABORT, 'vesting agreements cannot be deleted');
END;

-- Append-only audit log of lifecycle operations.
CREATE TABLE vesting_events (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    agreement_id      TEXT    NOT NULL REFERENCES vesting_agreements (id),
    kind              TEXT    NOT NULL CHECK (kind IN ('created', 'activated', 'cancelled')),
    actor_user_id     TEXT    NOT NULL REFERENCES users (id),
    occurred_at       INTEGER NOT NULL CHECK (occurred_at >= 0),
    vested_zats_after INTEGER NOT NULL CHECK (vested_zats_after >= 0)
) STRICT;

CREATE INDEX vesting_events_agreement ON vesting_events (agreement_id, id);

CREATE TRIGGER vesting_events_no_update
BEFORE UPDATE ON vesting_events
BEGIN
    SELECT RAISE(ABORT, 'vesting_events is append-only');
END;

CREATE TRIGGER vesting_events_no_delete
BEFORE DELETE ON vesting_events
BEGIN
    SELECT RAISE(ABORT, 'vesting_events is append-only');
END;

CREATE TABLE vesting_claims (
    id                         TEXT    PRIMARY KEY CHECK (length(id) = 36),
    agreement_id               TEXT    NOT NULL REFERENCES vesting_agreements (id),
    requested_by               TEXT    NOT NULL REFERENCES users (id),
    idempotency_key            TEXT    NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    amount_zats                INTEGER NOT NULL CHECK (amount_zats > 0 AND amount_zats <= 2100000000000000),
    status                     TEXT    NOT NULL CHECK (status IN ('reserved', 'released', 'settled')),
    vested_at_reservation_zats INTEGER NOT NULL CHECK (vested_at_reservation_zats >= amount_zats),
    created_at                 INTEGER NOT NULL CHECK (created_at >= 0),
    resolved_at                INTEGER CHECK (resolved_at IS NULL OR resolved_at >= created_at),
    resolved_by                TEXT    REFERENCES users (id),
    settlement_ref             TEXT    CHECK (settlement_ref IS NULL OR length(settlement_ref) BETWEEN 1 AND 256),
    CHECK ((status = 'reserved') = (resolved_at IS NULL)),
    CHECK ((status = 'reserved') = (resolved_by IS NULL)),
    CHECK ((status = 'settled') = (settlement_ref IS NOT NULL)),
    -- Duplicate requests (same idempotency key) can never create a second claim.
    UNIQUE (agreement_id, idempotency_key)
) STRICT;

CREATE INDEX vesting_claims_agreement_status ON vesting_claims (agreement_id, status);

-- Defence in depth: a new reservation is made by the employee of an
-- activated agreement, the vested amount it was checked against never exceeds
-- what the agreement can ever vest (its total, or the amount frozen at
-- cancellation), and reserved + settled never exceed that vested amount.
CREATE TRIGGER vesting_claims_valid_insert
BEFORE INSERT ON vesting_claims
WHEN NEW.status <> 'reserved'
  OR NOT EXISTS (
        SELECT 1 FROM vesting_agreements a
        WHERE a.id = NEW.agreement_id AND a.status <> 'draft'
          AND a.employee_user_id = NEW.requested_by
          AND NEW.vested_at_reservation_zats <= COALESCE(a.vested_at_cancel_zats, a.total_zats))
BEGIN
    SELECT RAISE(ABORT, 'vesting claim must be reserved, by the employee, of an activated agreement, within what can vest');
END;

CREATE TRIGGER vesting_claims_no_overdraw
BEFORE INSERT ON vesting_claims
WHEN NEW.amount_zats + (
        SELECT COALESCE(SUM(amount_zats), 0) FROM vesting_claims
        WHERE agreement_id = NEW.agreement_id AND status IN ('reserved', 'settled')
     ) > NEW.vested_at_reservation_zats
BEGIN
    SELECT RAISE(ABORT, 'claim would exceed vested compensation');
END;

CREATE TRIGGER vesting_claims_immutable
BEFORE UPDATE OF id, agreement_id, requested_by, idempotency_key, amount_zats,
                 vested_at_reservation_zats, created_at
ON vesting_claims
BEGIN
    SELECT RAISE(ABORT, 'claim terms are immutable');
END;

CREATE TRIGGER vesting_claims_valid_transition
BEFORE UPDATE ON vesting_claims
WHEN NOT (OLD.status = 'reserved' AND NEW.status IN ('released', 'settled'))
BEGIN
    SELECT RAISE(ABORT, 'invalid claim status transition');
END;

CREATE TRIGGER vesting_claims_no_delete
BEFORE DELETE ON vesting_claims
BEGIN
    SELECT RAISE(ABORT, 'claims cannot be deleted');
END;

-- Cancellation can never freeze less than is already reserved or paid.
CREATE TRIGGER vesting_agreements_cancel_covers_claims
BEFORE UPDATE OF status ON vesting_agreements
WHEN NEW.status = 'cancelled' AND NEW.vested_at_cancel_zats < (
        SELECT COALESCE(SUM(amount_zats), 0) FROM vesting_claims
        WHERE agreement_id = NEW.id AND status IN ('reserved', 'settled'))
BEGIN
    SELECT RAISE(ABORT, 'cancellation would freeze less than is already claimed');
END;

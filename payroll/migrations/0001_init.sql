-- hushush payroll schema v1.
-- All money is INTEGER zatoshis (1 ZEC = 100,000,000 zats). All times are
-- INTEGER unix seconds (UTC), supplied by the server clock. Tables are STRICT,
-- so SQLite rejects values of the wrong type (e.g. a REAL amount).

CREATE TABLE users (
    id           TEXT    PRIMARY KEY CHECK (length(id) = 36),
    email        TEXT    NOT NULL UNIQUE COLLATE NOCASE
                         CHECK (length(email) BETWEEN 3 AND 320 AND email LIKE '%_@_%'),
    display_name TEXT    NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
    created_at   INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE TABLE organizations (
    id         TEXT    PRIMARY KEY CHECK (length(id) = 36),
    name       TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    created_by TEXT    NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE TABLE organization_members (
    organization_id TEXT    NOT NULL REFERENCES organizations (id),
    user_id         TEXT    NOT NULL REFERENCES users (id),
    role            TEXT    NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    created_at      INTEGER NOT NULL CHECK (created_at >= 0),
    PRIMARY KEY (organization_id, user_id)
) STRICT;

-- A stream pays `rate_zats` per `rate_period_secs` of ACTIVE time.
-- Accrual state is kept as `banked_active_secs` (closed active intervals) plus
-- `active_since` (start of the currently open interval, NULL unless active).
CREATE TABLE payroll_streams (
    id                 TEXT    PRIMARY KEY CHECK (length(id) = 36),
    organization_id    TEXT    NOT NULL,
    recipient_user_id  TEXT    NOT NULL,
    created_by         TEXT    NOT NULL REFERENCES users (id),
    rate_zats          INTEGER NOT NULL CHECK (rate_zats > 0 AND rate_zats <= 2100000000000000),
    rate_period_secs   INTEGER NOT NULL CHECK (rate_period_secs > 0 AND rate_period_secs <= 3153600000),
    start_at           INTEGER NOT NULL CHECK (start_at >= 0),
    end_at             INTEGER CHECK (end_at IS NULL OR end_at > start_at),
    status             TEXT    NOT NULL CHECK (status IN ('active', 'paused', 'ended')),
    banked_active_secs INTEGER NOT NULL DEFAULT 0 CHECK (banked_active_secs >= 0),
    active_since       INTEGER CHECK (active_since IS NULL OR active_since >= start_at),
    ended_at           INTEGER,
    created_at         INTEGER NOT NULL CHECK (created_at >= 0),
    last_event_at      INTEGER NOT NULL,
    CHECK ((status = 'active') = (active_since IS NOT NULL)),
    CHECK ((status = 'ended') = (ended_at IS NOT NULL)),
    -- The recipient must be a member of the paying organization.
    FOREIGN KEY (organization_id, recipient_user_id)
        REFERENCES organization_members (organization_id, user_id),
    FOREIGN KEY (organization_id) REFERENCES organizations (id)
) STRICT;

CREATE INDEX payroll_streams_org ON payroll_streams (organization_id);
CREATE INDEX payroll_streams_recipient ON payroll_streams (recipient_user_id);

-- Terms of a stream are fixed once created; accrual state only moves forward;
-- 'ended' is terminal.
CREATE TRIGGER payroll_streams_immutable_terms
BEFORE UPDATE OF id, organization_id, recipient_user_id, created_by, rate_zats,
                 rate_period_secs, start_at, end_at, created_at
ON payroll_streams
BEGIN
    SELECT RAISE(ABORT, 'payroll stream terms are immutable');
END;

CREATE TRIGGER payroll_streams_forward_only
BEFORE UPDATE ON payroll_streams
WHEN NEW.banked_active_secs < OLD.banked_active_secs
  OR NEW.last_event_at < OLD.last_event_at
  OR OLD.status = 'ended'
BEGIN
    SELECT RAISE(ABORT, 'payroll stream state may only move forward and ended streams are final');
END;

CREATE TRIGGER payroll_streams_no_delete
BEFORE DELETE ON payroll_streams
BEGIN
    SELECT RAISE(ABORT, 'payroll streams cannot be deleted');
END;

-- Append-only audit log of lifecycle operations.
CREATE TABLE stream_events (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id                TEXT    NOT NULL REFERENCES payroll_streams (id),
    kind                     TEXT    NOT NULL CHECK (kind IN ('created', 'paused', 'resumed', 'ended')),
    actor_user_id            TEXT    NOT NULL REFERENCES users (id),
    occurred_at              INTEGER NOT NULL CHECK (occurred_at >= 0),
    banked_active_secs_after INTEGER NOT NULL CHECK (banked_active_secs_after >= 0),
    earned_zats_after        INTEGER NOT NULL CHECK (earned_zats_after >= 0)
) STRICT;

CREATE INDEX stream_events_stream ON stream_events (stream_id, id);

CREATE TRIGGER stream_events_no_update
BEFORE UPDATE ON stream_events
BEGIN
    SELECT RAISE(ABORT, 'stream_events is append-only');
END;

CREATE TRIGGER stream_events_no_delete
BEFORE DELETE ON stream_events
BEGIN
    SELECT RAISE(ABORT, 'stream_events is append-only');
END;

-- Claims are reservations against earned wages. No payout happens in Phase 1.
--   reserved -> released  (cancelled; funds become available again)
--   reserved -> settled   (paid out by a later phase; `settlement_ref` set)
CREATE TABLE claims (
    id                         TEXT    PRIMARY KEY CHECK (length(id) = 36),
    stream_id                  TEXT    NOT NULL REFERENCES payroll_streams (id),
    requested_by               TEXT    NOT NULL REFERENCES users (id),
    idempotency_key            TEXT    NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    amount_zats                INTEGER NOT NULL CHECK (amount_zats > 0 AND amount_zats <= 2100000000000000),
    status                     TEXT    NOT NULL CHECK (status IN ('reserved', 'released', 'settled')),
    earned_at_reservation_zats INTEGER NOT NULL CHECK (earned_at_reservation_zats >= amount_zats),
    created_at                 INTEGER NOT NULL CHECK (created_at >= 0),
    resolved_at                INTEGER CHECK (resolved_at IS NULL OR resolved_at >= created_at),
    resolved_by                TEXT    REFERENCES users (id),
    settlement_ref             TEXT    CHECK (settlement_ref IS NULL OR length(settlement_ref) BETWEEN 1 AND 256),
    CHECK ((status = 'reserved') = (resolved_at IS NULL)),
    CHECK ((status = 'reserved') = (resolved_by IS NULL)),
    CHECK ((status = 'settled') = (settlement_ref IS NOT NULL)),
    -- Duplicate requests (same idempotency key) can never create a second claim.
    UNIQUE (stream_id, idempotency_key)
) STRICT;

CREATE INDEX claims_stream_status ON claims (stream_id, status);

-- Defence in depth: a new reservation may never push reserved + settled above
-- the earned amount the server computed for it in the same transaction.
CREATE TRIGGER claims_no_overdraw
BEFORE INSERT ON claims
WHEN NEW.amount_zats + (
        SELECT COALESCE(SUM(amount_zats), 0) FROM claims
        WHERE stream_id = NEW.stream_id AND status IN ('reserved', 'settled')
     ) > NEW.earned_at_reservation_zats
BEGIN
    SELECT RAISE(ABORT, 'claim would exceed earned wages');
END;

CREATE TRIGGER claims_new_must_be_reserved
BEFORE INSERT ON claims
WHEN NEW.status <> 'reserved'
BEGIN
    SELECT RAISE(ABORT, 'claims must be created in the reserved state');
END;

CREATE TRIGGER claims_immutable
BEFORE UPDATE OF id, stream_id, requested_by, idempotency_key, amount_zats,
                 earned_at_reservation_zats, created_at
ON claims
BEGIN
    SELECT RAISE(ABORT, 'claim terms are immutable');
END;

CREATE TRIGGER claims_valid_transition
BEFORE UPDATE OF status ON claims
WHEN NOT (OLD.status = 'reserved' AND NEW.status IN ('released', 'settled'))
BEGIN
    SELECT RAISE(ABORT, 'invalid claim status transition');
END;

CREATE TRIGGER claims_no_delete
BEFORE DELETE ON claims
BEGIN
    SELECT RAISE(ABORT, 'claims cannot be deleted');
END;

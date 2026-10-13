-- hushush payroll schema v1 — PostgreSQL edition.
-- Mirrors migrations/0001_init.sql (SQLite) table-for-table and rule-for-rule.
-- All money is BIGINT zatoshis; all times BIGINT unix seconds from the server clock.

CREATE TABLE users (
    id           TEXT   PRIMARY KEY CHECK (length(id) = 36),
    email        TEXT   NOT NULL CHECK (length(email) BETWEEN 3 AND 320 AND email LIKE '%_@_%'),
    display_name TEXT   NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
    created_at   BIGINT NOT NULL CHECK (created_at >= 0)
);
CREATE UNIQUE INDEX users_email_lower ON users (lower(email));

CREATE TABLE organizations (
    id         TEXT   PRIMARY KEY CHECK (length(id) = 36),
    name       TEXT   NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    created_by TEXT   NOT NULL REFERENCES users (id),
    created_at BIGINT NOT NULL CHECK (created_at >= 0)
);

CREATE TABLE organization_members (
    organization_id TEXT   NOT NULL REFERENCES organizations (id),
    user_id         TEXT   NOT NULL REFERENCES users (id),
    role            TEXT   NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    created_at      BIGINT NOT NULL CHECK (created_at >= 0),
    PRIMARY KEY (organization_id, user_id)
);
CREATE INDEX organization_members_user ON organization_members (user_id);

CREATE TABLE payroll_streams (
    id                 TEXT   PRIMARY KEY CHECK (length(id) = 36),
    organization_id    TEXT   NOT NULL REFERENCES organizations (id),
    recipient_user_id  TEXT   NOT NULL,
    created_by         TEXT   NOT NULL REFERENCES users (id),
    rate_zats          BIGINT NOT NULL CHECK (rate_zats > 0 AND rate_zats <= 2100000000000000),
    rate_period_secs   BIGINT NOT NULL CHECK (rate_period_secs > 0 AND rate_period_secs <= 3153600000),
    start_at           BIGINT NOT NULL CHECK (start_at >= 0),
    end_at             BIGINT CHECK (end_at IS NULL OR end_at > start_at),
    status             TEXT   NOT NULL CHECK (status IN ('active', 'paused', 'ended')),
    banked_active_secs BIGINT NOT NULL DEFAULT 0 CHECK (banked_active_secs >= 0),
    active_since       BIGINT CHECK (active_since IS NULL OR active_since >= start_at),
    ended_at           BIGINT,
    created_at         BIGINT NOT NULL CHECK (created_at >= 0),
    last_event_at      BIGINT NOT NULL,
    CHECK ((status = 'active') = (active_since IS NOT NULL)),
    CHECK ((status = 'ended') = (ended_at IS NOT NULL)),
    FOREIGN KEY (organization_id, recipient_user_id)
        REFERENCES organization_members (organization_id, user_id)
);
CREATE INDEX payroll_streams_org ON payroll_streams (organization_id);
CREATE INDEX payroll_streams_recipient ON payroll_streams (recipient_user_id);

CREATE FUNCTION payroll_streams_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'payroll streams cannot be deleted';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
       OR NEW.recipient_user_id IS DISTINCT FROM OLD.recipient_user_id
       OR NEW.created_by IS DISTINCT FROM OLD.created_by
       OR NEW.rate_zats IS DISTINCT FROM OLD.rate_zats
       OR NEW.rate_period_secs IS DISTINCT FROM OLD.rate_period_secs
       OR NEW.start_at IS DISTINCT FROM OLD.start_at
       OR NEW.end_at IS DISTINCT FROM OLD.end_at
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'payroll stream terms are immutable';
    END IF;
    IF NEW.banked_active_secs < OLD.banked_active_secs
       OR NEW.last_event_at < OLD.last_event_at
       OR OLD.status = 'ended' THEN
        RAISE EXCEPTION 'payroll stream state may only move forward and ended streams are final';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER payroll_streams_guard
BEFORE UPDATE OR DELETE ON payroll_streams
FOR EACH ROW EXECUTE FUNCTION payroll_streams_guard();

CREATE TABLE stream_events (
    id                       BIGSERIAL PRIMARY KEY,
    stream_id                TEXT   NOT NULL REFERENCES payroll_streams (id),
    kind                     TEXT   NOT NULL CHECK (kind IN ('created', 'paused', 'resumed', 'ended')),
    actor_user_id            TEXT   NOT NULL REFERENCES users (id),
    occurred_at              BIGINT NOT NULL CHECK (occurred_at >= 0),
    banked_active_secs_after BIGINT NOT NULL CHECK (banked_active_secs_after >= 0),
    earned_zats_after        BIGINT NOT NULL CHECK (earned_zats_after >= 0)
);
CREATE INDEX stream_events_stream ON stream_events (stream_id, id);

CREATE FUNCTION append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '% is append-only', TG_TABLE_NAME;
END $$;

CREATE TRIGGER stream_events_append_only
BEFORE UPDATE OR DELETE ON stream_events
FOR EACH ROW EXECUTE FUNCTION append_only();

CREATE TABLE claims (
    id                         TEXT   PRIMARY KEY CHECK (length(id) = 36),
    stream_id                  TEXT   NOT NULL REFERENCES payroll_streams (id),
    requested_by               TEXT   NOT NULL REFERENCES users (id),
    idempotency_key            TEXT   NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    amount_zats                BIGINT NOT NULL CHECK (amount_zats > 0 AND amount_zats <= 2100000000000000),
    status                     TEXT   NOT NULL CHECK (status IN ('reserved', 'released', 'settled')),
    earned_at_reservation_zats BIGINT NOT NULL CHECK (earned_at_reservation_zats >= amount_zats),
    created_at                 BIGINT NOT NULL CHECK (created_at >= 0),
    resolved_at                BIGINT CHECK (resolved_at IS NULL OR resolved_at >= created_at),
    resolved_by                TEXT   REFERENCES users (id),
    settlement_ref             TEXT   CHECK (settlement_ref IS NULL OR length(settlement_ref) BETWEEN 1 AND 256),
    CHECK ((status = 'reserved') = (resolved_at IS NULL)),
    CHECK ((status = 'reserved') = (resolved_by IS NULL)),
    CHECK ((status = 'settled') = (settlement_ref IS NOT NULL)),
    UNIQUE (stream_id, idempotency_key)
);
CREATE INDEX claims_stream_status ON claims (stream_id, status);

CREATE FUNCTION claims_guard_insert() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.status <> 'reserved' THEN
        RAISE EXCEPTION 'claims must be created in the reserved state';
    END IF;
    IF NEW.amount_zats + (
           SELECT COALESCE(SUM(amount_zats), 0) FROM claims
           WHERE stream_id = NEW.stream_id AND status IN ('reserved', 'settled')
       ) > NEW.earned_at_reservation_zats THEN
        RAISE EXCEPTION 'claim would exceed earned wages';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER claims_guard_insert
BEFORE INSERT ON claims
FOR EACH ROW EXECUTE FUNCTION claims_guard_insert();

CREATE FUNCTION claims_guard_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'claims cannot be deleted';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.stream_id IS DISTINCT FROM OLD.stream_id
       OR NEW.requested_by IS DISTINCT FROM OLD.requested_by
       OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
       OR NEW.amount_zats IS DISTINCT FROM OLD.amount_zats
       OR NEW.earned_at_reservation_zats IS DISTINCT FROM OLD.earned_at_reservation_zats
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'claim terms are immutable';
    END IF;
    IF NEW.status IS DISTINCT FROM OLD.status
       AND NOT (OLD.status = 'reserved' AND NEW.status IN ('released', 'settled')) THEN
        RAISE EXCEPTION 'invalid claim status transition';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER claims_guard_change
BEFORE UPDATE OR DELETE ON claims
FOR EACH ROW EXECUTE FUNCTION claims_guard_change();

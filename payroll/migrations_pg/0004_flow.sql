-- hushush payroll schema v4 — PostgreSQL edition of migrations/0003_flow.sql
-- (Phase 5 flow): stream offers and locked payout addresses. See that file
-- for the model; rules here are identical.

CREATE TABLE stream_offers (
    id              TEXT   PRIMARY KEY CHECK (length(id) = 36),
    organization_id TEXT   NOT NULL REFERENCES organizations (id),
    created_by      TEXT   NOT NULL REFERENCES users (id),
    worker_name     TEXT   NOT NULL CHECK (length(worker_name) BETWEEN 1 AND 200),
    total_zats      BIGINT NOT NULL CHECK (total_zats > 0 AND total_zats <= 2100000000000000),
    start_at        BIGINT NOT NULL CHECK (start_at >= 0),
    end_at          BIGINT NOT NULL CHECK (end_at > start_at AND end_at - start_at <= 3153600000),
    link_hash       TEXT   NOT NULL UNIQUE CHECK (length(link_hash) = 64),
    created_at      BIGINT NOT NULL CHECK (created_at >= 0),
    stream_id       TEXT   UNIQUE REFERENCES payroll_streams (id),
    bound_user_id   TEXT   REFERENCES users (id),
    bound_at        BIGINT,
    cancelled_at    BIGINT,
    CHECK ((stream_id IS NULL) = (bound_at IS NULL)),
    CHECK ((stream_id IS NULL) = (bound_user_id IS NULL)),
    CHECK (stream_id IS NULL OR cancelled_at IS NULL)
);
CREATE INDEX stream_offers_org ON stream_offers (organization_id);

CREATE FUNCTION stream_offers_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'stream offers cannot be deleted';
    END IF;
    IF OLD.stream_id IS NOT NULL OR OLD.cancelled_at IS NOT NULL THEN
        RAISE EXCEPTION 'stream offer is final';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
       OR NEW.created_by IS DISTINCT FROM OLD.created_by
       OR NEW.worker_name IS DISTINCT FROM OLD.worker_name
       OR NEW.total_zats IS DISTINCT FROM OLD.total_zats
       OR NEW.start_at IS DISTINCT FROM OLD.start_at
       OR NEW.end_at IS DISTINCT FROM OLD.end_at
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'stream offer terms are immutable';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER stream_offers_guard
BEFORE UPDATE OR DELETE ON stream_offers
FOR EACH ROW EXECUTE FUNCTION stream_offers_guard();

CREATE TABLE stream_payout_addresses (
    stream_id           TEXT   PRIMARY KEY REFERENCES payroll_streams (id),
    set_by              TEXT   NOT NULL REFERENCES users (id),
    address_fingerprint TEXT   NOT NULL CHECK (length(address_fingerprint) = 64),
    address_sealed      TEXT   NOT NULL CHECK (length(address_sealed) BETWEEN 1 AND 4000),
    set_at              BIGINT NOT NULL CHECK (set_at >= 0)
);

CREATE FUNCTION stream_payout_addresses_insert() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM payroll_streams s
                   WHERE s.id = NEW.stream_id AND s.recipient_user_id = NEW.set_by) THEN
        RAISE EXCEPTION 'only the stream recipient can set its payout address';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER stream_payout_addresses_insert
BEFORE INSERT ON stream_payout_addresses
FOR EACH ROW EXECUTE FUNCTION stream_payout_addresses_insert();

CREATE FUNCTION stream_payout_addresses_locked() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'payout address is locked for the life of the stream';
END $$;

CREATE TRIGGER stream_payout_addresses_locked
BEFORE UPDATE OR DELETE ON stream_payout_addresses
FOR EACH ROW EXECUTE FUNCTION stream_payout_addresses_locked();

-- Same row-level security as 0003_rls.sql.
ALTER TABLE stream_offers ENABLE ROW LEVEL SECURITY;
ALTER TABLE stream_payout_addresses ENABLE ROW LEVEL SECURITY;

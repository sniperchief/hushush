-- hushush payroll schema v3 (Phase 5 flow): stream offers and locked payout
-- addresses.
--
-- A *stream offer* is what an employer creates: the worker's display name and
-- the terms (total, start, end). It is reachable through a single-use worker
-- link (only the SHA-256 of the link token is stored). When a signed-in worker
-- opens the link, the offer is *bound*: the worker joins the organization and
-- the real payroll stream is created with the offer's terms, in one
-- transaction. The stream pays exactly `total_zats` over `end_at - start_at`
-- (rate_zats = total_zats, rate_period_secs = end_at - start_at).
--
-- A stream's payout address is set once by its recipient and is then locked
-- for the life of the stream (no update, no delete). The address itself is
-- stored sealed (encrypted by payroll-api with a key the database never sees)
-- next to a salted fingerprint used for equality checks.

CREATE TABLE stream_offers (
    id              TEXT    PRIMARY KEY CHECK (length(id) = 36),
    organization_id TEXT    NOT NULL REFERENCES organizations (id),
    created_by      TEXT    NOT NULL REFERENCES users (id),
    worker_name     TEXT    NOT NULL CHECK (length(worker_name) BETWEEN 1 AND 200),
    total_zats      INTEGER NOT NULL CHECK (total_zats > 0 AND total_zats <= 2100000000000000),
    start_at        INTEGER NOT NULL CHECK (start_at >= 0),
    end_at          INTEGER NOT NULL CHECK (end_at > start_at AND end_at - start_at <= 3153600000),
    link_hash       TEXT    NOT NULL UNIQUE CHECK (length(link_hash) = 64),
    created_at      INTEGER NOT NULL CHECK (created_at >= 0),
    stream_id       TEXT    UNIQUE REFERENCES payroll_streams (id),
    bound_user_id   TEXT    REFERENCES users (id),
    bound_at        INTEGER,
    cancelled_at    INTEGER,
    CHECK ((stream_id IS NULL) = (bound_at IS NULL)),
    CHECK ((stream_id IS NULL) = (bound_user_id IS NULL)),
    CHECK (stream_id IS NULL OR cancelled_at IS NULL)
) STRICT;

CREATE INDEX stream_offers_org ON stream_offers (organization_id);

CREATE TRIGGER stream_offers_no_delete
BEFORE DELETE ON stream_offers
BEGIN
    SELECT RAISE(ABORT, 'stream offers cannot be deleted');
END;

CREATE TRIGGER stream_offers_immutable_terms
BEFORE UPDATE OF id, organization_id, created_by, worker_name, total_zats, start_at, end_at, created_at
ON stream_offers
BEGIN
    SELECT RAISE(ABORT, 'stream offer terms are immutable');
END;

-- Bound and cancelled offers are final (their link can no longer change either).
CREATE TRIGGER stream_offers_final
BEFORE UPDATE ON stream_offers
WHEN OLD.stream_id IS NOT NULL OR OLD.cancelled_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'stream offer is final');
END;

CREATE TABLE stream_payout_addresses (
    stream_id           TEXT    PRIMARY KEY REFERENCES payroll_streams (id),
    set_by              TEXT    NOT NULL REFERENCES users (id),
    address_fingerprint TEXT    NOT NULL CHECK (length(address_fingerprint) = 64),
    address_sealed      TEXT    NOT NULL CHECK (length(address_sealed) BETWEEN 1 AND 4000),
    set_at              INTEGER NOT NULL CHECK (set_at >= 0)
) STRICT;

-- Set by the stream's recipient only.
CREATE TRIGGER stream_payout_addresses_recipient
BEFORE INSERT ON stream_payout_addresses
WHEN NOT EXISTS (
        SELECT 1 FROM payroll_streams s
        WHERE s.id = NEW.stream_id AND s.recipient_user_id = NEW.set_by)
BEGIN
    SELECT RAISE(ABORT, 'only the stream recipient can set its payout address');
END;

CREATE TRIGGER stream_payout_addresses_locked
BEFORE UPDATE ON stream_payout_addresses
BEGIN
    SELECT RAISE(ABORT, 'payout address is locked for the life of the stream');
END;

CREATE TRIGGER stream_payout_addresses_no_delete
BEFORE DELETE ON stream_payout_addresses
BEGIN
    SELECT RAISE(ABORT, 'payout address is locked for the life of the stream');
END;

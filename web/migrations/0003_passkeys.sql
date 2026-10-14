-- hushush web app, Phase 5: passkey (WebAuthn) accounts replace passwords,
-- e-mail verification and invitations.
--
-- * An account is either an employer (owns exactly one company) or a worker
--   (joins streams through worker links). The kind is fixed at sign-up.
-- * Exactly one passkey per account (MVP decision; no recovery passkey).
-- * Accounts created before this migration had passwords only; they can no
--   longer sign in. Their payroll rows are left untouched (payroll history is
--   append-only); a deployment that held rehearsal data should be reset.

DROP TABLE invitations;
DROP TABLE email_verification_tokens;
DROP TABLE login_attempts;
DROP TABLE auth_credentials;
-- Password-era sessions are void.
DELETE FROM sessions;

CREATE TABLE accounts (
    user_id    TEXT        PRIMARY KEY REFERENCES users (id),
    kind       TEXT        NOT NULL CHECK (kind IN ('employer', 'worker')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE passkeys (
    -- base64url credential id, as the authenticator reports it.
    credential_id    TEXT        PRIMARY KEY CHECK (length(credential_id) BETWEEN 16 AND 1400),
    -- One passkey per account.
    user_id          TEXT        NOT NULL UNIQUE REFERENCES accounts (user_id),
    -- WebAuthn user handle (random, base64url); not the payroll user id.
    webauthn_user_id TEXT        NOT NULL UNIQUE,
    public_key       BYTEA       NOT NULL,
    counter          BIGINT      NOT NULL DEFAULT 0 CHECK (counter >= 0),
    transports       TEXT[]      NOT NULL DEFAULT '{}',
    device_type      TEXT        NOT NULL,
    backed_up        BOOLEAN     NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at     TIMESTAMPTZ
);

-- A pending WebAuthn ceremony. The browser holds a random handle in an
-- HttpOnly cookie; only its SHA-256 is stored. Single use, short-lived.
CREATE TABLE webauthn_challenges (
    handle_hash TEXT        PRIMARY KEY CHECK (length(handle_hash) = 64),
    purpose     TEXT        NOT NULL CHECK (purpose IN ('register', 'login')),
    challenge   TEXT        NOT NULL,
    -- register: { kind, displayName, webauthnUserId }
    payload     JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ
);
CREATE INDEX webauthn_challenges_expiry ON webauthn_challenges (expires_at);

-- Same row-level security as 0002_rls.sql.
ALTER TABLE accounts ENABLE ROW LEVEL SECURITY;
ALTER TABLE passkeys ENABLE ROW LEVEL SECURITY;
ALTER TABLE webauthn_challenges ENABLE ROW LEVEL SECURITY;

-- hushush web app: authentication, sessions and invitations.
-- Runs AFTER the payroll service's migrations (it references payroll's `users`
-- and `organizations` tables, which the payroll engine owns). The web app
-- never writes payroll tables directly; it calls payroll-api for that.

CREATE TABLE auth_credentials (
    user_id           TEXT        PRIMARY KEY REFERENCES users (id),
    password_hash     TEXT        NOT NULL CHECK (password_hash LIKE 'scrypt$%'),
    email_verified_at TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only SHA-256 hashes of tokens are stored; the raw token exists only in the
-- email link / cookie.
CREATE TABLE email_verification_tokens (
    token_hash TEXT        PRIMARY KEY CHECK (length(token_hash) = 64),
    user_id    TEXT        NOT NULL REFERENCES users (id),
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX email_verification_tokens_user ON email_verification_tokens (user_id);

CREATE TABLE sessions (
    token_hash   TEXT        PRIMARY KEY CHECK (length(token_hash) = 64),
    user_id      TEXT        NOT NULL REFERENCES users (id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL,
    CHECK (expires_at > created_at)
);
CREATE INDEX sessions_user ON sessions (user_id);

CREATE TABLE login_attempts (
    id           BIGSERIAL   PRIMARY KEY,
    email_lower  TEXT        NOT NULL,
    succeeded    BOOLEAN     NOT NULL,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX login_attempts_email_time ON login_attempts (email_lower, attempted_at);

CREATE TABLE invitations (
    id              TEXT        PRIMARY KEY,
    organization_id TEXT        NOT NULL REFERENCES organizations (id),
    email           TEXT        NOT NULL CHECK (length(email) BETWEEN 3 AND 320),
    role            TEXT        NOT NULL CHECK (role IN ('admin', 'member')),
    invited_by      TEXT        NOT NULL REFERENCES users (id),
    token_hash      TEXT        NOT NULL UNIQUE CHECK (length(token_hash) = 64),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    accepted_at     TIMESTAMPTZ,
    accepted_by     TEXT        REFERENCES users (id),
    revoked_at      TIMESTAMPTZ,
    CHECK (NOT (accepted_at IS NOT NULL AND revoked_at IS NOT NULL)),
    CHECK ((accepted_at IS NULL) = (accepted_by IS NULL))
);
-- At most one open invitation per email per organization.
CREATE UNIQUE INDEX invitations_open_unique
    ON invitations (organization_id, lower(email))
    WHERE accepted_at IS NULL AND revoked_at IS NULL;

-- Development-only mail sink (used when SMTP_URL is not configured).
CREATE TABLE dev_outbox (
    id         BIGSERIAL   PRIMARY KEY,
    to_address TEXT        NOT NULL,
    subject    TEXT        NOT NULL,
    body       TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

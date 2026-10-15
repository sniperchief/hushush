-- hushush web app: e-mail sign-in links for employers, next to passkeys.
--
-- * An employer account can have one e-mail address: set at e-mail sign-up,
--   or added later as a backup to a passkey. Worker accounts have none.
-- * The address lives only here; payroll's users.email keeps its reserved
--   placeholder (see createAccount).
-- * Sign-in links follow the passkey-challenge pattern: the raw token exists
--   only in the e-mailed link, only its SHA-256 is stored, single use,
--   short-lived.

CREATE TABLE account_emails (
    user_id     TEXT        PRIMARY KEY REFERENCES accounts (user_id),
    -- Trimmed and lower-cased before storage.
    email_lower TEXT        NOT NULL UNIQUE CHECK (length(email_lower) BETWEEN 3 AND 254 AND email_lower = lower(email_lower)),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE email_login_tokens (
    token_hash  TEXT        PRIMARY KEY CHECK (length(token_hash) = 64),
    -- signup: create an employer account; login: sign in to user_id;
    -- link: attach email_lower to user_id as its sign-in address.
    purpose     TEXT        NOT NULL CHECK (purpose IN ('signup', 'login', 'link')),
    email_lower TEXT        NOT NULL,
    user_id     TEXT        REFERENCES accounts (user_id),
    -- signup: { displayName }
    payload     JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ,
    CHECK ((purpose = 'signup') = (user_id IS NULL))
);
-- Rate limiting counts recent links per address.
CREATE INDEX email_login_tokens_email_time ON email_login_tokens (email_lower, created_at);
CREATE INDEX email_login_tokens_expiry ON email_login_tokens (expires_at);

-- Same row-level security as 0002_rls.sql.
ALTER TABLE account_emails ENABLE ROW LEVEL SECURITY;
ALTER TABLE email_login_tokens ENABLE ROW LEVEL SECURITY;

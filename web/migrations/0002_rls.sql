-- Row-level security for the web app's tables (see payroll's 0003_rls.sql):
-- no policies, so only the owning role (the app's own connection) can read or
-- write them; platform API roles (e.g. Supabase `anon`) see nothing. These
-- tables hold password hashes, session and verification token hashes.
ALTER TABLE auth_credentials ENABLE ROW LEVEL SECURITY;
ALTER TABLE sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE email_verification_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE invitations ENABLE ROW LEVEL SECURITY;
ALTER TABLE login_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE dev_outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE web_schema_migrations ENABLE ROW LEVEL SECURITY;

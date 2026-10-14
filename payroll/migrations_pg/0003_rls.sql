-- hushush payroll schema v3 (PostgreSQL only): row-level security.
--
-- Hosted Postgres platforms can expose the `public` schema to their own API
-- roles (Supabase's Data API grants `anon` / `authenticated` access to new
-- tables by default). Enabling RLS with NO policies denies every row to every
-- role except the tables' owner — the role hushush itself connects as, which
-- bypasses RLS because it is not FORCEd. Behaviour of the app is unchanged.
ALTER TABLE users ENABLE ROW LEVEL SECURITY;
ALTER TABLE organizations ENABLE ROW LEVEL SECURITY;
ALTER TABLE organization_members ENABLE ROW LEVEL SECURITY;
ALTER TABLE payroll_streams ENABLE ROW LEVEL SECURITY;
ALTER TABLE stream_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE claims ENABLE ROW LEVEL SECURITY;
ALTER TABLE payouts ENABLE ROW LEVEL SECURITY;
ALTER TABLE payout_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE payroll_schema_migrations ENABLE ROW LEVEL SECURITY;

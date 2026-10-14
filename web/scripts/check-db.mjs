// Pre-flight check of a PostgreSQL database (e.g. Supabase) for hushush,
// BEFORE deploying. Read-only except for objects created inside a
// transaction that is rolled back: nothing is left behind.
//
// Usage (Git Bash):
//   DATABASE_URL='postgresql://…?sslmode=require' DATABASE_CA_PEM="$(cat ca.crt)" node scripts/check-db.mjs
//
// Checks: TLS is used and verified against the given CA; server version;
// named prepared statements (needed by payroll-api — fails on transaction
// poolers); advisory locks (migrations); creating tables, PL/pgSQL functions,
// triggers and row-level security (the migrations); connection limits; and,
// if hushush is already migrated, that every table has RLS enabled.
import pg from "pg";

function dbConfig(url) {
  const pem = (process.env.DATABASE_CA_PEM ?? "").replace(/\\n/g, "\n").trim();
  if (!pem) return { connectionString: url };
  const u = new URL(url);
  for (const k of ["sslmode", "sslrootcert", "sslcert", "sslkey", "uselibpqcompat"]) u.searchParams.delete(k);
  return { connectionString: u.toString(), ssl: { ca: pem, rejectUnauthorized: true } };
}

const url = process.env.DATABASE_URL;
if (!url) {
  console.error("Set DATABASE_URL (and DATABASE_CA_PEM for Supabase).");
  process.exit(2);
}
const host = new URL(url).hostname;
const port = new URL(url).port || "5432";
let failures = 0;
const ok = (m) => console.log(`  ok    ${m}`);
const bad = (m) => {
  failures++;
  console.log(`  FAIL  ${m}`);
};
const warn = (m) => console.log(`  warn  ${m}`);

console.log(`hushush database pre-flight: ${host}:${port}`);
if (host.endsWith(".supabase.com") || host.endsWith(".supabase.co")) {
  if (port === "6543") bad("this is Supabase's TRANSACTION pooler (port 6543): use the SESSION pooler (port 5432)");
  if (host.startsWith("db.")) warn("this is Supabase's direct connection (IPv6-only on the free plan); Railway/Vercel need the session pooler");
  if (!process.env.DATABASE_CA_PEM) warn("DATABASE_CA_PEM is not set: Supabase's certificate will not verify");
}

const c = new pg.Client(dbConfig(url));
try {
  await c.connect();
} catch (e) {
  bad(`connect: ${e.message}`);
  process.exit(1);
}
try {
  const tls = (await c.query("SELECT ssl, version FROM pg_stat_ssl WHERE pid = pg_backend_pid()")).rows[0];
  if (tls?.ssl) ok(`TLS ${tls.version}, certificate verified`);
  else bad("connection is NOT encrypted");
  const v = (await c.query("SHOW server_version")).rows[0].server_version;
  Number.parseInt(v, 10) >= 15 ? ok(`PostgreSQL ${v}`) : bad(`PostgreSQL ${v} (need 15+)`);

  await c.query({ name: "hushush_preflight_stmt", text: "SELECT $1::int + 1 AS n", values: [41] });
  const r = await c.query({ name: "hushush_preflight_stmt", text: "SELECT $1::int + 1 AS n", values: [1] });
  r.rows[0].n === 2 ? ok("named prepared statements work (payroll-api needs them)") : bad("prepared statements");

  await c.query("BEGIN");
  try {
    await c.query("SELECT pg_advisory_xact_lock(hashtext('hushush_preflight'))");
    ok("advisory locks");
    await c.query("CREATE TABLE hushush_preflight (id INT PRIMARY KEY, v TEXT)");
    await c.query(`CREATE FUNCTION hushush_preflight_guard() RETURNS trigger LANGUAGE plpgsql AS $$
      BEGIN IF NEW.v = 'bad' THEN RAISE EXCEPTION 'guarded'; END IF; RETURN NEW; END $$`);
    await c.query(`CREATE TRIGGER hushush_preflight_guard BEFORE INSERT ON hushush_preflight
      FOR EACH ROW EXECUTE FUNCTION hushush_preflight_guard()`);
    await c.query("ALTER TABLE hushush_preflight ENABLE ROW LEVEL SECURITY");
    await c.query("INSERT INTO hushush_preflight VALUES (1, 'good')");
    await c.query("SAVEPOINT s");
    try {
      await c.query("INSERT INTO hushush_preflight VALUES (2, 'bad')");
      bad("trigger did not fire");
    } catch {
      await c.query("ROLLBACK TO SAVEPOINT s");
      ok("can create tables, PL/pgSQL functions, triggers and row-level security (migrations)");
    }
    const rows = (await c.query("SELECT count(*)::int n FROM hushush_preflight")).rows[0].n;
    rows === 1 ? ok("table owner is not blocked by row-level security") : bad("owner blocked by RLS");
  } finally {
    await c.query("ROLLBACK");
  }

  const lim = (await c.query("SHOW max_connections")).rows[0].max_connections;
  const used = (await c.query("SELECT count(*)::int n FROM pg_stat_activity")).rows[0].n;
  ok(`max_connections ${lim}, in use now ${used} (behind a pooler the pool size is the real limit: keep PAYROLL_DB_POOL_SIZE≈5, DATABASE_POOL_MAX≈2)`);

  const migrated = (await c.query("SELECT to_regclass('payouts') IS NOT NULL AS m")).rows[0].m;
  if (migrated) {
    const { rows: t } = await c.query(`SELECT c.relname, c.relrowsecurity FROM pg_class c
      JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = current_schema() AND c.relkind = 'r'`);
    const open = t.filter((x) => !x.relrowsecurity).map((x) => x.relname);
    open.length ? bad(`tables WITHOUT row-level security: ${open.join(", ")}`) : ok(`all ${t.length} tables have row-level security`);
  } else {
    ok("hushush not migrated yet (payroll-api migrates on first start, then `npm run db:migrate`)");
  }
} catch (e) {
  bad(e.message);
} finally {
  await c.end();
}
console.log(failures ? `\n${failures} check(s) FAILED` : "\nALL CHECKS PASSED");
process.exit(failures ? 1 : 0);

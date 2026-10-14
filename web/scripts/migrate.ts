// Applies web-app migrations (migrations/*.sql) in order, once each.
// Usage: DATABASE_URL=… node --experimental-strip-types scripts/migrate.ts
// The payroll service must have migrated first (payroll-api does so on start).
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import pg from "pg";
import { pgConfig } from "../src/lib/pg-config.ts";

export async function migrate(databaseUrl: string, log = console.log): Promise<string[]> {
  const dir = path.resolve(import.meta.dirname, "..", "migrations");
  const files = readdirSync(dir).filter((f) => /^\d{4}_.+\.sql$/.test(f)).sort();
  const client = new pg.Client(pgConfig(databaseUrl));
  await client.connect();
  const applied: string[] = [];
  try {
    await client.query("BEGIN");
    await client.query("SELECT pg_advisory_xact_lock(hashtext('hushush_web_migrations'))");
    const { rows: hasUsers } = await client.query(
      "SELECT to_regclass('users') IS NOT NULL AS ok",
    );
    if (!hasUsers[0]?.ok) {
      throw new Error("payroll schema missing: start payroll-api (it migrates) before migrating the web app");
    }
    await client.query(`CREATE TABLE IF NOT EXISTS web_schema_migrations (
      name TEXT PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())`);
    const { rows } = await client.query<{ name: string }>("SELECT name FROM web_schema_migrations");
    const done = new Set(rows.map((r) => r.name));
    for (const f of files) {
      if (done.has(f)) continue;
      await client.query(readFileSync(path.join(dir, f), "utf8"));
      await client.query("INSERT INTO web_schema_migrations (name) VALUES ($1)", [f]);
      applied.push(f);
    }
    await client.query("COMMIT");
  } catch (e) {
    await client.query("ROLLBACK").catch(() => {});
    throw e;
  } finally {
    await client.end();
  }
  log(applied.length ? `applied: ${applied.join(", ")}` : "web schema up to date");
  return applied;
}

if (process.argv[1]?.endsWith("migrate.ts")) {
  const url = process.env.DATABASE_URL;
  if (!url) {
    console.error("DATABASE_URL is required");
    process.exit(1);
  }
  migrate(url).catch((e) => {
    console.error(e);
    process.exit(1);
  });
}

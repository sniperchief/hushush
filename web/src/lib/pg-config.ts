import type { PoolConfig } from "pg";

/**
 * node-postgres connection config, shared by the app, the migrator and the
 * test harness (so no `server-only` import here).
 *
 * `DATABASE_CA_PEM` adds a trusted CA for the database's TLS certificate —
 * e.g. Supabase signs database certificates with its own root CA. Because an
 * `sslmode` in the URL would override an explicit `ssl` option, the URL's TLS
 * parameters are replaced by full verification against that CA. Literal `\n`
 * sequences are accepted (single-line secret stores such as Vercel's).
 */
export function pgConfig(url: string, env: NodeJS.ProcessEnv = process.env): PoolConfig {
  const pem = (env.DATABASE_CA_PEM ?? "").replace(/\\n/g, "\n").trim();
  if (!pem) return { connectionString: url };
  if (!pem.includes("BEGIN CERTIFICATE")) throw new Error("DATABASE_CA_PEM is not a PEM certificate");
  const u = new URL(url);
  for (const k of ["sslmode", "sslrootcert", "sslcert", "sslkey", "uselibpqcompat"]) u.searchParams.delete(k);
  return { connectionString: u.toString(), ssl: { ca: pem, rejectUnauthorized: true } };
}

/** Maximum connections per process (keep small on serverless + hosted poolers). */
export function poolMax(env: NodeJS.ProcessEnv = process.env): number {
  const n = Number(env.DATABASE_POOL_MAX ?? 10);
  return Number.isInteger(n) && n > 0 ? n : 10;
}

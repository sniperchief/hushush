import "server-only";
import pg from "pg";
import { env } from "./env";

// Return BIGINT as string (never as a lossy JS number).
pg.types.setTypeParser(20, (v) => v);

let pool: pg.Pool | undefined;

export function db(): pg.Pool {
  pool ??= new pg.Pool({ connectionString: env.databaseUrl, max: 10 });
  return pool;
}

/** For tests: close and forget the pool. */
export async function closeDb(): Promise<void> {
  await pool?.end();
  pool = undefined;
}

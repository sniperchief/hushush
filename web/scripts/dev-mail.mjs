// Prints the most recent development emails (verification and invite links).
// Without SMTP_URL, the app stores outgoing mail in the dev_outbox table.
// Usage: DATABASE_URL=… node scripts/dev-mail.mjs [count]
import pg from "pg";

const n = Number(process.argv[2] ?? 5);
const c = new pg.Client({ connectionString: process.env.DATABASE_URL });
await c.connect();
const { rows } = await c.query(
  "SELECT to_address, subject, body, created_at FROM dev_outbox ORDER BY id DESC LIMIT $1",
  [n],
);
for (const r of rows.reverse()) {
  const links = r.body.match(/https?:\/\/\S+/g) ?? [];
  console.log(`\n${r.created_at.toISOString()}  to: ${r.to_address}\n  ${r.subject}`);
  for (const l of links) console.log(`  → ${l}`);
}
if (!rows.length) console.log("No dev emails yet. Sign up or send an invite first.");
await c.end();

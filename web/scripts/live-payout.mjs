// LIVE TESTNET payout driver for scripts/phase3_live_e2e.sh (repo root).
// Drives the RUNNING app (`next start`) over real HTTP with real cookies, the
// same way a browser would; reads payout progress from payroll-api.
//
//   node scripts/live-payout.mjs setup
//   node scripts/live-payout.mjs claim <zec>                 → claim id
//   node scripts/live-payout.mjs quote <claimId> <address>
//   node scripts/live-payout.mjs pay <claimId> <address>
//   node scripts/live-payout.mjs show <claimId>
//   node scripts/live-payout.mjs wait <claimId> <payout-state> <timeout-secs>
//   node scripts/live-payout.mjs pages <claimId>
//
// Env: APP_URL, DATABASE_URL (for dev_outbox links), PAYROLL_API_URL,
// PAYROLL_API_TOKEN, STATE_FILE (outside the repo: holds test session cookies).
import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import pg from "pg";

const APP = process.env.APP_URL ?? "http://localhost:3100";
const STATE = process.env.STATE_FILE;
if (!STATE) throw new Error("STATE_FILE is required");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const out = (v) => console.log(typeof v === "string" ? v : JSON.stringify(v));
const die = (m) => {
  console.error(`FAIL: ${m}`);
  process.exit(1);
};

async function req(cookie, method, path, body) {
  const res = await fetch(APP + path, {
    method,
    redirect: "manual",
    headers: {
      ...(cookie ? { cookie } : {}),
      ...(method !== "GET" ? { origin: APP, "content-type": "application/json" } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let json;
  try {
    json = JSON.parse(text);
  } catch {
    json = undefined;
  }
  return { status: res.status, text, json, setCookie: res.headers.get("set-cookie") };
}

async function account(db, name) {
  const email = `${name.toLowerCase()}-${randomUUID().slice(0, 6)}@example.test`;
  const password = "correct horse battery staple";
  let r = await req("", "POST", "/api/auth/signup", { email, displayName: name, password });
  if (r.status !== 202) die(`signup ${r.status}`);
  let token;
  for (let i = 0; i < 30 && !token; i++) {
    const { rows } = await db.query("SELECT body FROM dev_outbox WHERE to_address = $1 ORDER BY id DESC LIMIT 1", [email]);
    token = rows[0]?.body.match(/\/verify\?token=([\w-]+)/)?.[1];
    if (!token) await sleep(200);
  }
  if (!token) die("no verification mail");
  if ((await req("", "POST", "/api/auth/verify", { token })).status !== 200) die("verify failed");
  r = await req("", "POST", "/api/auth/login", { email, password });
  if (r.status !== 200) die(`login ${r.status}`);
  const cookie = r.setCookie.split(";")[0];
  const { rows } = await db.query("SELECT id FROM users WHERE lower(email) = lower($1)", [email]);
  return { email, cookie, id: rows[0].id };
}

const load = () => JSON.parse(readFileSync(STATE, "utf8"));

async function show(claimId) {
  const s = load();
  const res = await fetch(`${process.env.PAYROLL_API_URL}/v1/streams/${s.streamId}`, {
    headers: { authorization: `Bearer ${process.env.PAYROLL_API_TOKEN}`, "x-actor-id": s.worker.id },
  });
  if (!res.ok) throw new Error(`payroll-api ${res.status}`);
  const d = await res.json();
  const c = d.claims.find((c) => c.id === claimId);
  if (!c) die(`no claim ${claimId}`);
  return { claim: c, balance: d.stream.balance };
}

const [cmd, ...args] = process.argv.slice(2);
switch (cmd) {
  case "setup": {
    const db = new pg.Client({ connectionString: process.env.DATABASE_URL });
    await db.connect();
    const owner = await account(db, "LiveOwner");
    const worker = await account(db, "LiveContributor");
    await db.end();
    let r = await req(owner.cookie, "POST", "/api/orgs", { name: "Live Payout Co (testnet)" });
    if (r.status !== 201) die(`org ${r.status}`);
    const orgId = r.json.id;
    r = await req(owner.cookie, "POST", `/api/orgs/${orgId}/invitations`, { email: worker.email, role: "member" });
    if (r.status !== 201) die(`invite ${r.status} ${r.text}`);
    const db2 = new pg.Client({ connectionString: process.env.DATABASE_URL });
    await db2.connect();
    const { rows } = await db2.query("SELECT body FROM dev_outbox WHERE to_address = $1 ORDER BY id DESC LIMIT 1", [worker.email]);
    await db2.end();
    const inviteToken = rows[0].body.match(/\/invite\/([\w-]+)/)[1];
    if ((await req(worker.cookie, "POST", "/api/invitations/accept", { token: inviteToken })).status !== 200) die("accept");
    // 0.36 ZEC/hour = 10,000 zats/second, so a few seconds earn the payouts.
    r = await req(owner.cookie, "POST", `/api/orgs/${orgId}/streams`, {
      recipientUserId: worker.id,
      rateZec: "0.36",
      ratePeriodSecs: 3600,
      startAt: null,
      endAt: null,
    });
    if (r.status !== 201) die(`stream ${r.status} ${r.text}`);
    const streamId = r.json.id;
    await sleep(60_000);
    r = await req(owner.cookie, "POST", `/api/streams/${streamId}/lifecycle/pause`);
    if (r.status !== 200) die(`pause ${r.status}`);
    writeFileSync(STATE, JSON.stringify({ owner, worker, orgId, streamId }, null, 2));
    out({ orgId, streamId, earned_zec: r.json.balance.earned_zec, owner: owner.id, worker: worker.id });
    break;
  }
  case "claim": {
    const s = load();
    const r = await req(s.worker.cookie, "POST", `/api/streams/${s.streamId}/claims`, { amountZec: args[0], idempotencyKey: randomUUID() });
    if (r.status !== 201) die(`claim ${r.status} ${r.text}`);
    out(r.json.claim.id);
    break;
  }
  case "quote": {
    const s = load();
    const r = await req(s.worker.cookie, "POST", `/api/claims/${args[0]}/payout/quote`, { address: args[1] });
    out({ status: r.status, ...r.json });
    break;
  }
  case "pay": {
    const s = load();
    const t0 = Date.now();
    const r = await req(s.worker.cookie, "POST", `/api/claims/${args[0]}/payout`, { address: args[1] });
    out({ status: r.status, secs: Math.round((Date.now() - t0) / 1000), ...(r.json ?? { text: r.text.slice(0, 200) }) });
    break;
  }
  case "show":
    out(await show(args[0]));
    break;
  case "wait": {
    const [claimId, want, secs] = args;
    const until = Date.now() + Number(secs) * 1000;
    let last = "";
    for (;;) {
      let line;
      try {
        const { claim } = await show(claimId);
        const p = claim.payout;
        line = `claim=${claim.status} payout=${p?.state} conf=${p?.confirmations}/${p?.required_confirmations} mined=${p?.mined_height} txid=${p?.txid} detail=${p?.detail ?? ""}`;
        if (p?.state === want) {
          out(`[${new Date().toISOString()}] ${line}`);
          break;
        }
      } catch (e) {
        line = `payroll-api unreachable (${e.message})`;
      }
      if (line !== last) out(`[${new Date().toISOString()}] ${line}`);
      last = line;
      if (Date.now() > until) die(`timed out waiting for ${want}`);
      await sleep(10_000);
    }
    break;
  }
  case "kill-when-reserved": {
    // Crash test: hard-kill payroll-api (Windows pid) the moment the payout
    // row exists in `reserved` state, i.e. while its wallet call is in flight.
    const [claimId, winpid, secs] = args;
    const db = new pg.Client({ connectionString: process.env.DATABASE_URL });
    await db.connect();
    const until = Date.now() + Number(secs ?? 120) * 1000;
    for (;;) {
      const { rows } = await db.query("SELECT id, state FROM payouts WHERE claim_id = $1", [claimId]);
      if (rows[0]) {
        const killed = rows[0].state === "reserved";
        if (killed) (await import("node:child_process")).execSync(`taskkill /F /PID ${winpid}`, { stdio: "ignore" });
        out({ killed, payout_id: rows[0].id, state_at_kill: rows[0].state });
        break;
      }
      if (Date.now() > until) die("payout row never appeared");
      await sleep(25);
    }
    await db.end();
    break;
  }
  case "payout-of": {
    // Reads the payout straight from the database (payroll-api may be down).
    const db = new pg.Client({ connectionString: process.env.DATABASE_URL });
    await db.connect();
    const { rows } = await db.query("SELECT id, state, txid, lease_owner IS NOT NULL AS leased FROM payouts WHERE claim_id = $1", [args[0]]);
    await db.end();
    out(rows[0] ?? null);
    break;
  }
  case "wages": {
    // The contributor's stream page shows the server-computed earned and
    // available wages, and they match what payroll-api reports.
    const s = load();
    const res = await fetch(`${process.env.PAYROLL_API_URL}/v1/streams/${s.streamId}`, {
      headers: { authorization: `Bearer ${process.env.PAYROLL_API_TOKEN}`, "x-actor-id": s.worker.id },
    });
    const b = (await res.json()).stream.balance;
    const page = await req(s.worker.cookie, "GET", `/app/streams/${s.streamId}`);
    const owner = await req(s.owner.cookie, "GET", `/app/orgs/${s.orgId}`);
    const shown = page.text.includes(b.earned_zec) && page.text.includes(b.available_zec);
    if (page.status !== 200 || !shown || owner.status !== 200) die("wages not shown");
    out({ contributor_page: page.status, earned_zec: b.earned_zec, available_zec: b.available_zec, shown_on_page: shown, employer_org_page: owner.status });
    break;
  }
  case "events": {
    // The payout's state history, straight from the append-only audit table.
    const db = new pg.Client({ connectionString: process.env.DATABASE_URL });
    await db.connect();
    const { rows } = await db.query(
      `SELECT string_agg(e.to_state, ' > ' ORDER BY e.id) AS h FROM payouts p
       JOIN payout_events e ON e.payout_id = p.id WHERE p.claim_id = $1`,
      [args[0]],
    );
    await db.end();
    out(rows[0]?.h ?? null);
    break;
  }
  case "pages": {
    const s = load();
    const w = await req(s.worker.cookie, "GET", `/app/streams/${s.streamId}`);
    const o = await req(s.owner.cookie, "GET", `/app/orgs/${s.orgId}`);
    const labels = ["Paid · payout confirmed", "Pending · unpaid", "Sent ·", "Checking payout", "Payout failed"].filter((l) => w.text.includes(l));
    out({
      contributor_stream_page: w.status,
      claim_labels_on_page: labels,
      owner_org_page: o.status,
      owner_sees_payout_wallet: o.text.includes("Payout wallet") && o.text.includes("Funding address"),
    });
    break;
  }
  default:
    die(`unknown command ${cmd}`);
}

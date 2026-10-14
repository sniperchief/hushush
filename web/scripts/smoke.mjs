// End-to-end smoke test against a RUNNING app (`next start`) + payroll-api.
// Exercises the main employer and contributor flows over real HTTP with real
// cookies, and checks that the server-rendered pages show the right data.
//
// Usage: APP_URL=http://localhost:3000 DATABASE_URL=… node scripts/smoke.mjs
// (Reads verification/invite links from the dev_outbox table.)
import { randomUUID } from "node:crypto";
import pg from "pg";

const APP = process.env.APP_URL ?? "http://localhost:3000";
// Same TLS rule as src/lib/pg-config.ts (DATABASE_CA_PEM, e.g. Supabase's root CA).
function dbConfig(url) {
  const pem = (process.env.DATABASE_CA_PEM ?? "").replace(/\\n/g, "\n").trim();
  if (!pem) return { connectionString: url };
  const u = new URL(url);
  for (const k of ["sslmode", "sslrootcert", "sslcert", "sslkey", "uselibpqcompat"]) u.searchParams.delete(k);
  return { connectionString: u.toString(), ssl: { ca: pem, rejectUnauthorized: true } };
}
const db = new pg.Client(dbConfig(process.env.DATABASE_URL));
await db.connect();

let step = 0;
const log = (m) => console.log(`${String(++step).padStart(2)}. ${m}`);
const fail = (m) => {
  console.error(`FAIL: ${m}`);
  process.exit(1);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

class Browser {
  cookie = "";
  async req(method, path, body) {
    const res = await fetch(APP + path, {
      method,
      redirect: "manual",
      headers: {
        ...(this.cookie ? { cookie: this.cookie } : {}),
        ...(body !== undefined ? { "content-type": "application/json", origin: APP } : method !== "GET" ? { origin: APP } : {}),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const sc = res.headers.get("set-cookie");
    if (sc) this.cookie = sc.split(";")[0];
    const text = await res.text();
    let json;
    try {
      json = JSON.parse(text);
    } catch {
      json = undefined;
    }
    return { status: res.status, text, json, location: res.headers.get("location") };
  }
  post(path, body) {
    return this.req("POST", path, body ?? {});
  }
  get(path) {
    return this.req("GET", path);
  }
}

async function mailTo(email, re) {
  for (let i = 0; i < 20; i++) {
    const { rows } = await db.query("SELECT body FROM dev_outbox WHERE to_address = $1 ORDER BY id DESC LIMIT 1", [email]);
    const m = rows[0]?.body.match(re);
    if (m) return m[1];
    await sleep(200);
  }
  fail(`no mail for ${email}`);
}

async function account(name) {
  const b = new Browser();
  const email = `${name.toLowerCase()}-${randomUUID().slice(0, 6)}@example.test`;
  const pw = "correct horse battery staple";
  let r = await b.post("/api/auth/signup", { email, displayName: name, password: pw });
  if (r.status !== 202) fail(`signup ${r.status} ${r.text}`);
  r = await b.post("/api/auth/login", { email, password: pw });
  if (r.status !== 403) fail(`unverified login should be 403, got ${r.status}`);
  const token = await mailTo(email, /\/verify\?token=([\w-]+)/);
  const page = await b.get(`/verify?token=${token}`);
  if (page.status !== 200 || !page.text.includes("Verify my email")) fail("verify page did not render");
  r = await b.post("/api/auth/verify", { token });
  if (r.status !== 200) fail(`verify ${r.status}`);
  r = await b.post("/api/auth/login", { email, password: pw });
  if (r.status !== 200 || !b.cookie) fail(`login ${r.status}`);
  log(`${name}: signed up, verified email via link, signed in`);
  return { b, email };
}

const must = (cond, msg) => (cond ? log(msg) : fail(msg));

// --- Employer -----------------------------------------------------------------
const anon = new Browser();
must((await anon.get("/app")).status === 307, "signed-out /app redirects to /login");

const emp = await account("Employer");
let r = await emp.b.post("/api/orgs", { name: "Smoke Test Labs" });
if (r.status !== 201) fail(`create org ${r.status}`);
const orgId = r.json.id;
log("employer created organization");

const con = await account("Contributor");
r = await emp.b.post(`/api/orgs/${orgId}/invitations`, { email: con.email, role: "member" });
must(r.status === 201, "employer invited contributor by email");
const inviteToken = await mailTo(con.email, /\/invite\/([\w-]+)/);
const invPage = await con.b.get(`/invite/${inviteToken}`);
must(invPage.status === 200 && invPage.text.includes("Smoke Test Labs") && invPage.text.includes("Accept invitation"), "invite page shows org and Accept button");
r = await con.b.post("/api/invitations/accept", { token: inviteToken });
must(r.status === 200 && r.json.organizationId === orgId, "contributor accepted invitation");

const orgPage = await emp.b.get(`/app/orgs/${orgId}`);
must(orgPage.status === 200 && orgPage.text.includes(con.email) && orgPage.text.includes("New stream"), "org page lists contributor and stream form");
const { rows: [conUser] } = await db.query("SELECT id FROM users WHERE email = $1", [con.email]);

r = await emp.b.post(`/api/orgs/${orgId}/streams`, { recipientUserId: conUser.id, rateZec: "3600", ratePeriodSecs: 3600, startAt: null, endAt: null });
must(r.status === 201 && r.json.rate_zec === "3600.00000000", "employer created a 3600 ZEC/hour stream");
const streamId = r.json.id;
r = await emp.b.post(`/api/orgs/${orgId}/streams`, { recipientUserId: conUser.id, rateZec: "1", ratePeriodSecs: 3600, startAt: Math.floor(Date.now() / 1000) - 600, endAt: null });
must(r.status === 422, `backdated stream rejected (${r.json?.message})`);

await sleep(2100);
r = await emp.b.post(`/api/streams/${streamId}/lifecycle/pause`);
must(r.status === 200 && r.json.status === "paused", `paused; earned ${r.json.balance.earned_zec} ZEC`);
const earnedAtPause = r.json.balance.earned_zats;
await sleep(1500);
const sp = await emp.b.get(`/app/streams/${streamId}`);
must(sp.status === 200 && sp.text.includes(r.json.balance.earned_zec) && sp.text.includes("Accrual is paused"), "stream page shows frozen earnings while paused");
r = await emp.b.post(`/api/streams/${streamId}/lifecycle/resume`);
must(r.status === 200 && r.json.status === "active", "resumed");
await sleep(1100);
r = await emp.b.post(`/api/streams/${streamId}/lifecycle/pause`);
must(r.json.balance.earned_zats > earnedAtPause, "accrual continued after resume, then paused again");
const earned = r.json.balance.earned_zats;

// --- Contributor ----------------------------------------------------------------
const dash = await con.b.get("/app");
must(dash.status === 200 && dash.text.includes("Smoke Test Labs") && dash.text.includes("My payroll streams"), "contributor dashboard lists the stream");

const key = randomUUID();
r = await con.b.post(`/api/streams/${streamId}/claims`, { amountZec: "1.5", idempotencyKey: key });
must(r.status === 201 && r.json.claim.status === "reserved", "contributor requested a partial claim (1.5 ZEC)");
r = await con.b.post(`/api/streams/${streamId}/claims`, { amountZec: "1.5", idempotencyKey: key });
must(r.status === 200 && r.json.duplicate === true, "duplicate submission returned the original claim");

const detail = await con.b.get(`/app/streams/${streamId}`);
must(detail.text.includes("Pending · unpaid") && !detail.text.includes("Paid · payout confirmed"), "claim shown as 'Pending · unpaid', never as paid");
must(detail.text.includes("Request a claim"), "contributor sees the claim form");
must(!detail.text.includes("End stream"), "contributor does not see employer controls");

const avail = earned - 150_000_000;
const availZec = `${Math.floor(avail / 1e8)}.${String(avail % 1e8).padStart(8, "0")}`;
r = await con.b.post(`/api/streams/${streamId}/claims`, { amountZec: availZec, idempotencyKey: randomUUID() });
must(r.status === 201, `full claim of remaining ${availZec} ZEC reserved`);
r = await con.b.post(`/api/streams/${streamId}/claims`, { amountZec: "0.00000001", idempotencyKey: randomUUID() });
must(r.status === 409, `over-claim rejected: ${r.json.message}`);

r = await con.b.post(`/api/streams/${streamId}/lifecycle/end`);
must(r.status === 403, "contributor cannot end the stream");
r = await emp.b.post(`/api/streams/${streamId}/claims`, { amountZec: "0.1", idempotencyKey: randomUUID() });
must(r.status === 403, "employer cannot claim the contributor's wages");

// --- Cross-organization -----------------------------------------------------------
const rival = await account("Rival");
r = await rival.b.post(`/api/streams/${streamId}/lifecycle/pause`);
must(r.status === 404, "outsider cannot touch the stream (404)");
must((await rival.b.get(`/app/streams/${streamId}`)).status === 404, "outsider gets 404 for the stream page");
must((await rival.b.get(`/app/orgs/${orgId}`)).status === 404, "outsider gets 404 for the org page");

// --- End ---------------------------------------------------------------------------
r = await emp.b.post(`/api/streams/${streamId}/lifecycle/end`);
must(r.status === 200 && r.json.status === "ended" && r.json.balance.earned_zats === earned, "employer ended the stream; earned wages preserved");
const hist = await emp.b.get(`/app/streams/${streamId}`);
must(["Stream created", "Paused", "Resumed", "Ended"].every((t) => hist.text.includes(t)), "history shows created → paused → resumed → paused → ended");

r = await emp.b.post("/api/auth/logout");
must(r.status === 200 && (await emp.b.get("/app")).status === 307, "logout ends the session");

console.log("\nSMOKE PASS");
await db.end();

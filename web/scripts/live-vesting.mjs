// LIVE TESTNET driver for payout-wallet/scripts/live_vesting.sh (Vesting Phase 3).
// Drives the RUNNING app (`next start`) over real HTTP with real cookies —
// employer and employee in separate cookie jars. Every mutation goes through
// the web route handlers. Reads of the agreement come from payroll-api
// (GET /v1/vesting/{id} as the employee): the server-side accounting.
//
//   node scripts/live-vesting.mjs setup <total-zec> <start-in-secs> → agreement id (employee joins via a worker link first)
//   node scripts/live-vesting.mjs wait-funded <zats> <timeout-secs>
//   node scripts/live-vesting.mjs address <address>              → HTTP status + body
//   node scripts/live-vesting.mjs quote <zec>
//   node scripts/live-vesting.mjs withdraw <zec> [key]           → one request
//   node scripts/live-vesting.mjs wait-available <zats> <timeout-secs>
//   node scripts/live-vesting.mjs wait <payout-id> <state> <timeout-secs>
//   node scripts/live-vesting.mjs show
//   node scripts/live-vesting.mjs pages <address> <txid>
//   node scripts/live-vesting.mjs create <label> <total-zec> <start-in-secs> → another agreement (same employee), activated
//   node scripts/live-vesting.mjs cancel                         → employer cancels the selected agreement
//
// AGREEMENT=<label> selects an agreement made by `create` (default: the one from `setup`).
//
// Env: APP_URL, STATE_FILE (outside the repo), PAYROLL_API_URL, PAYROLL_API_TOKEN.
import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { Browser } from "./lib/browser.mjs";

const APP = process.env.APP_URL ?? "http://localhost:3300";
const API = process.env.PAYROLL_API_URL ?? "http://127.0.0.1:7980";
const TOKEN = process.env.PAYROLL_API_TOKEN;
const STATE = process.env.STATE_FILE;
if (!STATE) throw new Error("STATE_FILE is required");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const out = (v) => console.log(typeof v === "string" ? v : JSON.stringify(v));
const die = (m) => {
  console.error(`FAIL: ${m}`);
  process.exit(1);
};
const state = () => {
  const s = JSON.parse(readFileSync(STATE, "utf8"));
  const label = process.env.AGREEMENT;
  if (label) {
    if (!s.agreements?.[label]) die(`no agreement labelled ${label}`);
    s.agreementId = s.agreements[label];
  }
  return s;
};
const browser = (session) => {
  const b = new Browser(APP);
  b.session = session;
  return b;
};

/** The agreement as the employee sees it, from payroll-api (server accounting). */
async function detail(s = state()) {
  if (!TOKEN) die("PAYROLL_API_TOKEN is required");
  const r = await fetch(`${API}/v1/vesting/${s.agreementId}`, {
    headers: { authorization: `Bearer ${TOKEN}`, "x-actor-id": s.employeeId },
  });
  if (r.status !== 200) die(`vesting ${r.status} ${await r.text()}`);
  return r.json();
}

const summary = (d) => ({
  phase: d.agreement.phase,
  status: d.agreement.status,
  cancelled_at: d.agreement.cancelled_at,
  balance: Object.fromEntries(Object.entries(d.agreement.balance).filter(([k]) => k.endsWith("_zats") || k === "as_of")),
  withdrawable_zats: d.withdrawable_zats,
  address_locked: d.address_locked,
  claims: d.claims.map((c) => ({ id: c.id, status: c.status, amount_zats: c.amount_zats, payout: c.payout?.id ?? null })),
  payouts: d.payouts.map((p) => ({ id: p.id, type: p.compensation_type, state: p.state, amount_zats: p.amount_zats, fee_estimate_zats: p.fee_estimate_zats, fee_zats: p.fee_zats, txid: p.txid, mined_height: p.mined_height, confirmations: p.confirmations, required: p.required_confirmations, submitted_at: p.submitted_at, resolved_at: p.resolved_at, detail: p.detail })),
});

const [cmd, ...args] = process.argv.slice(2);
switch (cmd) {
  case "setup": {
    const [total = "0.05", startIn = "90"] = args;
    const boss = new Browser(APP);
    await boss.signUp("employer", "Live Vesting Employer");
    let r = await boss.post("/api/company", { name: `Live Vesting Co ${randomUUID().slice(0, 4)}` });
    if (r.status !== 201) die(`company ${r.status} ${r.text}`);
    r = await boss.post("/api/company/faucet", { idempotencyKey: randomUUID() });
    if (r.status !== 201) die(`faucet ${r.status} ${r.text}`);
    const faucet = { payment_id: r.json.payment_id, state: r.json.state, amount_zats: r.json.amount_zats, txid: r.json.txid };
    // Membership: employees join through a worker link (there is no vesting-only invite).
    // The stream is tiny and is never withdrawn from in this run.
    r = await boss.post("/api/streams", { workerName: "Vesting Employee", totalZec: "0.0001", startAt: null, endAt: Math.floor(Date.now() / 1000) + 86_400 });
    if (r.status !== 201) die(`stream ${r.status} ${r.text}`);
    const link = new URL(r.json.link);
    const emp = new Browser(APP);
    await emp.signUp("worker", "Vesting Employee");
    r = await emp.post(`/api/join/${link.pathname.split("/join/")[1]}`);
    if (r.status !== 201) die(`join ${r.status} ${r.text}`);
    const streamId = r.json.streamId;
    r = await emp.get(`/api/streams/${streamId}`);
    if (r.status !== 200) die(`stream view ${r.status} ${r.text}`);
    const employeeId = r.json.recipient_user_id;
    const organizationId = r.json.organization_id;
    const startAt = Math.floor(Date.now() / 1000) + Number(startIn);
    r = await boss.post("/api/vesting", { employeeUserId: employeeId, totalZec: total, startAt, durationDays: 1, cliffDays: null });
    if (r.status !== 201) die(`create vesting ${r.status} ${r.text}`);
    const agreementId = r.json.agreement.id;
    const draft = r.json.agreement;
    r = await boss.post(`/api/vesting/${agreementId}/activate`);
    if (r.status !== 200) die(`activate ${r.status} ${r.text}`);
    writeFileSync(STATE, JSON.stringify({ agreementId, streamId, employeeId, organizationId, boss: boss.session, employee: emp.session }));
    out({ agreementId, organizationId, status: r.json.status, phase: r.json.phase, total_zats: draft.total_zats, start_at: draft.start_at, end_at: draft.end_at, cliff_secs: draft.cliff_secs, faucet });
    break;
  }
  case "wait-funded": {
    const [zats = "1", secs = "900"] = args;
    const until = Date.now() + Number(secs) * 1000;
    for (;;) {
      const r = await browser(state().boss).get("/api/company/wallet");
      if (r.status !== 200) die(`wallet ${r.status} ${r.text}`);
      if (r.json.spendable_zats >= Number(zats)) {
        out({ spendable_zats: r.json.spendable_zats, total_zats: r.json.total_zats ?? null });
        break;
      }
      if (Date.now() > until) die(`company wallet not funded in time (spendable ${r.json.spendable_zats}, pending ${r.json.pending_zats})`);
      await sleep(10_000);
    }
    break;
  }
  case "address": {
    const r = await browser(state().employee).post(`/api/vesting/${state().agreementId}/address`, { address: args[0] });
    out({ status: r.status, error: r.json?.error, message: r.json?.message, address_locked: r.json?.address_locked });
    break;
  }
  case "quote": {
    const r = await browser(state().employee).post(`/api/vesting/${state().agreementId}/quote`, { amountZec: args[0] });
    out({ status: r.status, ...(r.json ?? {}) });
    break;
  }
  case "withdraw": {
    const [amountZec, key = randomUUID()] = args;
    const r = await browser(state().employee).post(`/api/vesting/${state().agreementId}/withdraw`, { amountZec, idempotencyKey: key });
    const p = r.json?.payout;
    out({ status: r.status, key, duplicate: r.json?.duplicate ?? null, error: r.json?.error, message: r.json?.message, payout: p ? { id: p.id, type: p.compensation_type, state: p.state, amount_zats: p.amount_zats, fee_estimate_zats: p.fee_estimate_zats, txid: p.txid } : null });
    break;
  }
  case "wait-available": {
    const [zats, secs = "1800"] = args;
    const until = Date.now() + Number(secs) * 1000;
    for (;;) {
      const d = await detail();
      if (d.agreement.balance.available_zats >= Number(zats)) {
        out(summary(d));
        break;
      }
      if (Date.now() > until) die(`not available in time: ${JSON.stringify(d.agreement.balance)}`);
      await sleep(15_000);
    }
    break;
  }
  case "wait": {
    const [id, want, secs = "3000"] = args;
    const until = Date.now() + Number(secs) * 1000;
    let last = "";
    for (;;) {
      const p = (await detail()).payouts.find((x) => x.id === id);
      if (!p) die(`no payout ${id}`);
      const now = `${p.state} ${p.confirmations}/${p.required_confirmations}${p.mined_height ? ` mined ${p.mined_height}` : ""}`;
      if (now !== last) console.log(`[${new Date().toISOString().slice(11, 19)}] payout ${id.slice(0, 8)}: ${now}`);
      last = now;
      if (p.state === want) break;
      if (p.state === "confirmed" || p.state === "failed") die(`payout ended ${p.state}, wanted ${want}: ${p.detail}`);
      if (Date.now() > until) die(`timed out waiting for ${want}`);
      await sleep(10_000);
    }
    break;
  }
  case "show":
    out(summary(await detail()));
    break;
  case "create": {
    const [label, total = "1", startIn = "90"] = args;
    const raw = JSON.parse(readFileSync(STATE, "utf8"));
    const boss = browser(raw.boss);
    const startAt = Math.floor(Date.now() / 1000) + Number(startIn);
    let r = await boss.post("/api/vesting", { employeeUserId: raw.employeeId, totalZec: total, startAt, durationDays: 1, cliffDays: null });
    if (r.status !== 201) die(`create vesting ${r.status} ${r.text}`);
    const id = r.json.agreement.id;
    r = await boss.post(`/api/vesting/${id}/activate`);
    if (r.status !== 200) die(`activate ${r.status} ${r.text}`);
    raw.agreements = { ...(raw.agreements ?? {}), [label]: id };
    writeFileSync(STATE, JSON.stringify(raw));
    out({ label, agreementId: id, status: r.json.status, phase: r.json.phase, total_zats: r.json.total_zats, start_at: r.json.start_at });
    break;
  }
  case "cancel": {
    const r = await browser(state().boss).post(`/api/vesting/${state().agreementId}/cancel`);
    out({ status: r.status, agreement_status: r.json?.status, phase: r.json?.phase, error: r.json?.error, message: r.json?.message, balance: r.json?.balance ? Object.fromEntries(Object.entries(r.json.balance).filter(([k]) => k.endsWith("_zats"))) : null });
    break;
  }
  case "pages": {
    const [address, txid] = args;
    const st = state();
    const e = await browser(st.employee).get(`/app/vesting/${st.agreementId}`);
    const m = await browser(st.boss).get(`/app/vesting/${st.agreementId}`);
    const checks = {
      employee_page_200: e.status === 200,
      employee_sees_paid: e.html.includes("Paid ✓"),
      employer_sees_paid: m.html.includes("Paid ✓"),
      employee_sees_txid: e.html.includes(txid.slice(0, 10)),
      employee_sees_own_address: e.html.includes(address),
      employer_page_200: m.status === 200,
      employer_sees_txid: m.html.includes(txid.slice(0, 10)),
      employer_never_sees_address: !m.html.includes(address),
    };
    out(checks);
    if (!Object.values(checks).every(Boolean)) die("page checks failed");
    break;
  }
  default:
    die(`unknown command ${cmd}`);
}

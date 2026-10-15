// LIVE TESTNET driver for payout-wallet/scripts/live_flow.sh (Phase 5 flow).
// Drives the RUNNING app (`next start`) over real HTTP with real cookies —
// the employer and the worker in separate cookie jars — and reads stream
// state through the app (as the worker). Passkeys come from a SOFTWARE
// authenticator.
//
//   node scripts/live-flow.mjs setup <total-zec> <seconds>   → stream id (and asks the faucet to fund the company)
//   node scripts/live-flow.mjs wait-funded <zats> <timeout-secs> → the company wallet's spendable balance
//   node scripts/live-flow.mjs address <address>             → HTTP status + body
//   node scripts/live-flow.mjs withdraw [n]                  → n concurrent taps
//   node scripts/live-flow.mjs wait-withdrawable <timeout-secs>
//   node scripts/live-flow.mjs wait <payout-id> <state> <timeout-secs>
//   node scripts/live-flow.mjs end
//   node scripts/live-flow.mjs show
//   node scripts/live-flow.mjs pages <address> <txid>...
//
// Env: APP_URL, STATE_FILE (outside the repo: holds the test accounts'
// session cookies).
import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { Browser } from "./lib/browser.mjs";

const APP = process.env.APP_URL ?? "http://localhost:3300";
const STATE = process.env.STATE_FILE;
if (!STATE) throw new Error("STATE_FILE is required");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const out = (v) => console.log(typeof v === "string" ? v : JSON.stringify(v));
const die = (m) => {
  console.error(`FAIL: ${m}`);
  process.exit(1);
};
const state = () => JSON.parse(readFileSync(STATE, "utf8"));
const browser = (session) => {
  const b = new Browser(APP);
  b.session = session;
  return b;
};

/** The stream as the worker sees it (GET /api/streams/:id, read-only). */
async function show(s = state()) {
  const r = await browser(s.worker).get(`/api/streams/${s.streamId}`);
  if (r.status !== 200) die(`stream ${r.status} ${r.text}`);
  return r.json;
}

const [cmd, ...args] = process.argv.slice(2);
switch (cmd) {
  case "setup": {
    const [total = "0.001", secs = "600"] = args;
    const boss = new Browser(APP);
    await boss.signUp("employer", "Live Employer");
    let r = await boss.post("/api/company", { name: `Live Testnet Co ${randomUUID().slice(0, 4)}` });
    if (r.status !== 201) die(`company ${r.status} ${r.text}`);
    // Phase 6: the company pays from its own wallet; fund it from the treasury (testnet faucet).
    r = await boss.post("/api/company/faucet", { idempotencyKey: randomUUID() });
    if (r.status !== 201) die(`faucet ${r.status} ${r.text}`);
    const faucet = { payment_id: r.json.payment_id, state: r.json.state, amount_zats: r.json.amount_zats, txid: r.json.txid };
    r = await boss.post("/api/streams", { workerName: "Carol", totalZec: total, startAt: null, endAt: Math.floor(Date.now() / 1000) + Number(secs) });
    if (r.status !== 201) die(`stream ${r.status} ${r.text}`);
    const link = new URL(r.json.link);
    const worker = new Browser(APP);
    const preview = await worker.get(link.pathname);
    if (!preview.html.includes("is paying you")) die("worker link preview missing");
    await worker.signUp("worker", "Carol");
    r = await worker.post(`/api/join/${link.pathname.split("/join/")[1]}`);
    if (r.status !== 201) die(`join ${r.status} ${r.text}`);
    const streamId = r.json.streamId;
    writeFileSync(STATE, JSON.stringify({ streamId, boss: boss.session, worker: worker.session }));
    const s = await show();
    out({ streamId, total_zec: s.total_zec, start_at: s.start_at, end_at: s.end_at, milestone_zats: s.milestone_zats, company: s.organization_name, faucet });
    break;
  }
  case "wait-funded": {
    const [zats = "1", secs = "900"] = args;
    const until = Date.now() + Number(secs) * 1000;
    for (;;) {
      const r = await browser(state().boss).get("/api/company/wallet");
      if (r.status !== 200) die(`wallet ${r.status} ${r.text}`);
      if (r.json.spendable_zats >= Number(zats)) {
        out({ spendable_zats: r.json.spendable_zats, funding_address: r.json.funding_address });
        break;
      }
      if (Date.now() > until) die(`company wallet not funded in time (spendable ${r.json.spendable_zats}, pending ${r.json.pending_zats})`);
      await sleep(10_000);
    }
    break;
  }
  case "address": {
    const r = await browser(state().worker).post(`/api/streams/${state().streamId}/address`, { address: args[0] });
    out({ status: r.status, error: r.json?.error, message: r.json?.message, address_locked: r.json?.address_locked });
    break;
  }
  case "withdraw": {
    // n concurrent taps: the first half share one idempotency key (double
    // taps), the rest each use a new key (refreshes / other tabs).
    const n = Number(args[0] ?? 1);
    const w = browser(state().worker);
    const shared = randomUUID();
    const rs = await Promise.all(
      Array.from({ length: n }, (_, i) => w.post(`/api/streams/${state().streamId}/withdraw`, { idempotencyKey: i < Math.ceil(n / 2) ? shared : randomUUID() })),
    );
    out({
      statuses: rs.map((r) => r.status),
      created: rs.filter((r) => r.status === 201).length,
      payout_ids: [...new Set(rs.map((r) => r.json?.payout?.id).filter(Boolean))],
      states: rs.map((r) => r.json?.payout?.state ?? r.json?.error),
      amount_zats: rs.find((r) => r.json?.payout)?.json.payout.amount_zats,
      txid: rs.find((r) => r.json?.payout?.txid)?.json.payout.txid ?? null,
      messages: [...new Set(rs.map((r) => r.json?.message).filter(Boolean))],
    });
    break;
  }
  case "wait-withdrawable": {
    const until = Date.now() + Number(args[0] ?? 900) * 1000;
    for (;;) {
      const s = await show();
      if (s.withdrawable_zats !== null) {
        out({ withdrawable_zats: s.withdrawable_zats, earned_zats: s.balance.earned_zats, status: s.status });
        break;
      }
      if (Date.now() > until) die(`never withdrawable: ${JSON.stringify(s.balance)}`);
      await sleep(5000);
    }
    break;
  }
  case "wait": {
    const [id, want, secs = "3000"] = args;
    const until = Date.now() + Number(secs) * 1000;
    let last = "";
    for (;;) {
      const p = (await show()).payouts.find((x) => x.id === id);
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
  case "end": {
    const r = await browser(state().boss).post(`/api/streams/${state().streamId}/end`);
    if (r.status !== 200) die(`end ${r.status} ${r.text}`);
    out({ status: r.json.status, earned_zats: r.json.balance.earned_zats, available_zats: r.json.balance.available_zats, withdrawable_zats: r.json.withdrawable_zats });
    break;
  }
  case "show": {
    const s = await show();
    out({ organization_id: s.organization_id, status: s.status, balance: s.balance, withdrawable_zats: s.withdrawable_zats, address_locked: s.address_locked, payouts: s.payouts.map((p) => ({ id: p.id, state: p.state, amount_zats: p.amount_zats, txid: p.txid, confirmations: p.confirmations, fee_zats: p.fee_zats })) });
    break;
  }
  case "pages": {
    const [address, ...txids] = args;
    const st = state();
    const w = await browser(st.worker).get(`/app/streams/${st.streamId}`);
    const e = await browser(st.boss).get("/app");
    const ed = await browser(st.boss).get(`/app/streams/${st.streamId}`);
    const checks = {
      worker_page_200: w.status === 200,
      worker_sees_paid: w.html.includes("Paid ✓"),
      worker_sees_locked_address: w.html.includes("Locked for the life of this stream") && w.html.includes(address),
      employer_home_200: e.status === 200,
      employer_sees_payments: txids.every((t) => e.html.includes(t.slice(0, 10))),
      employer_sees_payout_wallet: e.html.includes("Payout wallet"),
      employer_never_sees_address: !e.html.includes(address) && !ed.html.includes(address) && ed.html.includes("set and locked by the worker"),
      worker_txids_shown: txids.every((t) => w.html.includes(t.slice(0, 10))),
    };
    out(checks);
    if (!Object.values(checks).every(Boolean)) die("page checks failed");
    break;
  }
  default:
    die(`unknown command ${cmd}`);
}

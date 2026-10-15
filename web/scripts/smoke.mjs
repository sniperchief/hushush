// End-to-end smoke test against a RUNNING app (`next start`) + payroll-api.
// Drives the Phase 5 flow over real HTTP with real cookies — employer and
// worker in SEPARATE cookie jars, as two people in two browsers — and checks
// the server-rendered pages. Passkeys come from a SOFTWARE authenticator
// (scripts/lib/soft-passkey.mjs); everything else is the real app.
//
// Usage: APP_URL=http://localhost:3000 node scripts/smoke.mjs
// Payouts: if payroll-api has no wallet configured, the address step must
// report "Payouts are not enabled"; with one, it locks a mock-valid address.
import { Browser } from "./lib/browser.mjs";

const APP = process.env.APP_URL ?? "http://localhost:3000";
const ADDRESS = process.env.SMOKE_ADDRESS; // optional: a valid testnet UA

let step = 0;
const log = (m) => console.log(`${String(++step).padStart(2)}. ${m}`);
const fail = (m) => {
  console.error(`FAIL: ${m}`);
  process.exit(1);
};
const expect = (cond, m) => (cond ? log(m) : fail(m));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const now = () => Math.floor(Date.now() / 1000);

// ---- employer -----------------------------------------------------------------
const boss = new Browser(APP);
let r = await boss.get("/");
expect(r.status === 200 && r.html.includes("Payroll, kept private.") && r.html.includes('href="/signup"'), `landing page offers sign-up (HTTP ${r.status})`);
const bossKey = await boss.signUp("employer", "Smoke Boss");
log("employer signed up with a passkey (no password, no e-mail)");
r = await boss.get("/app");
expect(r.status === 200 && r.html.includes("Name your company"), "employer is asked for the company name");
r = await boss.post("/api/company", { name: "Smoke Testnet Co" });
expect(r.status === 201, "company created");
r = await boss.get("/app");
expect(r.html.includes("Smoke Testnet Co") && r.html.includes("Payout wallet") && r.html.includes("Get test TAZ"), "employer home shows the company and its own payout wallet with the faucet");
r = await boss.post("/api/streams", { workerName: "Wanda", totalZec: "0.5", startAt: null, endAt: now() + 120 });
expect(r.status === 201 && /\/join\/[A-Za-z0-9_-]{43}$/.test(r.json.link), `stream created; worker link ${r.json?.link}`);
const link = new URL(r.json.link);
const offerId = r.json.offer.id;
r = await boss.get("/app");
expect(r.html.includes("Waiting for the worker to join") && r.html.includes("Wanda"), "employer home lists the stream as waiting for the worker");

// The employer can't accept their own company's worker link (payroll refuses).
r = await boss.post(`/api/join${link.pathname.slice("/join".length)}`);
expect(r.status === 403, "employer can't accept their own worker link");

// ---- worker (another browser) -------------------------------------------------
const wanda = new Browser(APP);
r = await wanda.get(link.pathname);
expect(
  r.status === 200 && r.html.includes("Smoke Testnet Co") && r.html.includes("0.5 ZEC") && r.html.includes("Sign up with a passkey"),
  "worker link previews the terms and offers passkey sign-up",
);
const wandaKey = await wanda.signUp("worker", "Wanda");
r = await wanda.post(`/api/join${link.pathname.slice("/join".length)}`);
expect(r.status === 201 && r.json.streamId, "worker joined through the link");
const streamId = r.json.streamId;
r = await wanda.get("/app");
expect(r.html.includes("What you") && r.html.includes("Smoke Testnet Co") && !r.html.includes("Treasury"), "worker home shows their stream, and no employer UI");
r = await wanda.get(`/app/streams/${streamId}`);
expect(r.html.includes("Payout address") && r.html.includes("Set payout address"), "worker stream page asks for the payout address");
r = await wanda.get("/app/streams/new");
expect(r.status === 307 && new URL(r.location, APP).pathname === "/app", "worker cannot open the employer's new-stream page (redirected home)");

// The link is single-use.
const eve = new Browser(APP);
await eve.signUp("worker", "Eve");
r = await eve.post(`/api/join${link.pathname.slice("/join".length)}`);
expect(r.status === 409, "a second worker cannot use the same link");
r = await eve.get(link.pathname);
expect(r.html.includes("already been used"), "the used link says so to anyone else");
r = await eve.get(`/app/streams/${streamId}`);
expect(r.status === 404, "another worker cannot see the stream");

// ---- address ------------------------------------------------------------------
r = await wanda.post(`/api/streams/${streamId}/address`, { address: ADDRESS ?? "utest1smokeaddressaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" });
if (r.status === 503) {
  expect(r.json?.error === "payouts_disabled", "payouts disabled on this server: setting an address is refused cleanly");
} else {
  expect(r.status === 200 && r.json.address_locked, "payout address set and locked");
  r = await wanda.post(`/api/streams/${streamId}/address`, { address: "utest1someotheraddressbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" });
  expect(r.status === 409, "a different address is refused: the lock holds");
  r = await boss.get(`/app/streams/${streamId}`);
  expect(r.html.includes("set and locked by the worker") && !r.html.includes("utest1smoke"), "employer sees that it is locked, never the address");
}

// ---- employer view and ending -----------------------------------------------
r = await boss.get("/app");
expect(r.html.includes("Wanda") && !r.html.includes("Waiting for the worker to join"), "employer home now lists Wanda's stream as joined");
await sleep(1100);
r = await boss.post(`/api/streams/${streamId}/end`);
expect(r.status === 200 && r.json.status === "ended" && r.json.balance.earned_zats > 0, `employer ended the stream; earned ${r.json?.balance?.earned_zec} ZEC stays the worker's`);
r = await wanda.get(`/app/streams/${streamId}`);
expect(r.html.includes("Ended"), "worker sees the stream as ended");

// Cancel an unjoined stream.
r = await boss.post("/api/streams", { workerName: "Nobody", totalZec: "1", startAt: null, endAt: now() + 3600 });
const unused = new URL(r.json.link);
r = await boss.post(`/api/offers/${r.json.offer.id}/cancel`);
expect(r.status === 200 && r.json.status === "cancelled", "an unjoined stream can be cancelled");
r = await eve.get(unused.pathname);
expect(r.html.includes("cancelled"), "its link says it was cancelled");
r = await boss.post(`/api/offers/${offerId}/cancel`);
expect(r.status === 409, "a joined stream cannot be cancelled (only ended)");

// ---- sessions -------------------------------------------------------------------
const again = new Browser(APP);
expect(await again.signIn(wandaKey), "worker signs in again on another browser with the same passkey");
expect((await again.get("/app")).html.includes("Smoke Testnet Co"), "and sees the stream");
const bossAgain = new Browser(APP);
expect(await bossAgain.signIn(bossKey), "employer signs in again with their passkey");
r = await again.post("/api/auth/logout");
expect(r.status === 200, "logout");
r = await again.get("/app");
expect(r.status === 307 && r.location?.includes("/login"), "signed-out /app redirects to sign-in");
r = await fetch(`${APP}/api/company`, { method: "POST", headers: { origin: "https://evil.example", "content-type": "application/json", cookie: boss.cookie }, body: "{}" });
expect(r.status === 403, "cross-site requests are refused");

console.log(`\nSMOKE PASS: ${step} steps`);

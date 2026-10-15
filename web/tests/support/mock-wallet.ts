// *** MOCK payout wallet — NOT a real wallet. ***
//
// An in-memory HTTP server implementing the payout wallet service's API
// contract (payout-wallet/src/http.rs) for the web integration tests: the
// real payroll-api binary talks to it exactly as it talks to the real wallet.
// There are no keys, no proofs, no transactions and no network. It simulates
// a chain (tip height, mining) and counts every "transaction" it builds, so
// tests can assert nothing is ever paid twice. The real wallet is exercised
// by the live testnet run (docs/PHASE3_PAYOUTS.md).
//
// Phase 6: per-company accounts (`/v1/accounts`), each with its own balance
// and funding address, quotes/payments from a named `account` (the treasury
// when omitted), and the testnet faucet (treasury → account) with the real
// wallet's limits. A new account starts with `newAccountZats` so flows that
// don't care about funding still work; tests that do set balances explicitly.
//
// Test controls live under /__mock/* (only on this mock).
import { createHash, randomBytes } from "node:crypto";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";

type Mode = "broadcast" | "fail" | "lost_response";

interface Rec {
  payment_id: string;
  account: string | null;
  address: string | null;
  amount_zats: number;
  max_fee_zats: number;
  state: "pending" | "failed" | "built" | "broadcast";
  failure_reason: string | null;
  txid: string | null;
  fee_zats: number | null;
  expiry_height: number | null;
}

export function fingerprint(paymentId: string, address: string): string {
  return createHash("sha256").update(`hushush-payout-recipient-v1\0${paymentId}\0${address.trim()}`).digest("hex");
}

const FEE = 10_000;

export function startMockWallet(token: string): Promise<{ url: string; server: Server }> {
  const records = new Map<string, Rec>();
  const mined = new Map<string, number>();
  const modes: Mode[] = [];
  let tip = 1_000;
  let spendable = 5_000_000_000; // the treasury
  let newAccountZats = 5_000_000_000;
  const accounts = new Map<string, { spendable: number; address: string }>();
  const faucet = { amount: 1_000_000, cooldown: 86_400, max: 3 };
  const grants = new Map<string, { key: string; at: number }>(); // payment id → grant
  let txsBuilt = 0;
  let submits = 0;

  const balanceOf = (account: string | null) => (account === null ? spendable : (accounts.get(account)?.spendable ?? 0));
  const debit = (account: string | null, z: number) => {
    if (account === null) spendable -= z;
    else accounts.get(account)!.spendable -= z;
  };
  const liveGrants = (key: string) =>
    [...grants.entries()].filter(([id, g]) => g.key === key && records.get(id)?.state !== "failed").map(([, g]) => g.at);
  const faucetView = (key: string) => {
    const g = liveGrants(key);
    const remaining = Math.max(0, faucet.max - g.length);
    const last = g[g.length - 1];
    const nextAt = last !== undefined && last + faucet.cooldown > Date.now() / 1000 ? last + faucet.cooldown : null;
    return { amount_zats: faucet.amount, available: faucet.amount > 0 && remaining > 0 && nextAt === null, next_at: remaining > 0 ? nextAt : null, remaining };
  };

  const validRecipient = (a: string) =>
    a.startsWith("utest1") && !a.includes("sapling") ? null : "Sapling-only or non-shielded address (mock rule)";

  function view(r: Rec) {
    const v: Record<string, unknown> = {
      ...r,
      recipient_fingerprint: r.address ? fingerprint(r.payment_id, r.address) : "",
      chain: null,
      verification: null,
    };
    delete v.address;
    if (r.txid && r.address) {
      const h = mined.get(r.txid) ?? null;
      v.chain = {
        tip_height: tip,
        mined_height: h,
        confirmations: h === null ? 0 : tip - h + 1,
        in_mempool: h === null,
        wallet_mined_height: h,
        wallet_expired_unmined: h === null && r.expiry_height !== null && tip > r.expiry_height,
      };
      v.verification = {
        source: h === null ? "local" : "chain",
        fully_shielded: true,
        single_pool: true,
        fee_zats: r.fee_zats,
        outputs: [{ recipient_fingerprint: fingerprint(r.payment_id, r.address), amount_zats: r.amount_zats, memo_matches: true, pool: "ironwood" }],
      };
    }
    return v;
  }

  async function body(req: IncomingMessage): Promise<Record<string, unknown>> {
    let s = "";
    for await (const c of req) s += c;
    return s ? JSON.parse(s) : {};
  }

  function send(res: ServerResponse, status: number, data: unknown) {
    res.writeHead(status, { "content-type": "application/json" });
    res.end(JSON.stringify(data));
  }

  const server = createServer(async (req, res) => {
    const url = new URL(req.url ?? "/", "http://mock");
    const p = url.pathname;
    try {
      if (p.startsWith("/__mock/")) {
        const b = req.method === "POST" ? await body(req) : {};
        if (p === "/__mock/next") modes.push(b.mode as Mode);
        if (p === "/__mock/spendable") {
          // Every balance (treasury, existing and future accounts).
          spendable = Number(b.zats);
          newAccountZats = Number(b.zats);
          for (const a of accounts.values()) a.spendable = Number(b.zats);
        }
        if (p === "/__mock/account-spendable") accounts.get(String(b.key))!.spendable = Number(b.zats);
        if (p === "/__mock/new-account-zats") newAccountZats = Number(b.zats);
        if (p === "/__mock/mine") for (const r of records.values()) if (r.state === "broadcast" && r.txid && !mined.has(r.txid)) mined.set(r.txid, tip);
        if (p === "/__mock/advance") tip += Number(b.blocks ?? 1);
        return send(res, 200, { txs_built: txsBuilt, submits, tip, treasury: spendable, accounts: Object.fromEntries([...accounts].map(([k, a]) => [k, a.spendable])) });
      }
      if (req.headers.authorization !== `Bearer ${token}`) return send(res, 401, { error: "unauthorized", message: "invalid token" });

      if (p === "/v1/status") {
        return send(res, 200, { network: "testnet", spendable_zats: spendable, pending_zats: 0, total_zats: spendable, funding_address: "utest1mockfundingaddress", chain_tip: tip, last_sync_at: Math.floor(Date.now() / 1000) });
      }
      if (p === "/v1/accounts" && req.method === "POST") {
        const key = String((await body(req)).key);
        const created = !accounts.has(key);
        if (created) accounts.set(key, { spendable: newAccountZats, address: `utest1mockaccount${key.replace(/[^a-z0-9]/gi, "")}` });
        return send(res, 200, { key, funding_address: accounts.get(key)!.address, created });
      }
      const acct = p.match(/^\/v1\/accounts\/([A-Za-z0-9_-]{1,64})(\/faucet)?$/);
      if (acct) {
        const key = acct[1]!;
        const a = accounts.get(key);
        if (!a) return send(res, 409, { error: "unknown_account", message: `no payout account is registered for "${key}"` });
        if (!acct[2]) {
          return send(res, 200, { network: "testnet", key, funding_address: a.address, spendable_zats: a.spendable, pending_zats: 0, total_zats: a.spendable, chain_tip: tip, last_sync_at: Math.floor(Date.now() / 1000), faucet: faucetView(key) });
        }
        const id = String((await body(req)).payment_id);
        const existing = records.get(id);
        if (existing) {
          return grants.get(id)?.key === key ? send(res, 200, view(existing)) : send(res, 409, { error: "idempotency_conflict", message: "payment id was already used for something else" });
        }
        const v = faucetView(key);
        if (v.remaining === 0) return send(res, 409, { error: "refused", message: "This company has used all its test TAZ top-ups." });
        if (v.next_at !== null) return send(res, 409, { error: "refused", message: `The next test TAZ top-up is available in ${Math.ceil((v.next_at - Date.now() / 1000) / 60)} minutes.` });
        grants.set(id, { key, at: Math.floor(Date.now() / 1000) });
        const r: Rec = { payment_id: id, account: null, address: a.address, amount_zats: faucet.amount, max_fee_zats: FEE * 10, state: "failed", failure_reason: null, txid: null, fee_zats: null, expiry_height: null };
        if (spendable < faucet.amount + FEE) {
          r.failure_reason = "insufficient spendable balance (mock)";
        } else {
          txsBuilt++;
          spendable -= faucet.amount + FEE;
          a.spendable += faucet.amount; // the mock credits at once (the real one after confirmations)
          Object.assign(r, { state: "broadcast", txid: randomBytes(32).toString("hex"), fee_zats: FEE, expiry_height: tip + 40 });
        }
        records.set(id, r);
        return send(res, 200, view(r));
      }
      if (p === "/v1/quotes" && req.method === "POST") {
        const b = await body(req);
        const account = (b.account as string | null | undefined) ?? null;
        if (account !== null && !accounts.has(account)) return send(res, 409, { error: "unknown_account", message: "unknown account" });
        const why = validRecipient(String(b.address));
        const amount = Number(b.amount_zats);
        const ok = !why && balanceOf(account) >= amount + FEE;
        return send(res, 200, {
          valid_recipient: !why,
          amount_zats: amount,
          fee_zats: why ? null : FEE,
          spendable_zats: balanceOf(account),
          pending_zats: 0,
          sufficient: ok,
          pool: ok ? "ironwood" : null,
          reason: why ?? (ok ? null : "Insufficient balance (mock)"),
        });
      }
      if (p === "/v1/payments" && req.method === "POST") {
        submits++;
        const b = await body(req);
        const id = String(b.payment_id);
        const address = String(b.address).trim();
        const account = (b.account as string | null | undefined) ?? null;
        const existing = records.get(id);
        if (existing) {
          if (existing.address !== null && (existing.address !== address || existing.amount_zats !== Number(b.amount_zats) || existing.account !== account)) {
            return send(res, 409, { error: "idempotency_conflict", message: "payment id reused" });
          }
          return send(res, 200, view(existing));
        }
        if (account !== null && !accounts.has(account)) return send(res, 409, { error: "unknown_account", message: "unknown account" });
        const mode = modes.shift() ?? "broadcast";
        const r: Rec = { payment_id: id, account, address, amount_zats: Number(b.amount_zats), max_fee_zats: Number(b.max_fee_zats), state: "failed", failure_reason: null, txid: null, fee_zats: null, expiry_height: null };
        const why = validRecipient(address);
        if (why || mode === "fail" || balanceOf(account) < r.amount_zats + FEE) {
          r.failure_reason = why ?? (mode === "fail" ? "could not propose payment: Insufficient balance (mock)" : "insufficient spendable balance (mock)");
        } else {
          txsBuilt++;
          debit(account, r.amount_zats + FEE);
          Object.assign(r, { state: "broadcast", txid: randomBytes(32).toString("hex"), fee_zats: FEE, expiry_height: tip + 40 });
        }
        records.set(id, r);
        if (mode === "lost_response") {
          req.socket.destroy(); // built and broadcast, but the caller never hears back
          return;
        }
        return send(res, 200, view(r));
      }
      const m = p.match(/^\/v1\/payments\/([0-9a-f-]{36})(\/cancel)?$/);
      if (m) {
        const id = m[1]!;
        if (m[2] && req.method === "POST") {
          if (!records.has(id)) {
            records.set(id, { payment_id: id, account: null, address: null, amount_zats: 0, max_fee_zats: 0, state: "failed", failure_reason: "cancelled before any transaction was built", txid: null, fee_zats: null, expiry_height: null });
          }
          return send(res, 200, view(records.get(id)!));
        }
        const r = records.get(id);
        return r ? send(res, 200, view(r)) : send(res, 404, { error: "not_found", message: "unknown payment id" });
      }
      return send(res, 404, { error: "not_found", message: "no route" });
    } catch (e) {
      return send(res, 500, { error: "internal", message: String(e) });
    }
  });

  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const a = server.address();
      const port = a && typeof a === "object" ? a.port : 0;
      resolve({ url: `http://127.0.0.1:${port}`, server });
    });
  });
}

// Phase 6: every company pays from its own payout wallet account, funded by
// the employer (or, on testnet, by the faucet from the shared treasury).
// Real route handlers, real payroll-api and PostgreSQL, against the MOCK
// payout wallet (tests/support/mock-wallet.ts): no keys, chain or funds here.
import { randomUUID } from "node:crypto";
import { beforeAll, describe, expect, it } from "vitest";
import * as faucetRoute from "@/app/api/company/faucet/route";
import { payroll } from "@/lib/payroll";
import { boundStream, call, flow, mock, newEmployer, newWorker, setAddress, waitFor, withdraw, type TestUser } from "./support/helpers";

const ADDR = "utest1mockworkeraddressaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FEE = 10_000; // the mock's fee
const FAUCET = 1_000_000; // the faucet amount (0.01 TAZ), as in the real wallet's default

let boss: TestUser & { companyId: string };
let rival: TestUser & { companyId: string };
let worker: TestUser;

beforeAll(async () => {
  [boss, rival, worker] = await Promise.all([newEmployer("Bea", "Bea Studio"), newEmployer("Ray", "Ray Works"), newWorker("Walt")]);
});

const stats = () => mock("stats");
const topUp = (u: TestUser, key: string = randomUUID()) => call(faucetRoute.POST, "POST", {}, { cookie: u.cookie, body: { idempotencyKey: key } });

async function readyStream(employer: TestUser): Promise<string> {
  const id = await boundStream(employer, worker, "0.001", 10);
  expect((await setAddress(worker, id, ADDR)).status).toBe(200);
  await waitFor(() => flow(worker, id), (s) => s.withdrawable_zats !== null);
  return id;
}

describe("company payout wallets", () => {
  it("each company gets its own wallet and funding address, not the treasury's", async () => {
    const a = await payroll.payoutWallet(boss.id, boss.companyId);
    const b = await payroll.payoutWallet(rival.id, rival.companyId);
    expect(a.funding_address).toMatch(/^utest1mockaccount/);
    expect(b.funding_address).toMatch(/^utest1mockaccount/);
    expect(a.funding_address).not.toBe(b.funding_address);
    expect(a.funding_address).not.toBe("utest1mockfundingaddress");
    expect(a.faucet).toMatchObject({ amount_zats: FAUCET, amount_zec: "0.01000000", remaining: 3 });
    // Only the company's managers can see it (outsiders don't even learn it exists).
    for (const u of [rival, worker]) {
      const e = await payroll.payoutWallet(u.id, boss.companyId).catch((x: { status: number }) => x);
      expect([403, 404]).toContain((e as { status: number }).status);
    }
  });

  it("a withdrawal is paid from the company's own account; the treasury is untouched", async () => {
    const id = await readyStream(boss);
    const before = await stats();
    const r = await withdraw(worker, id);
    expect(r.status, JSON.stringify(r.body)).toBe(201);
    const amount = (r.body.payout as { amount_zats: number }).amount_zats;
    const after = await stats();
    expect(after.treasury).toBe(before.treasury);
    expect(after.accounts[boss.companyId]).toBe(before.accounts[boss.companyId]! - amount - FEE);
    expect(after.accounts[rival.companyId]).toBe(before.accounts[rival.companyId]);
  });

  it("an unfunded company wallet refuses the withdrawal; nothing is reserved or built", async () => {
    const id = await readyStream(rival);
    await mock("account-spendable", { key: rival.companyId, zats: 0 });
    const before = await stats();
    const r = await withdraw(worker, id);
    expect(r.status).toBe(409);
    expect(r.body.error).toBe("insufficient_wallet_balance");
    expect(String(r.body.message)).toMatch(/company's payout wallet/);
    expect((await flow(worker, id)).balance.reserved_zats).toBe(0);
    const after = await stats();
    expect(after.txs_built).toBe(before.txs_built);
    expect(after.treasury).toBe(before.treasury);
  });

  it("Get test TAZ tops the company up from the treasury, once per key, within the limits", async () => {
    await mock("account-spendable", { key: rival.companyId, zats: 0 });
    const before = await stats();
    const key = randomUUID();
    const r = await topUp(rival, key);
    expect(r.status, JSON.stringify(r.body)).toBe(201);
    expect(r.body).toMatchObject({ state: "broadcast", amount_zats: FAUCET, amount_zec: "0.01000000" });
    let s = await stats();
    expect(s.accounts[rival.companyId]).toBe(FAUCET);
    expect(s.treasury).toBe(before.treasury - FAUCET - FEE);

    // A retry with the same key returns the same top-up; nothing new is sent.
    const again = await topUp(rival, key);
    expect(again.body.payment_id).toBe(r.body.payment_id);
    expect((await stats()).txs_built).toBe(s.txs_built);

    // A new top-up within the cooldown is refused with a clear message.
    const soon = await topUp(rival);
    expect(soon.status).toBe(409);
    expect(soon.body.error).toBe("faucet_refused");
    expect(String(soon.body.message)).toMatch(/next test TAZ top-up/i);
    const w = await payroll.payoutWallet(rival.id, rival.companyId);
    expect(w.faucet).toMatchObject({ available: false, remaining: 2 });
    expect(w.faucet.next_at).toBeGreaterThan(Math.floor(Date.now() / 1000));

    // Now funded: the earlier refused withdrawal goes through, from the company account.
    const id = await readyStream(rival);
    const paid = await withdraw(worker, id);
    expect(paid.status, JSON.stringify(paid.body)).toBe(201);
    s = await stats();
    expect(s.accounts[rival.companyId]).toBeLessThan(FAUCET);
  });

  it("a repeat tap while the withdrawal's change is still confirming gets that withdrawal back, not an error", async () => {
    // Live run p6run1: a fresh company account holds one note; the first
    // withdrawal spends it and the change is unconfirmed, so spendable is 0
    // for a few blocks. Concurrent taps must still see the duplicate.
    const id = await readyStream(boss);
    const first = await withdraw(worker, id);
    expect(first.status).toBe(201);
    await mock("account-spendable", { key: boss.companyId, zats: 0 });
    try {
      const built = (await stats()).txs_built;
      const again = await withdraw(worker, id); // a new idempotency key: another tab
      expect(again.status, JSON.stringify(again.body)).toBe(200);
      expect(again.body.duplicate).toBe(true);
      expect((again.body.payout as { id: string }).id).toBe((first.body.payout as { id: string }).id);
      expect((await stats()).txs_built).toBe(built);
    } finally {
      await mock("account-spendable", { key: boss.companyId, zats: 5_000_000_000 });
    }
  });

  it("only a company's own manager can request test TAZ", async () => {
    expect((await call(faucetRoute.POST, "POST", {}, { body: { idempotencyKey: randomUUID() } })).status).toBe(401);
    const noCompany = await topUp(worker);
    expect(noCompany.status).toBe(409);
    expect(noCompany.body.error).toBe("no_company");
    expect((await call(faucetRoute.POST, "POST", {}, { cookie: boss.cookie, body: { idempotencyKey: "not-a-uuid" } })).status).toBe(422);
  });
});

// Integration test harness: a real PostgreSQL database and the real Rust
// payroll-api binary. Nothing payroll-related is mocked. The payout WALLET is
// a MOCK (tests/support/mock-wallet.ts): an in-memory implementation of the
// wallet service's API contract, so no keys, chain or funds are involved.
import { spawn, type ChildProcess } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync } from "node:fs";
import { createServer } from "node:net";
import path from "node:path";
import pg from "pg";
import type { TestProject } from "vitest/node";
import { migrate } from "../../scripts/migrate.ts";
import { pgConfig } from "../../src/lib/pg-config.ts";
import { startMockWallet } from "./mock-wallet.ts";

declare module "vitest" {
  export interface ProvidedContext {
    testEnv: Record<string, string>;
  }
}

let child: ChildProcess | undefined;
let wallet: Awaited<ReturnType<typeof startMockWallet>> | undefined;

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.listen(0, "127.0.0.1", () => {
      const a = s.address();
      s.close(() => (a && typeof a === "object" ? resolve(a.port) : reject(new Error("no port"))));
    });
  });
}

async function waitHealthy(url: string, ms = 60_000): Promise<void> {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    try {
      if ((await fetch(`${url}/healthz`)).ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error("payroll-api did not become healthy");
}

export default async function setup(project: TestProject) {
  const dbUrl = process.env.TEST_DATABASE_URL;
  if (!dbUrl) {
    throw new Error("TEST_DATABASE_URL must point at a disposable PostgreSQL database (it is wiped).");
  }
  if (!/test/i.test(new URL(dbUrl).pathname)) {
    throw new Error("Refusing to wipe a database whose name does not contain 'test'.");
  }

  // Fresh schema every run.
  const c = new pg.Client(pgConfig(dbUrl));
  await c.connect();
  await c.query("DROP SCHEMA public CASCADE; CREATE SCHEMA public;");
  await c.end();

  const exe = process.platform === "win32" ? "hushush-payroll-api.exe" : "hushush-payroll-api";
  const bin = process.env.PAYROLL_API_BIN ?? path.resolve(import.meta.dirname, "../../../payroll-api/target/debug", exe);
  if (!existsSync(bin)) throw new Error(`payroll-api binary not found at ${bin}; run \`cargo build\` in payroll-api/`);

  const port = await freePort();
  const token = randomBytes(32).toString("hex");
  const walletToken = randomBytes(32).toString("hex");
  wallet = await startMockWallet(walletToken);
  const apiUrl = `http://127.0.0.1:${port}`;
  child = spawn(bin, [], {
    env: {
      ...process.env,
      PAYROLL_DATABASE_URL: dbUrl,
      PAYROLL_API_TOKEN: token,
      PAYROLL_API_ADDR: `127.0.0.1:${port}`,
      RUST_LOG: "warn",
      // Payouts against the MOCK wallet; fast worker, short confirmation policy.
      WALLET_URL: wallet.url,
      WALLET_API_TOKEN: walletToken,
      PAYOUT_ENABLED_ORGS: "*",
      PAYOUT_ADDRESS_KEY: randomBytes(32).toString("hex"),
      PAYOUT_POLL_SECS: "1",
      PAYOUT_CONFIRMATIONS: "3",
      WALLET_TIMEOUT_SECS: "5",
      WALLET_SUBMIT_TIMEOUT_SECS: "10",
    },
    stdio: ["ignore", "inherit", "inherit"],
  });
  await waitHealthy(apiUrl); // payroll-api migrates the payroll schema on start
  await migrate(dbUrl, () => {});

  project.provide("testEnv", {
    DATABASE_URL: dbUrl,
    PAYROLL_API_URL: apiUrl,
    PAYROLL_API_TOKEN: token,
    APP_URL: "http://localhost:3000",
    MOCK_WALLET_URL: wallet.url,
  });

  return async () => {
    child?.kill();
    wallet?.server.close();
  };
}

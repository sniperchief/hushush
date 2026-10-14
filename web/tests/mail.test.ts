// Phase 4 regression: the dev mail outbox (which stores live verification and
// invite links) can't be enabled on a public production deployment.
import { afterEach, describe, expect, it } from "vitest";
import { sendMail } from "@/lib/mail";

const saved = { ...process.env };
afterEach(() => {
  for (const k of ["NODE_ENV", "APP_URL", "SMTP_URL", "HUSHUSH_ALLOW_DEV_OUTBOX"]) {
    if (saved[k] === undefined) delete process.env[k];
    else process.env[k] = saved[k];
  }
});

const mail = { to: "x@example.test", subject: "s", text: "link http://localhost/verify?token=t" };

function production(appUrl: string, allow?: string) {
  Object.assign(process.env, { NODE_ENV: "production", APP_URL: appUrl });
  delete process.env.SMTP_URL;
  if (allow) process.env.HUSHUSH_ALLOW_DEV_OUTBOX = allow;
  else delete process.env.HUSHUSH_ALLOW_DEV_OUTBOX;
}

describe("dev mail outbox in production", () => {
  it("is refused without SMTP", async () => {
    production("https://pay.example.com");
    await expect(sendMail(mail)).rejects.toThrow("SMTP_URL must be configured");
  });
  it("is refused for a public APP_URL even with the override", async () => {
    production("https://pay.example.com", "1");
    await expect(sendMail(mail)).rejects.toThrow("SMTP_URL must be configured");
  });
  it("is allowed only for a localhost APP_URL with the override", async () => {
    production("http://localhost:3100", "1");
    await expect(sendMail(mail)).resolves.toBeUndefined();
  });
});

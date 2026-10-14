import "server-only";
import nodemailer from "nodemailer";
import { db } from "./db";
import { env } from "./env";

export interface Mail {
  to: string;
  subject: string;
  text: string;
}

/**
 * Sends mail via SMTP_URL. Without SMTP in development/test, mail is stored in
 * the `dev_outbox` table and logged (so links can be followed locally).
 * Production without SMTP is a configuration error.
 */
function isLocalAppUrl(): boolean {
  const host = new URL(env.appUrl).hostname;
  return host === "localhost" || host === "127.0.0.1" || host === "[::1]";
}

export async function sendMail(mail: Mail): Promise<void> {
  if (env.smtpUrl) {
    const transport = nodemailer.createTransport(env.smtpUrl);
    await transport.sendMail({ from: env.mailFrom, to: mail.to, subject: mail.subject, text: mail.text });
    return;
  }
  // The outbox stores (and in development logs) live verification and invite
  // links. In production it is only allowed for a local smoke/live test: the
  // explicit override AND an APP_URL on localhost.
  if (env.isProduction && !(process.env.HUSHUSH_ALLOW_DEV_OUTBOX === "1" && isLocalAppUrl())) {
    throw new Error("SMTP_URL must be configured in production");
  }
  await db().query("INSERT INTO dev_outbox (to_address, subject, body) VALUES ($1, $2, $3)", [
    mail.to,
    mail.subject,
    mail.text,
  ]);
  if (process.env.VITEST !== "true" && !env.isProduction) {
    console.info(`[dev mail] to=${mail.to} subject=${JSON.stringify(mail.subject)}\n${mail.text}`);
  }
}

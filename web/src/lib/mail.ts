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
export async function sendMail(mail: Mail): Promise<void> {
  if (env.smtpUrl) {
    const transport = nodemailer.createTransport(env.smtpUrl);
    await transport.sendMail({ from: env.mailFrom, to: mail.to, subject: mail.subject, text: mail.text });
    return;
  }
  if (env.isProduction && process.env.HUSHUSH_ALLOW_DEV_OUTBOX !== "1") {
    throw new Error("SMTP_URL must be configured in production");
  }
  await db().query("INSERT INTO dev_outbox (to_address, subject, body) VALUES ($1, $2, $3)", [
    mail.to,
    mail.subject,
    mail.text,
  ]);
  if (process.env.VITEST !== "true") {
    console.info(`[dev mail] to=${mail.to} subject=${JSON.stringify(mail.subject)}\n${mail.text}`);
  }
}

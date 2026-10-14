import "server-only";

function required(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set (see .env.example)`);
  return v;
}

/** Server configuration. Read lazily so `next build` works without secrets. */
export const env = {
  get databaseUrl() {
    return required("DATABASE_URL");
  },
  get payrollApiUrl() {
    return process.env.PAYROLL_API_URL ?? "http://127.0.0.1:7878";
  },
  get payrollApiToken() {
    return required("PAYROLL_API_TOKEN");
  },
  get appUrl() {
    return required("APP_URL").replace(/\/+$/, "");
  },
  /** Public origin (scheme://host[:port]) — the WebAuthn expected origin. */
  get appOrigin() {
    return new URL(this.appUrl).origin;
  },
  /** WebAuthn Relying Party ID: the app's host name unless overridden. */
  get rpId() {
    return process.env.WEBAUTHN_RP_ID || new URL(this.appUrl).hostname;
  },
  get smtpUrl() {
    return process.env.SMTP_URL || undefined;
  },
  get mailFrom() {
    return process.env.MAIL_FROM ?? "hushush <no-reply@localhost>";
  },
  get isProduction() {
    return process.env.NODE_ENV === "production";
  },
};

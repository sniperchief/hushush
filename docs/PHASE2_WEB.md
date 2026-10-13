# hushush Phase 2 — Employer and Contributor Experience

Date: 2026-10-13 · Code: [`web/`](../web/) (Next.js), [`payroll-api/`](../payroll-api/) (Rust service), [`payroll/`](../payroll/) (engine, extended)

Phase 2 adds the web app. Employers create organizations, invite people, and
run fixed-rate ZEC payroll streams. Contributors see their earnings and
request claims.

**Claims are still accounting reservations only.** No wallet is connected, no
ZEC moves, and the system has no way to mark a claim paid.

## 1. Architecture

```text
 Browser ──HTTPS──▶ Next.js (web/)  ──HTTP, localhost/private + bearer token──▶ payroll-api (Rust)
                     │  auth, sessions, invitations, UI                          │  payroll engine (payroll/)
                     │                                                           │  all payroll rules + accrual
                     └───────────────┐                          ┌────────────────┘
                                     ▼                          ▼
                                   PostgreSQL (one database)
                     web tables: auth_credentials, sessions,     payroll tables: users, organizations,
                     email_verification_tokens, invitations,     organization_members, payroll_streams,
                     login_attempts, dev_outbox                  stream_events, claims
```

| Concern | Lives in | Why |
|---|---|---|
| Accrual, balances, claim reservation, idempotency, stream lifecycle, org roles and every payroll authorization decision | **Rust engine** via `payroll-api` | Reuses Phase 1. The web app never calculates wages or money. ZEC strings are parsed (`"1.5"` → 150,000,000 zats) and formatted by the engine's `money` module. |
| Passwords, email verification, sessions, invitations, CSRF, UI | **Next.js server** | Web concerns. Secrets stay server-side; the browser only talks to Next.js. |
| Storage | **PostgreSQL** | The engine gained a PostgreSQL backend (`payroll/src/pg.rs`). The Phase 1 SQLite backend is kept and its tests are unchanged. |

### How the engine was extended without replacing its tested storage

- The business rules from Phase 1 now live once, in `payroll/src/engine.rs`, generic over a small storage trait (`payroll/src/store.rs`).
- Two backends implement that trait:
  - `sqlite.rs`, the Phase 1 storage layer.
  - `pg.rs`, new. Writes take `SELECT … FOR UPDATE` row locks on the stream (or claim), so concurrent requests are serialized per stream, just as SQLite's single writer serializes them.
- The PostgreSQL schema (`payroll/migrations_pg/0001_init.sql`) mirrors the SQLite one rule for rule: CHECK constraints, immutable stream terms, an append-only event log, valid claim transitions, the overdraw guard, and the recipient-must-be-member foreign key.
- The Phase 1 test file `payroll/tests/engine.rs` is **unchanged** and still passes against SQLite.
- The new `payroll/tests/backends.rs` runs the same scenarios against **both** backends.
- New engine operations for the UI:
  - listing organizations, members, organization streams and the user's own streams, each with server-computed balances;
  - changing a member's role (owner only; the last owner is protected);
  - removing a member (refused if they have payroll history).

### payroll-api

- Listens on `PAYROLL_API_ADDR` (default `127.0.0.1:7878`).
- Every `/v1` call requires `Authorization: Bearer $PAYROLL_API_TOKEN`, compared in constant time, plus `X-Actor-Id`, the user that Next.js authenticated.
- It must never be reachable from the internet. It runs migrations on start, guarded by an advisory lock.
- **There is no settle/payout endpoint.**

## 2. Features

**Employer (owner/admin):**

- Create an organization (the creator becomes owner).
- Invite people by email as contributor or admin. Only owners can invite admins.
- See pending invitations and revoke them.
- Change roles (owner only) and remove members. Members with streams are kept to preserve payroll history.
- Create streams: contributor, ZEC rate, a period (hour, day, week, 30 days or 365 days), a start time ("now" or a future time) and an optional end date.
- See each stream's earned, reserved, claimed and available balances, its status and its full history.
- Pause, resume and end streams. Ending asks for confirmation and explains that a rate change means ending the stream and creating a new one.

**Contributor (member):**

- A dashboard of their organizations and streams, with server-computed balances.
- Stream detail with balances and history.
- Request a partial claim, or use "Full amount", which fills in the available balance computed by the server.
- See claim records and their status, and cancel a pending claim.

**Claim labels:**

| Status | Shown as |
|---|---|
| `reserved` | **"Pending · unpaid"**, with the explanation that no ZEC has been sent |
| `released` | "Cancelled" |
| `settled` | "Paid · payout confirmed". Only a confirmed payout can produce this; it is unreachable in Phase 2 (tested). |

Balances are computed by the server at render time and labelled "as of HH:MM:SS UTC". Pages refresh every 30 s, and there is a Refresh button. No accrual is calculated in the browser.

## 3. Security model

| Control | Implementation |
|---|---|
| Passwords | Node's built-in `scrypt`: N=2^17, r=8, p=1, 64-byte key, 16-byte random salt, constant-time compare. Minimum 12 characters. |
| Email verification | Required before sign-in. The single-use token expires after 24 h and only its SHA-256 hash is stored. The link opens a page with a confirm button, so mail scanners that prefetch links can't consume it. |
| Account enumeration | Sign-up gives the same response for new and existing emails (existing holders get a notice email). Login gives the same error for an unknown email and a wrong password, and always runs scrypt. |
| Brute force | 10 failed sign-ins per email per 15 min, then 429. |
| Sessions | 256-bit random token in an `HttpOnly`, `SameSite=Lax` cookie (`Secure` and `__Host-` prefix over HTTPS). Only the hash is stored. 30-day absolute and 7-day idle expiry. Logout deletes the session server-side. |
| CSRF | Every state-changing route requires `Origin` to equal `APP_URL`'s origin and a JSON body. |
| Authorization | Enforced on the server for every operation. Org roles are decided by the engine from the authenticated actor. Outsiders get 404, so they can't learn whether something exists. |
| Invitations | Must be accepted by the invited (verified) email address. Membership is granted by the engine acting as the inviter, so the inviter's current authority is re-checked when the invitation is accepted. |
| Headers | `X-Frame-Options: DENY`, `frame-ancestors 'none'`, `nosniff`, `no-referrer`, HSTS, Permissions-Policy. |
| Money | Integer zatoshis end to end. BIGINT columns are returned to Node as strings. Amounts in JSON stay below 2^53. |

## 4. Setup

Prerequisites:

- PostgreSQL ≥ 15.
- Rust ≥ 1.88; on Windows, MSVC Build Tools.
- Node.js ≥ 20; tested on 24.18.

```sh
# 1. Database
createdb hushush

# 2. Payroll service (migrates the payroll schema on start)
cd payroll-api
cargo build --release
PAYROLL_DATABASE_URL=postgres://user:pass@host:5432/hushush \
PAYROLL_API_TOKEN=$(openssl rand -hex 32) \
PAYROLL_API_ADDR=127.0.0.1:7878 \
./target/release/hushush-payroll-api

# 3. Web app (migrate after payroll-api has started once)
cd web
npm ci
cp .env.example .env.local      # fill in DATABASE_URL, PAYROLL_API_TOKEN (same value), APP_URL, SMTP_URL
npm run db:migrate
npm run build && npm start      # or `npm run dev`
```

- Without `SMTP_URL` in development, emails (verification and invite links) are written to the `dev_outbox` table and logged to the console.
- In production, missing SMTP is an error. `HUSHUSH_ALLOW_DEV_OUTBOX=1` overrides this; use it only for local smoke tests.
- Migration order: payroll first (`payroll_schema_migrations`), then web (`web_schema_migrations`). The web migrator refuses to run if the payroll tables are missing.
- TLS to PostgreSQL: add `sslmode=require` to the URL. The Rust service then uses rustls with the Mozilla roots, and node-postgres honours it as well.

## 5. Tests and checks (run 2026-10-13)

| Command | Result |
|---|---|
| `payroll`: `cargo test -- --include-ignored` (with `PAYROLL_TEST_DATABASE_URL`) | **58 passed**: 12 unit + 29 Phase 1 SQLite (unchanged file) + 17 backend scenarios on SQLite **and** PostgreSQL |
| `payroll`: concurrency scenarios on both backends, repeated 15× | 15/15 |
| `payroll`, `payroll-api`: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` | OK |
| `web`: `npm test` (with `TEST_DATABASE_URL`) | **42 passed** (auth 12, claims 12, streams 8, permissions 10) |
| `web`: `npm run typecheck` (TypeScript 6.0.3, strict) | OK |
| `web`: `npm run lint` (eslint-config-next 16.4.0) | OK, 0 problems |
| `web`: `npm run build` (Next.js 16.4.0) | OK |
| `web`: `node scripts/smoke.mjs` against `next start` + payroll-api + PostgreSQL | **31/31 steps passed** |
| Phase 0 `wallet-proof`: `cargo test` (code unchanged) | 24 passed; 3 live testnet tests are opt-in and were not re-run |

The web tests are integration tests. A global setup:

1. wipes a database whose name must contain "test";
2. starts the real `payroll-api` binary on a random port with a random token;
3. runs the web migrations.

Then each test calls the actual Next.js route handlers. Nothing payroll-related is mocked.

Coverage:

- **Authentication:** hashing, verification required and single-use, enumeration resistance, password policy, throttling, logout, absolute and idle expiry, forged cookies, CSRF origin and content type.
- **Permissions:** contributor vs admin vs owner, recipient-only claims, last-owner protection, invitation email binding, revoked or duplicate invitations, inviter re-check.
- **Stream management:** exact zatoshi rates, rejection of backdated, malformed, bad-period and bad-end-date input, future start, end-date cap, pause/resume/end preserving earnings, history, rate change as end plus new stream.
- **Claims:** partial and full claims, over-available, malformed amounts, release, claims on paused and ended streams.
- **Duplicate requests:** the same key returns the original; the same key with a different amount is rejected; 12 concurrent retries produce exactly 1 claim.
- **Concurrent claims:** 10 parallel requests, only 2 fit, never over-reserved.
- **Cross-organization access:** another org's owner gets 404 on every read and write. The suite also confirms that you can't pay non-members, that there is no settle endpoint, and that payroll-api rejects calls without the service token.

Running the tests locally:

```sh
# payroll engine (PostgreSQL scenarios need a reachable server)
cd payroll && PAYROLL_TEST_DATABASE_URL=postgres://…/hushush_test cargo test -- --include-ignored
# web (build payroll-api first; the DB is WIPED)
cd payroll-api && cargo build
cd ../web && TEST_DATABASE_URL=postgres://…/hushush_test npm test
```

## 6. Known limitations

- **No password reset or email-change flow yet.** These weren't in scope but are needed before real users.
- **Rate limiting is per email only.** Put IP-based limits at the proxy/WAF in production.
- **No script Content-Security-Policy yet.** A nonce-based CSP would need middleware; only `frame-ancestors` is set.
- **Not checked in a real browser.** I couldn't open a browser in this environment. The UI was verified through server-rendered HTML, the compiled CSS (theme tokens, dark mode, responsive breakpoints) and the 31-step HTTP smoke test. A manual pass in a real browser on mobile and desktop is still recommended.
- **Times:** forms take local time (`datetime-local`) and pages show UTC.
- **PostgreSQL rounds decimals on raw inserts.** A direct-SQL insert of `1.5` into a BIGINT column is rounded by PostgreSQL's assignment cast. SQLite's STRICT tables reject it. The application always sends integers.

## 7. External credentials needed for deployment

1. **PostgreSQL** (managed is fine), with a dedicated database and user. Provide `DATABASE_URL` / `PAYROLL_DATABASE_URL` with `sslmode=require`.
2. **SMTP provider** (Postmark, SES, Resend, etc.) and a verified sending domain with SPF and DKIM. Provide `SMTP_URL` and `MAIL_FROM`.
3. **A public HTTPS domain** for `APP_URL`, with the TLS certificate from the host.
4. **`PAYROLL_API_TOKEN`:** generate it yourself (≥ 32 random characters) and store it in the host's secret manager for both processes.
5. **Hosting:**
   - a Node.js runtime for Next.js;
   - a place to run the Rust `payroll-api` binary on the same private network, not publicly exposed.

No wallet keys, Zcash infrastructure or payment credentials are needed. Payouts are out of scope until Phase 3.

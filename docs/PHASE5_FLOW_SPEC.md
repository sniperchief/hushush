# hushush Phase 5 — MVP flow redesign (spec, not yet built)

Status: **agreed with the product owner on 2026-10-14; built the same day — see [`PHASE5_REPORT.md`](PHASE5_REPORT.md)** (open points settled there, §1).
Start here in a new session. Background: `docs/PHASE0_REPORT.md` …
`docs/PHASE4_REPORT.md`, `docs/PHASE3_PAYOUTS.md` (payout system and config).

## Why

A real-browser rehearsal (2026-10-14) showed the Phase 2/3 flow is confusing:

- invitations by e-mail;
- organizations with roles;
- separate "claim" then "pay out" steps;
- employer and worker sharing one browser session.

In that session, the worker's claim UI showed up in what the employer
thought was their tab. The cause is that tabs in one browser share the
session cookie. A product shouldn't need "use incognito" instructions.

## Decisions (final)

| Topic | Decision |
|---|---|
| Main characters | **Employer** and **worker**, each with their **own home screen**; they never share UI |
| Accounts | **Both employer and worker must have an account.** A worker must sign up before entering a payout address |
| Login | **Passkeys (WebAuthn)**: sign up / sign in with Face ID, fingerprint or Windows Hello. **No passwords, no e-mail verification** |
| E-mail | Not used for auth anymore. SMTP (Gmail, already configured) is kept only for optional notifications later |
| Company model | **One employer account = one company** (MVP). Multiple admins come later |
| Streams | **Every stream has a start, an end date and a total amount** (rate derived from total ÷ duration). Earnings accrue per second, as today |
| Getting a worker onto a stream | Employer creates the stream and gets a **worker link** to share any way they like (copy / QR). The worker opens it, **signs up or signs in with a passkey**, and the stream binds to their account (the link is single-use once bound) |
| Payout address | The **worker** sets it **once per stream** after signing in. It's validated immediately (testnet Unified Address with an Orchard receiver, as today) and then **locked for the life of that stream** |
| Address changes | **Strict lock**, no exceptions and no escape hatch. To use a different address, the employer ends the stream and creates a new one. Money earned on the old stream can only go to the old address |
| Employer confirming the address | **Not required** |
| Withdrawals | **On demand** by the worker: one "Withdraw" button that pays **everything available** to the locked address |
| 20% rule | Withdraw is allowed when **available ≥ 20% of the stream's total**, so it unlocks at each 20% milestone (20/40/60/80/100%). If the worker waits, one withdrawal can take more. **After the stream ends, any remainder is withdrawable even if < 20%** (no stuck dust) |
| Fees | Paid by the employer's payout wallet on top of the amount (as today) |
| Network | **Testnet only.** No mainnet |

## Screens (target)

**Employer**

1. Passkey sign-up, then company name.
2. Home:
   - treasury card (spendable balance, funding address);
   - list of streams (worker, total, earned, withdrawn, status, end date);
   - payments with txids.
3. **New stream** form: worker display name, total ZEC, start (now/later), end date. On create, show the **worker link** with copy and QR buttons.
4. Stream detail: progress, payouts and their states (sending → confirming n/10 → paid, with txid), end stream.

**Worker**

1. Opens the link and sees "*Company* is paying you *X* ZEC from *date* to *date*", plus **Sign up / Sign in with passkey**.
2. Home: their streams, each with:
   - a **live ticking balance**;
   - a progress bar with 20% milestone marks;
   - available-to-withdraw.
3. **Set payout address** (once per stream) → shown with a lock icon.
4. **Withdraw** (enabled at milestones / after end) → Sending (≈1 min proof) → Confirming n/10 (≈4 min) → Paid ✓ with txid. Exactly-once: double-taps and refreshes never pay twice.

## What to reuse vs. change (implementation notes)

**Reuse unchanged** (proven by Phase 3–4 tests and live runs; keep their guarantees):

- `payout-wallet/` (wallet, intents, crash recovery, adoption);
- the payout state machine in `payroll/src/payout.rs`;
- `decide()` rules;
- exactly-once invariants;
- the accrual math (`payroll/src/accrual.rs`);
- PostgreSQL/Supabase support (CA, pools, RLS).

**Likely mapping onto the existing engine:**

| New concept | Existing engine |
|---|---|
| Employer account | user who owns a single organization |
| Worker joining via link | becomes a member of that organization |
| Stream with total + end date | `payroll_streams` with `end_at` required; rate = total ÷ duration (watch integer rounding: the total must be exactly reachable — consider storing the total and flooring accrual to it) |
| "Withdraw" | internally: reserve a claim for all available, then request a payout with the stream's locked address, in one action |
| Locked address per stream | a new column/table: store a salted fingerprint in payroll, and the address itself where the payout path can use it. Today the address is passed per request and **not stored** in payroll (privacy decision from Phase 3); revisit this deliberately |

**Remove from the UI (and auth):**

- password sign-up/login;
- e-mail verification;
- invitations;
- member/role management;
- the separate quote/claim screens.

**Web auth:** passkeys via a maintained WebAuthn library (e.g. `@simplewebauthn/server` + `@simplewebauthn/browser`).

- The Relying Party ID is the production host (`<app>.vercel.app` in production, `localhost` in dev).
- Keep the existing session-cookie design: hashed tokens, `HttpOnly`, idle and absolute expiry.
- Keep the Origin checks.

## Open points to settle at the start of the build

1. **Treasury per employer.** Today one payout wallet serves the whole deployment (`PAYOUT_ENABLED_ORGS`). With self-serve employer accounts, either:
   - (a) accept one shared demo treasury, with an allow-list of employer companies; or
   - (b) give each company its own wallet account and balance.

   Decide (a) for the hackathon unless told otherwise, and say so in the UI.
2. Can someone open a worker link **without** signing up to see a preview? The product owner said workers must have an account; a read-only preview before sign-up is optional.
3. Passkey recovery: allow adding a second passkey (phone + laptop) in the MVP?

## Current state of the code (2026-10-14)

- `main` = `07dcc81` (Phases 0–4) on GitHub.
- **Uncommitted** Supabase-readiness work, to be committed first:
  - DB CA support (`PAYROLL_DATABASE_CA_PEM`, `DATABASE_CA_PEM`);
  - pool sizing (`PAYROLL_DB_POOL_SIZE`, `DATABASE_POOL_MAX`);
  - RLS migrations (`payroll/migrations_pg/0003_rls.sql`, `web/migrations/0002_rls.sql`);
  - the concurrent-pay race fix in `payroll-api/src/payouts.rs`;
  - `web/scripts/check-db.mjs`;
  - `web/src/lib/pg-config.ts`.

  Verified on a TLS private-CA Postgres (web 56/56, payroll 101/101) and the real Supabase pre-flight passed.
- Deployment plan: Vercel (web), Railway (payroll-api + payout-wallet in one container, volume, EU West to match Supabase eu-west-1), Supabase (session pooler, CA cert), Gmail SMTP. Secrets live in `%LOCALAPPDATA%\hushush-dev\deploy.env` + `supabase-ca.crt` — never in the repo or chat.
- The Supabase project now holds the Phase 2-style tables and the rehearsal accounts. The redesign will change auth tables; plan a clean migration or reset.

## Non-negotiables carried over

- Testnet only.
- Never pay twice.
- Never release funds that may already have been broadcast.
- Keys stay isolated in `payout-wallet`.
- Mocked tests are labelled as mocked; nothing is claimed live unless run live.
- Keep the Phase 0–4 tests passing, or update them deliberately with a stated reason.

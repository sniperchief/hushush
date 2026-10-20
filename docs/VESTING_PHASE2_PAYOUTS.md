# Vesting Phase 2: payouts, API and UI

Vesting compensation (Phase 1: [VESTING_PHASE1_ACCOUNTING.md](VESTING_PHASE1_ACCOUNTING.md))
is now paid through the existing shielded payout pipeline and is usable from
the web app. Employers create, activate and cancel agreements. Employees see
their balances and withdraw what has vested. Streaming payroll is unchanged
and stays the primary product. Testnet only: mainnet payouts are not enabled.

## 1. How vesting claims join the payout lifecycle

There is one `payouts` table, one state machine, one worker and one wallet
for both compensation types.

* **Explicit reference.** Each payout has a `compensation_type`
  (`stream` | `vesting`) and exactly one claim reference:
  * stream payouts set `claim_id` and `stream_id`;
  * vesting payouts set `vesting_claim_id` and `vesting_agreement_id`.

  A CHECK constraint enforces "exactly one, matching the type". Each
  reference column is UNIQUE and has its own foreign key, so a claim can
  have at most one payout ever. In Rust, `Payout` carries
  `compensation_type`, `claim_id` and `source_id` (the stream or agreement).
* **Consistency on insert.** The insert trigger accepts a vesting payout
  only when it matches its claim: reserved, same claimant, same amount and
  same agreement.
* **Settlement.** `apply_payout_decision` settles or releases the claim of
  the payout's own type, in the same transaction as the state change:
  * only a verified, confirmed payout settles a claim (the existing
    confirmation rule, `settlement_ref` = txid);
  * only a proven failure releases it.

  Reserving and submitting never mark anything paid.
* **No other way to resolve a claim with a payout.**
  `release_vesting_claim` and `settle_vesting_claim` refuse claims that
  have a payout. A schema trigger (`vesting_claims_resolved_by_payout`)
  enforces the same rule against direct SQL.
* **The stream path ignores vesting claims.** `request_payout`,
  `payout_eligibility` and `release_claim` read only `claims`, so a
  vesting-claim id is "claim not found" there.
* **Wallet unchanged.** The payout id is the wallet's payment id. The wallet
  builds at most one transaction per id and derives the memo from it. There
  is no wallet change, no new transaction-construction path, and no vesting
  data in memos or on chain.

## 2. Atomic withdrawal, idempotency, recovery

`Payroll::withdraw_vesting` locks the agreement row (`SELECT … FOR UPDATE`
on PostgreSQL, `BEGIN IMMEDIATE` on SQLite). Under that lock it:
1. checks the actor is the employee;
2. handles idempotency;
3. checks the locked address fingerprint;
4. checks `amount ≤ available`;
5. inserts the vesting claim and its `reserved` payout (leased to the
   requesting process), and commits.

payroll-api then calls the unchanged `submit_payout` with the organization's
own payout account.

**Exactly-once:**
* the same key with the same amount returns the original payout, in any
  state;
* the same key with a different amount is an `IdempotencyConflict`;
* any request while a withdrawal of the agreement is unfinished returns
  that withdrawal (one open withdrawal per agreement, as for streams).

Agreement cancellation takes the same row lock, so it serializes with
withdrawals.

| Failure point | What happens (tested) |
|---|---|
| Before the reservation commits | The transaction rolls back: no claim, no payout. A retry with the same key is a fresh request. |
| After commit, before the transaction is built | When the lease expires, the worker fences the payment id at the wallet (`cancel`), marks the payout `failed` and releases the claim. If the crashed process later submits, the fence stops it and no transaction is built. |
| After a signed transaction is stored (built, not acknowledged) | `requires_reconciliation` with the txid recorded. Once the wallet's rebroadcast is acknowledged it becomes `submitted`, then `confirmed` and settled. |
| After broadcast, response lost | `requires_reconciliation`. The worker adopts the wallet's record (`submitted`), then confirms it. A lost request is fenced and failed instead. |
| After network confirmation, before local settlement | The settlement transaction rolls back (payout stays `submitted`, claim `reserved`). A restarted worker with a new connection and lease settles it exactly once. |
| Repeats after any of these | The same payout is returned; the mock wallet builds at most one transaction per payout in every scenario. |

## 3. Lifecycle changes

* **Late activation is refused.** An agreement can be activated only at or
  before `start_at`; activating exactly at the start is allowed because
  nothing has vested yet. This is enforced in the engine and by a schema
  trigger. A draft whose start has passed can only be discarded (voided) and
  created again; the UI says so.
* **Cancellation semantics are unchanged from Phase 1.** Vesting stops at
  cancellation; vested, unpaid compensation stays owed and withdrawable;
  the rest is forfeited. In-flight payouts continue and settle (or release)
  normally, and confirmed payouts are untouched. A fully vested agreement
  can't be cancelled.
* **Locked payout address per agreement** (`vesting_payout_addresses`): the
  employee sets it once, after activation, and it is then locked. The wallet
  validates it first (testnet Unified Address with an Orchard receiver).
  It is stored sealed (XChaCha20-Poly1305 under `PAYOUT_ADDRESS_KEY`, with
  associated data `vesting:<agreement id>`, so it can't be moved to another
  agreement or onto a stream), next to a salted fingerprint. It is never
  logged and is returned only to the employee.

## 4. Migrations

* SQLite `0005_vesting_payouts` (schema v5); PostgreSQL `0006_vesting_payouts`
  (schema v6).
* SQLite can't drop `NOT NULL`, so `payouts` and `payout_events` are rebuilt
  inside the migration transaction with every row copied (ids kept; the
  autoincrement continues), and the trigger on `claims` that reads `payouts`
  is recreated. PostgreSQL alters `payouts` in place.
* Existing rows become `compensation_type = 'stream'` with their references
  unchanged.
* **Upgrade tests** (automated, `upgrade_keeps_stream_payouts` on both
  backends) start from the previous version populated with a confirmed
  stream payout (claim settled) and an in-flight one. After the upgrade:
  * both payouts, their claims and their event history are intact;
  * the foreign-key check is clean and the rebuilt tables' guards still fire;
  * the in-flight payout is reconciled to confirmed and settled once by the
    new code;
  * a vesting withdrawal then works on the same database.
* payroll-api applies the migration automatically on its next start against
  a database.

## 5. payroll-api routes

The conventions are unchanged: `Authorization: Bearer $PAYROLL_API_TOKEN`
and `X-Actor-Id` (the user the web app authenticated) on every request, JSON
errors from `PayrollError`, and amounts as exact ZEC strings. The engine
checks the actor against the database for every decision.

| Route | Who | Notes |
|---|---|---|
| `GET /v1/orgs/{org}/vesting` | owner/admin of `org` | Agreements with balances. |
| `POST /v1/orgs/{org}/vesting` | owner/admin | Creates a draft (`employee_user_id` must be a member, not the caller). |
| `GET /v1/me/vesting` | any user | Their own activated agreements. |
| `GET /v1/vesting/{id}` | employee (once activated) or owner/admin | Agreement, balances, schedule, events, claims and payouts; the address goes to the employee only. |
| `POST /v1/vesting/{id}/activate` | owner/admin | Only at or before the start. |
| `POST /v1/vesting/{id}/cancel` | owner/admin | Drafts are voided; agreements not yet fully vested are cancelled. |
| `POST /v1/vesting/{id}/address` | employee | Set once; validated by the wallet. |
| `POST /v1/vesting/{id}/quote` | employee | Fee from a real wallet proposal for the amount, from the org's account. Reserves nothing. |
| `POST /v1/vesting/{id}/withdraw` | employee | Atomic reserve plus payout, then submit. Exactly-once (§2). |

* **Outsiders** get 404 (existence isn't revealed).
* **Managers on employee-only routes** get 403 (they never reach the
  wallet, so they can't probe addresses).
* **Missing or invalid service token** gets 401.
* **Shared payout view:** `PayoutView` now includes `compensation_type`.
* **Background worker:** the existing payout worker reconciles both types.

## 6. Web app

**Route handlers** (`/api/vesting`, `/api/vesting/[id]/{activate,cancel,address,quote,withdraw}`):
* same-origin check, session user, zod validation;
* the company comes from the session, never from the request;
* the service token stays server-side (`lib/payroll.ts` is `server-only`).

**Employer** (dashboard "Vesting" section and nav item, `/app/vesting/new`,
`/app/vesting/[id]`):
* list of agreements with vested, paid and in-flight amounts, and status;
* the create form: an employee picked from the company's members, total in
  ZEC, start, duration and an optional cliff in days;
* a review step that shows the schedule in plain language before the draft
  is created;
* activate and cancel/discard, with confirmations;
* the five balances, the address status, the payout list and the history.

**Employee** (dashboard "Vesting" section and nav item, `/app/vesting/[id]`):
* organization, total, the five balances and a progress bar;
* the schedule with start, cliff and end dates, and the status;
* the claim flow: lock the address → enter an amount → **Review** shows the
  amount, the network fee (paid by the company's payout wallet), the
  destination and the confirmation threshold → **Confirm**;
* live payout status (Sending / Confirming n/N / Paid / Not sent /
  Checking), using the existing badges;
* withdrawal history and agreement history.

**Display rules:**
* **Server values only.** All balances, eligibility, `withdrawable` and
  phase come from the server. The browser does no authoritative arithmetic.
  The only client calculation is the creation form's preview of the cliff
  share, in exact `BigInt` zatoshis with the engine's formula; the server
  recomputes it.
* **Cliff wording matches the engine:** "Nothing vests during the first N
  days … At the cliff, the share that would have vested over that period —
  X ZEC — vests at once. After that, the rest vests evenly …"
* **"Paid" is shown only for confirmed payouts.**
* **Privacy note** (employee page): the amount and recipient aren't visible
  on chain; that a transaction happened, its timing and its fee are.

## 7. Test results (2026-10-19)

| Suite | Result |
|---|---|
| payroll unit | 26 passed |
| `tests/backends.rs` (both backends) | 17 passed |
| `tests/engine.rs` | 29 passed |
| `tests/flow.rs` (both backends) | 24 passed |
| `tests/payouts.rs` (both backends, stream payout regression) | 41 passed |
| `tests/vesting.rs` (both backends, Phase 1, updated for late activation) | 32 passed |
| `tests/vesting_payouts.rs` (new; 12 scenarios × 2 backends + 2 upgrade tests) | 26 passed |
| **payroll total** (SQLite + PostgreSQL 17) | **195 passed** (was 169) |
| payroll-api unit | 5 passed |
| web vitest (real payroll-api + PostgreSQL, MOCK wallet), 7 files | **84 passed** (10 new in `tests/vesting.test.ts`) |
| payout-wallet unit (unchanged crate) | 12 passed |
| wallet-proof offline tests (unchanged crate, no dependency on payroll) | **not completed**: three builds (its own `target/`, after `cargo clean`, and a fresh separate directory) failed with corrupted compiler output (`E0786 invalid metadata files`, `E0462`). The toolchain is intact and the code is unchanged, so this looks like a machine problem (the disk filled up earlier in the session; antivirus file locking is also possible). Rerun on a healthy machine. |
| web typecheck / lint / production build | clean / 0 errors (4 existing warnings in `scripts/check-db.mjs`) / succeeds, all vesting routes and pages built |
| `cargo fmt --check` + `clippy -D warnings` | payroll and payroll-api clean; payout-wallet and wallet-proof clippy clean, `fmt --check` reports formatting drift that predates this phase (files unchanged here, not reformatted) |

**Scenarios covered by `tests/vesting_payouts.rs`:**
* **Payout integrity:** exactly one claim reference; both or neither
  refused; wrong agreement, employee, amount or type refused; a second
  payout per claim refused; payout terms immutable.
* **Claims with a payout:** can't be released or settled by hand (engine
  and SQL); the stream path refuses vesting claims.
* **Withdrawals:** repeats and concurrent withdrawals (8 threads) create
  one payout and one transaction; who may withdraw or set the address;
  unvested compensation and amounts above the available balance are
  refused.
* **Recovery:** each failure point in §2.
* **Cancellation:** with in-flight and failed payouts.
* **Activation:** before the start, exactly at it, and refused after it.

**`tests/vesting.test.ts`** goes through the real route handlers and
payroll-api:
* create, review terms and member eligibility;
* draft invisibility, then activation;
* refusal of late activation;
* cross-organization and role checks;
* 401 on a missing or invalid service token, CSRF and sessions;
* the cliff, then quote → withdraw → confirmed, with a duplicate and a
  concurrent withdrawal each paying once;
* a failed withdrawal releasing its amount;
* cancellation with an in-flight withdrawal.

**Not run:** live testnet payouts. All payout tests use the mock wallet.
The real wallet path is unchanged and was last verified live in Phase 6.

## 8. Gaps and deployment notes

* **Eligible employees are existing members.** People become members by
  accepting a stream's worker link. There is no vesting-only invitation yet.
* **One open withdrawal per agreement.** A second request while one is
  unfinished returns that one, as for streams.
* **Employer "Payments" card** still lists stream withdrawals only. Vesting
  withdrawals appear on each agreement's page, and their in-flight amount
  on the Vesting card.
* **No four-eyes control** for managers (unchanged from Phase 1).
* **Fee quote ≠ final fee.** The quote is a real proposal, but notes can
  change before submission. The payout's `max_fee` cap (estimate plus
  headroom) still bounds it.
* **Deployment:**
  * deploy payroll-api and the web app together; payroll-api migrates
    PostgreSQL to v6 on start;
  * no new environment variables (the existing `PAYOUT_ADDRESS_KEY`,
    `WALLET_*` and `PAYOUT_ENABLED_ORGS` apply);
  * mainnet stays disabled.
* **Live testnet run still to do.** A vesting withdrawal hasn't yet been made
  against the real payout wallet on testnet; that is recommended before
  calling this production-ready.

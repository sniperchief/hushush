# Vesting Phase 1: vesting engine and accounting

Vesting compensation is a second compensation type: a fixed ZEC allocation that
vests to one employee over time. Streaming payroll stays the core product and
is unchanged. This phase is the accounting foundation only: domain model,
schema (SQLite and PostgreSQL), engine operations, and tests. There is no UI,
no payroll-api route, no payout and no transaction yet.

> **Updated by Vesting Phase 2** ([VESTING_PHASE2_PAYOUTS.md](VESTING_PHASE2_PAYOUTS.md)):
> vesting claims are now paid through the payout state machine, late
> activation is refused, and the API and UI exist. Sections 7 and 8 below
> describe the state at the end of Phase 1.

Code: [`payroll/src/vesting.rs`](../payroll/src/vesting.rs) (rules and
operations), types in [`payroll/src/model.rs`](../payroll/src/model.rs),
storage in `sqlite.rs` / `pg.rs`, schema in
[`migrations/0004_vesting.sql`](../payroll/migrations/0004_vesting.sql) and
[`migrations_pg/0005_vesting.sql`](../payroll/migrations_pg/0005_vesting.sql),
tests in [`payroll/tests/vesting.rs`](../payroll/tests/vesting.rs).

## 1. Model

A **vesting agreement** (`vesting_agreements`) is its own entity. Streams,
stream claims (`claims`) and payouts are not reused or altered.

| Field | Meaning |
|---|---|
| `organization_id`, `employee_user_id` | Granting organization; the employee must be a member (composite FK to `organization_members`). |
| `created_by` | Owner/admin who created it; never the employee (engine + CHECK). |
| `total_zats` | The whole allocation, integer zatoshis, 1 ..= 21M ZEC. |
| `start_at` | Vesting start (unix seconds). Defaults to now; never before the agreement was created. |
| `duration_secs` | 1 s ..= 100 years. End = `start_at + duration_secs`. |
| `cliff_secs` | 0 (no cliff) ..= `duration_secs`. |
| `method` | `linear` (the only method). |
| `status` | `draft` → `active` → `cancelled` (also `draft` → `cancelled`). |
| `created_at`, `updated_at`, `activated_at/by`, `cancelled_at/by` | Lifecycle metadata. |
| `vested_at_cancel_zats` | Vested amount frozen at cancellation (0 for a voided draft). |

Terms are immutable after creation (schema triggers). Agreements, claims and
events can't be deleted. No keys, addresses, seeds or wallet credentials are
stored anywhere in these tables.

The **phase** is derived from status and the server clock, never stored:
`draft`, `not_started`, `in_cliff`, `vesting`, `fully_vested`, `cancelled`.

## 2. Vesting rule

Integer arithmetic only (i128 intermediate, no overflow), at the trusted
server clock `t`. Callers can never supply a time.

```text
elapsed   = t - start_at
vested(t) = 0                                       if elapsed <= 0 or elapsed < cliff_secs
          = total                                   if elapsed >= duration_secs
          = floor(total * elapsed / duration_secs)  otherwise
```

* **Cliff:** nothing vests before `start_at + cliff_secs`. Exactly at the
  cliff, the linear share up to the cliff vests at once
  (`floor(total × cliff / duration)`; e.g. a 1-year cliff on a 4-year grant
  releases 25%). After the cliff, vesting is linear. `cliff = duration` vests
  everything at the end.
* **Rounding:** floor, taken once over the whole elapsed time. Vesting is
  monotonic, never exceeds the total, and is never more than 1 zatoshi below
  the exact value before the end.
* **End:** at and after the end, vested = `total` exactly, by an explicit
  branch. The fully vested amount always reconciles to the allocation.
* A `draft` vests nothing. A `cancelled` agreement's vested amount is the
  frozen `vested_at_cancel_zats`.

## 3. Accounting

`VestingBalance` keeps the concepts apart:

| | |
|---|---|
| `total` | the allocation |
| `vested` | per §2 |
| `unvested` | `total - vested` (forfeited for good once cancelled) |
| `paid` | sum of `settled` claims |
| `reserved` | sum of `reserved` claims (held, not paid) |
| `available` | `vested - reserved - paid` |

**Invariants** (engine and schema agree):

1. `vested + unvested = total`; `0 <= vested <= total`.
2. `reserved + paid <= vested`, so `available >= 0`. A reservation is only
   made for `amount <= available` under the agreement's row lock. The schema
   re-checks `amount + reserved + paid <= vested_at_reservation`, and
   `vested_at_reservation <= total`, or `<= vested_at_cancel_zats` once cancelled.
3. Unvested compensation is never claimable. Before the start or during the
   cliff, `available = 0`.
4. A claim moves only `reserved → released` or `reserved → settled`, and
   `settled` is final. A paid amount can never become available again, even
   by direct SQL.
5. Idempotency: `UNIQUE (agreement_id, idempotency_key)`. A retry with the
   same key and amount returns the original claim (`duplicate = true`),
   whatever its status, and never reserves again. The same key with a
   different amount is `IdempotencyConflict`.

**Vesting claims** (`vesting_claims`) reuse the stream-claim design but live in
their own table with a non-null FK to the agreement. `VestingClaim` also
carries `compensation_type = "vesting"` for API consumers. A stream-claim id
is unknown to the vesting operations, and the reverse holds as well. The
payout path (`payout_eligibility` / `request_payout`) only reads `claims`, so
it can't touch a vesting claim (tested).

**Concurrency:** every reservation and every lifecycle change runs in a write
transaction that locks the agreement row (`SELECT … FOR UPDATE` on
PostgreSQL, `BEGIN IMMEDIATE` on SQLite). Release and settle lock the claim
row and use a compare-and-set on `status = 'reserved'`.

## 4. Lifecycle and cancellation policy

| Operation | Who | Rule |
|---|---|---|
| `create_vesting` | owner/admin of the org | Creates a `draft` for a member who isn't the creator. Terms validated (§1). |
| `activate_vesting` | owner/admin | `draft → active`. Vesting follows the agreed schedule from `start_at`. |
| `cancel_vesting` | owner/admin | See below. |
| `reserve_vesting_claim` | the employee only | `active` or `cancelled` agreements, up to `available`. |
| `release_vesting_claim` | claimant or owner/admin | `reserved → released`; the amount is available again. |
| `settle_vesting_claim` | owner/admin | `reserved → settled` with a reference (accounting only, as for `settle_claim`). |
| `get_vesting`, `vesting_balance`, `vesting_events`, `list_vesting_claims` | employee (once activated) or owner/admin | Others get `NotFound`, so nothing leaks. |
| `list_org_vesting` / `list_my_vesting` | owner/admin / the employee | `list_my_vesting` shows activated agreements only. |

**Cancellation policy:**

* Cancelling an **active** agreement stops vesting at the cancellation time.
  The vested amount at that moment is frozen and stored, and stays owed:
  unpaid vested compensation remains claimable afterwards. The unvested
  remainder is forfeited and can never become claimable (engine, plus a
  schema cap on new claims). Cancelling before the start or during the cliff
  freezes 0.
* Cancelling a **draft** voids it. Nothing was ever owed and the employee
  never sees it.
* A **fully vested** agreement can't be cancelled, because there is nothing
  left to stop.
* **Claims are untouched.** A reserved claim was reserved against vested
  compensation, which stays vested, so an in-progress payment can still
  settle, or release on failure, after cancellation. Settled claims stay
  settled. The schema refuses a cancellation that would freeze less than is
  already reserved plus paid. The engine refuses a cancellation (or
  activation) if the server clock is behind the agreement's last change
  (`ClockSkew`).
* `cancelled` is final (schema trigger).

Every lifecycle change (`created`, `activated`, `cancelled`) is written to the
append-only `vesting_events` with the actor, the server time and the vested
amount after the change, in the same transaction.

**Authorization** is server-side and from the database only. Every operation
takes the authenticated user id (`actor`, which payroll-api receives from the
web server as `X-Actor-Id`) and checks it against `organization_members` and
the agreement row. Organization, employee and role are never taken from the
caller's word. An organization id supplied by the caller must be one the
actor manages, and the employee must be a member of that organization.
`remove_member` now also refuses members who have vesting agreements, as it
already did for streams.

## 5. Migrations

* SQLite `0004_vesting` (schema v4), PostgreSQL `0005_vesting` (schema v5,
  including RLS on the three new tables, as in `0003_rls`). Both only add
  tables, indexes and triggers; no existing table changes.
* Checked on databases with existing data: SQLite v3 → v4 (via
  `payroll-migrate`) and PostgreSQL v4 → v5. Existing streams and claims were
  intact, foreign keys and integrity checks were clean, and new vesting rows
  inserted fine.
* payroll-api applies v5 automatically the next time it starts against a
  database.

## 6. Tests

`cargo test -- --include-ignored` with `PAYROLL_TEST_DATABASE_URL` set
(local PostgreSQL 17), on 2026-10-19:

| Suite | Before | After |
|---|---|---|
| unit (`src/`) | 17 | 26 (+9 vesting arithmetic) |
| `tests/backends.rs` | 17 | 17 |
| `tests/engine.rs` | 29 | 29 |
| `tests/flow.rs` | 24 | 24 |
| `tests/payouts.rs` | 41 | 41 |
| `tests/vesting.rs` (SQLite + PostgreSQL) | — | 32 (16 scenarios × 2) |
| **total** | **128** | **169**, all passing |

`tests/vesting.rs` was also run three more times against both backends
(concurrency scenarios), and passed every time. `cargo fmt --check` and
`cargo clippy --all-targets -D warnings` are clean for `payroll`.
payroll-api: clippy clean, 5/5 tests pass.

Coverage:

* **Arithmetic:** zero elapsed, just before / at / after the cliff, midway, one
  second before the end, the end, after the end, 1-second and 100-year
  durations (including 21M ZEC), 3-zatoshi and 1-zatoshi allocations,
  cliff = duration, monotonicity, the floor bound, and fully vested = total
  (unit tests and through the engine on both backends).
* **Claims:** less than available, exactly all available, more than
  available, with existing reservations, unvested rejected, release of a
  failed reservation (by the claimant and by a manager), one-way
  transitions, retries of released and settled claims, concurrent claims
  (16 threads), concurrent duplicates (8 threads, one claim), settled claims
  final (including against a direct SQL write).
* **Lifecycle and authorization:** draft invisibility, activation,
  cancellation (active, not started, in cliff, draft, fully vested, twice),
  claims racing a cancellation, the audit log and its append-only guard,
  clock skew, create/view/cancel/claim/release/settle by employee,
  colleague, stranger, rival-org owner and managers, organization
  boundaries, self-grants, member removal, stream and vesting ids kept apart,
  and schema guards against direct writes on both backends.

Not run in this phase: the web test suite (the web app is unchanged) and live
testnet runs (no payouts are involved).

## 7. Gaps and open points

* **No payouts for vesting claims yet.** Settlement is an owner/admin
  accounting record (`settle_vesting_claim`), like `settle_claim`. Phase 2
  must connect vesting claims to the payout state machine. Until then,
  "in-progress payout" means "reserved claim".
* **No HTTP routes** in payroll-api and no UI yet, so nothing is exposed to
  users.
* **Late activation is retroactive.** Vesting runs from the agreed
  `start_at`, so a draft activated long after its start vests everything due
  since then at once. The employer controls both steps, and `start_at` can't
  predate creation. A product decision may want to require activation before
  the start date instead.
* **No four-eyes control.** Any owner/admin can create, activate, cancel and
  settle (except for themselves at creation). An admin who is also an
  employee could activate an agreement another admin created for them.
* **Clock trust.** The schedule depends on the server clock. Backwards jumps
  are refused for lifecycle changes, but forward jumps would vest early (as
  for streams).
* **Not in scope:** no partial-cancel or amendment of terms (cancel and
  create a new agreement instead), and no reason field on cancellation.
* payroll-api's `cargo fmt --check` reports existing formatting drift in
  `src/flow.rs` and `src/payouts.rs`, which this phase didn't touch. It was
  left alone.

## 8. Interfaces for Phase 2

Engine (`hushush_payroll`), all taking the authenticated `actor`:

```rust
create_vesting(actor, NewVesting) -> VestingAgreement          // draft
activate_vesting(actor, id) -> VestingAgreement
cancel_vesting(actor, id) -> VestingAgreement
get_vesting(actor, id) -> VestingAgreement
vesting_balance(actor, id) -> VestingBalance
vesting_events(actor, id) -> Vec<VestingEvent>
list_org_vesting(actor, org) -> Vec<VestingListing>
list_my_vesting(actor) -> Vec<VestingListing>
reserve_vesting_claim(actor, id, amount_zats, idempotency_key) -> VestingClaimOutcome
release_vesting_claim(actor, claim_id) -> VestingClaim
settle_vesting_claim(actor, claim_id, settlement_ref) -> VestingClaim   // to be replaced by payout confirmation
list_vesting_claims(actor, id) -> Vec<VestingClaim>
vesting::vested_amount(&VestingTerms, t) -> i64                        // pure
```

Still needed in Phase 2:

1. **Payout linkage.** Let `payouts` reference a vesting claim explicitly.
   For example, add a nullable `vesting_claim_id` (FK to `vesting_claims`)
   next to `claim_id`, with a CHECK that exactly one is set and the
   insert/resolve triggers mirrored. Or use a separate `vesting_payouts`
   table on the same state machine. Then `apply_payout_decision` settles or
   releases the right claim table, and `release_vesting_claim` /
   `settle_vesting_claim` must refuse claims that have a payout, as
   `resolve_claim` does today.
2. **A locked payout address per agreement,** like
   `stream_payout_addresses`, and a `withdraw`-style single action that
   reserves and records the payout atomically under the agreement lock.
3. **payroll-api routes** under the existing conventions (service token plus
   `X-Actor-Id`, JSON errors from `PayrollError`, amounts as ZEC strings via
   `format_zec`/`parse_zec`). For example: `POST /v1/orgs/{org}/vesting`,
   `POST /v1/vesting/{id}/activate`, `POST /v1/vesting/{id}/cancel`,
   `GET /v1/vesting/{id}`, `GET /v1/me/vesting`,
   `POST /v1/vesting/{id}/claims`.
4. **The web UI** (vesting section on the employer and earner dashboards).

# hushush Phase 1 — Payroll Foundation

Date: 2026-10-13 · Code: [`payroll/`](../payroll/) (crate `hushush-payroll`)

Phase 1 adds the payroll engine and its database: organizations, members,
payroll streams with server-side accrual, an append-only stream event log, and
claims as **accounting reservations only**. Nothing in `payroll/` calls the
Zcash wallet; no payouts happen. The Phase 0 wallet proof
([`wallet-proof/`](../wallet-proof/)) is unchanged.

## 1. Stack and layout

| | |
|---|---|
| Language | Rust 2024 edition (same toolchain as Phase 0: rustc 1.99.0, MSVC) |
| Database | SQLite via `rusqlite` 0.37 (bundled SQLite), the library Phase 0 already uses. WAL journal, `synchronous=FULL`, `foreign_keys=ON`, `STRICT` tables |
| Other deps | `uuid` (v4 ids), `thiserror`; dev: `tempfile` |
| Layout | `payroll/migrations/*.sql` (schema), `src/accrual.rs` (pure math), `src/money.rs` (zatoshi parsing/formatting), `src/engine.rs` (operations + authorization), `src/migrations.rs` (runner), `src/bin/payroll-migrate.rs` (CLI), `tests/engine.rs` |

It is a library with no HTTP API. Callers pass the authenticated user's id as
`actor`; authentication itself is out of scope for this phase.

## 2. Schema (migration `0001_init`)

All money is `INTEGER` zatoshis (1 ZEC = 100,000,000). All times are `INTEGER`
unix seconds from the **server clock**. `STRICT` typing rejects REAL values, so
a floating-point amount cannot even be stored.

| Table | Purpose | Key constraints |
|---|---|---|
| `users` | People | UUID id; unique case-insensitive email; length checks |
| `organizations` | Employers | `created_by` → users |
| `organization_members` | Membership + role | PK `(organization_id, user_id)`; role ∈ `owner/admin/member` |
| `payroll_streams` | One pay arrangement | `rate_zats` 1..21M ZEC; `rate_period_secs` 1..100 years; `end_at > start_at`; status ∈ `active/paused/ended`; `active_since IS NOT NULL ⇔ active`; `ended_at IS NOT NULL ⇔ ended`; **composite FK `(organization_id, recipient_user_id)` → organization_members**, so the recipient must be a member |
| `stream_events` | Audit log: created/paused/resumed/ended, actor, time, active seconds and earned zats after the event | Append-only (UPDATE/DELETE triggers abort) |
| `claims` | Reservations against earned wages | `amount_zats > 0`; status ∈ `reserved/released/settled`; `UNIQUE (stream_id, idempotency_key)`; resolved fields set ⇔ not reserved; `settlement_ref` set ⇔ settled |
| `schema_migrations` | Applied migrations | Mirrors `PRAGMA user_version` |

Triggers (defence in depth, enforced even against direct SQL):

- **Stream terms are immutable:** rate, period, start, end, org, recipient, creator.
- **Stream state only moves forward:** `banked_active_secs` and `last_event_at` never decrease, and `ended` is final.
- **No deletes** of streams, events or claims.
- **Claims must be created `reserved`.** The only allowed transitions are `reserved → released` and `reserved → settled`. Amount, stream, requester and key are immutable.
- **`claims_no_overdraw`:** inserting a reservation fails if reserved + settled + new amount would exceed the earned amount the server computed for it in the same transaction.

## 3. Accrual formula

A stream pays `rate_zats` per `rate_period_secs` of **active** time:

```text
active_secs(t) = banked_active_secs + max(0, min(t, end_at) − active_since)   [if active]
               = banked_active_secs                                          [if paused/ended]
earned(t)      = min(21,000,000 ZEC, ⌊ rate_zats × active_secs(t) / rate_period_secs ⌋)
```

- **Integer only.** The product is computed in `i128`, so it can't overflow; the result is floored (never rounded up) and saturates at the total money supply.
- **Floored once over total active time,** not per interval. Pausing and resuming never loses fractional zatoshis (tested: 70 one-second intervals at 1 zat / 7 s = exactly 10 zats).
- **Pause / end** banks the open interval: `banked = active_secs(now)`, `active_since = NULL`. Earned wages are kept.
- **Resume** sets `active_since = max(now, start_at)`. It is rejected once `end_at` has passed.
- **`end_at`** caps accrual exactly at the end second. An active stream past `end_at` reports status `ended`.
- **`start_at`** defaults to now and may not be in the past, so there is no retroactive accrual (see §7). Before `start_at`, earned is 0.
- `t` is always the server clock. If the clock is behind the stream's last event, lifecycle writes are refused (`ClockSkew`) rather than rewriting history.

Example: 1 ZEC/hour → after 1 s, ⌊100,000,000 / 3,600⌋ = 27,777 zats.

## 4. Claim accounting rules

For each stream at time `now`:

```text
earned    = earned(now)
reserved  = Σ amount of claims with status 'reserved'
claimed   = Σ amount of claims with status 'settled'
available = earned − reserved − claimed      (invariant: ≥ 0; engine errors if ever violated)
```

| Operation | Who | Rule |
|---|---|---|
| `reserve_claim(amount, idempotency_key)` | Stream recipient only | In one `BEGIN IMMEDIATE` transaction: recompute `available` from the server clock, reject if `amount > available`, insert `reserved`. Allowed on paused and ended streams (earned wages stay claimable). |
| Duplicate request (same key) | — | Returns the original claim with `duplicate = true`, whatever its current status; creates nothing. Same key with a different amount → `IdempotencyConflict`. |
| `release_claim` | Claimant, or owner/admin | `reserved → released`; the amount becomes available again. |
| `settle_claim(settlement_ref)` | Owner/admin | `reserved → settled`; moves the amount from reserved to claimed. **Accounting only in Phase 1.** It is the hook a Phase 2 payout worker will call with a txid. |

**Concurrency:** every write uses `BEGIN IMMEDIATE`. SQLite admits one writer at
a time across all connections and processes, so a reservation's
read-check-insert can't interleave with another. The `UNIQUE` key and the
overdraw trigger back this up at the storage layer.

**Authorization summary:**

| Action | Allowed for |
|---|---|
| Manage streams (create, pause, resume, end) | org owner/admin |
| Add members | owner: any role; admin: `member` only |
| Claim | the stream's recipient only; admins cannot claim on a worker's behalf |
| View stream, balance, claims, events | the recipient, or org owner/admin |

Outsiders get `NotFound`, so they can't tell whether a stream exists.

## 5. Setup and migrations

```sh
cd payroll
cargo build
cargo run --bin payroll-migrate -- /path/to/payroll.sqlite   # apply pending migrations
```

`PayrollDb::open(path, clock)` also applies pending migrations on open. Rules:

- Migrations are forward-only and numbered (`migrations/NNNN_name.sql`, registered in `src/migrations.rs`).
- Each one runs in its own transaction.
- A database newer than the binary is refused (`SchemaTooNew`).

Keep the database file outside the repo; `.gitignore` already excludes `*.sqlite`.

To add a migration:

1. Add `migrations/0002_….sql`.
2. Append it to `MIGRATIONS`.
3. Never edit an applied file.

## 6. Test results (2026-10-13)

| Command | Result |
|---|---|
| `cargo build --all-targets` | OK, 0 warnings |
| `cargo test` | **41 passed, 0 failed** (12 unit + 29 integration) |
| Concurrency tests repeated 30× (`cargo test --test engine concurrent`) | 30/30 passed |
| `cargo clippy --all-targets -- -D warnings` | OK |
| `cargo fmt --check` | OK |
| `payroll-migrate` on a fresh DB, then again | applied `[1]`, then "up to date" |
| Phase 0 regression (`wallet-proof`: `cargo test`, unchanged code) | 24 passed (12 unit, 6 mocked-chain, 6 recovery); 3 live tests ignored by default, not re-run |

All payroll tests are **offline**, against real SQLite files with a manual
clock; there is no network and no wallet. Coverage:

- **Accrual boundaries:** exact start second, flooring, end-date cap, saturation.
- **Pauses and resumes:** many short pauses lose no zats; pausing before start; resuming after the end date is rejected.
- **End dates and end operation:** wages stay claimable after end.
- **Claim reservations:** overdraw, unearned future wages, release, settle.
- **Duplicate requests:** sequential, and 12 concurrent copies.
- **Concurrent claims:** 16 threads racing (exactly 6 of 16 succeed); claims racing a pause.
- **Authorization matrix.**
- **Schema tampering via raw SQL:** REAL amounts, overdraw, immutable terms, append-only log, no reopening an ended stream, recipient FK.
- **Migrations:** idempotent re-open with state persisted; newer schema refused.

## 7. Decisions made, and decisions still needed

Made in this phase (easy to revisit):

- **SQLite instead of Postgres.** It matches the existing stack and needs no server. For a multi-instance deployment, port the schema to Postgres (`SERIALIZABLE`, or `SELECT … FOR UPDATE` on the stream row). The engine's transaction boundaries already match that model.
- **No retroactive start dates,** to prevent back-dated wage grants.
- **Fixed rate per stream.** A raise means ending the stream and creating a new one.
- **Second granularity.**
- **A user may claim only their own streams.**

Still required from you before later phases:

1. **Authentication / identity provider.** Who issues the `actor` id: email + password, OAuth, or wallet-based sign-in?
2. **Production database.** Stay on SQLite on a single server, or move to Postgres.
3. **Payout policy for Phase 2,** including:
   - minimum claim size, given ZIP 317 fees of 5,000 zats per action;
   - who pays fees;
   - batching cadence;
   - key custody, still open from Phase 0.
4. **Business rules:**
   - whether back-dated streams should be allowed (with an audit reason);
   - rate changes mid-stream;
   - currency other than ZEC.

No credentials are needed for Phase 1.

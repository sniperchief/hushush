# OBS-1 fix: chain-aware resending in the payout wallet

Follows the OBS-1 investigation of
[VESTING_PHASE4_FAULT_INJECTION.md](VESTING_PHASE4_FAULT_INJECTION.md) §5.

**Summary:**
* A wallet-only change, with automated tests.
* No live transaction was broadcast and no service was started, stopped or
  restarted.
* Wallet data, funds and seeds were not touched. Mainnet stays disabled.
* Nothing was committed.

## 1. Root cause (confirmed)

**Evidence:** in Phase 4 scenario C, the wallet died after lightwalletd
accepted the transaction. The transaction was mined at 19:41:14Z.

**What the wallet did after restart:**
* It re-sent the stored bytes five times between 19:42:14 and 19:44:12.
* Each send was refused with:

  > `lightwalletd rejected transaction (code -1): any transaction with the same effects will be rejected from the mempool until a chain reset: transaction was committed to the best chain`

* The sixth attempt, at 19:44:30, was recorded as accepted.
* The refusal text was recovered from superseded pages in the intent store's
  WAL; see the investigation.

**Why it kept re-sending:**
* **`rebroadcast()`** (`payout-wallet/src/actor.rs`) re-sent every `built`
  intent without looking at the chain. Only `broadcast` intents were checked
  first.
* **`broadcast()`** treats a refusal as an acknowledgement only if its text
  contains `"already"`. This refusal doesn't.

**Impact:**
* No double payment was possible: every send carried the same signed
  transaction, with one txid.
* The intent wrongly stayed `built`, which counts as unfinished, and was
  re-sent on every sync. Had the late "accept" not happened, that would have
  lasted until its expiry height.

## 2. The fix

The chain decides, not the refusal text. Before a stored transaction is sent
again, it is looked up by txid, using the existing `chain::fetch`
(lightwalletd `GetTransaction`).

| Lookup result | `built` intent | `broadcast` intent (unchanged) |
|---|---|---|
| **Mined** | recorded as broadcast (`mark_observed_on_chain`); **not re-sent** | not re-sent |
| **In the mempool** | recorded as broadcast; **not re-sent** | not re-sent |
| **Missing** (definite not-found) | re-sent: the **stored bytes**, same txid | re-sent: the stored bytes |
| **Lookup error** | **nothing sent, nothing recorded**; retried at the next background sync | nothing sent (as before) |

**Unchanged safeguards:**
* intents past their expiry height are skipped, as before (now checked before
  the lookup);
* a malformed stored txid is logged and never sent on a guess;
* the "already" refusal handling in `broadcast()` stays as a secondary
  acknowledgement;
* the test-only `crash_after_broadcast` fault point stays where it was;
* the write-once trigger on `txid`/`raw_tx`;
* nothing in the new path can build a transaction.

**Defensive check:** the real lookup also verifies that the transaction
lightwalletd returns has the requested txid. A different one counts as a
lookup error, so nothing is recorded or sent.

**Where:**
* `payout-wallet/src/resend.rs` (new; 105 lines of code, 196 of tests): `Seen`
  (mined / mempool / missing), the `ChainLookup` trait (the test seam,
  implemented for the real `remote::Client` on top of `chain::fetch`), and
  `step()`, the per-intent decision.
* `payout-wallet/src/actor.rs`: `rebroadcast()` now calls `resend::step` for
  each `built`/`broadcast` intent and sends only on `Step::Send`.
  `broadcast()` is unchanged.
* `payout-wallet/src/intents.rs`: new `IntentStore::mark_observed_on_chain`.
* `payout-wallet/src/lib.rs`: `mod resend;` (private).

**Scope:**
* Only `rebroadcast()` is changed. It is the loop that re-sent C six times.
  It runs at start-up and on every background sync.
* The two one-shot send sites keep their current behaviour: the idempotent
  replay of a `submit` for a `built` intent, and `adopt()`. A refusal there
  leaves the intent `built`, and the next `rebroadcast()` pass then resolves
  it from the chain.

## 3. State-transition semantics

**Why a new method:** `record_broadcast(id, None)` means "the network accepted
*our send*". It increments `broadcast_attempts` and clears
`last_broadcast_error`. Using it for a chain observation would record an
attempt that never happened.

**`IntentStore::mark_observed_on_chain(id)`:**
* **Transition:** `built → broadcast` only, which the existing forward-only
  trigger already allows. It errors (and changes nothing) for `pending`,
  `failed` or `broadcast` intents.
* **`broadcast_attempts`:** **unchanged**, because nothing was sent. The
  counter keeps meaning "sends attempted".
* **`last_broadcast_error`:** **kept**, so the last real refusal (for example
  C's) stays readable. That is exactly what was missing during the OBS-1
  investigation.
* **`updated_at`:** set to now.
* **`txid`, `raw_tx`, `fee_zats`:** untouched (write-once).
* **Effect:** the intent no longer counts in `unfinished_payments`.

## 4. The payroll boundary (unchanged; now pinned by a test)

**What payroll looks at:**
* Payroll never sees the wallet's broadcast result, only its lookup view: the
  intent state, a **fresh** chain status, and verification of the bytes
  served by the chain.
* In `payroll/src/payout.rs::decide`, a **mined** transaction gives
  `Confirmed` only at `required_confirmations` with chain-source
  verification, whether the wallet says `built` or `broadcast`.
* An **unmined** transaction is never `Confirmed`. The intent state only
  selects `Submitted` (broadcast) or `requires_reconciliation` (built).

**What the fix changes:** a mempool sighting can now move an intent to
`broadcast`. Payroll then shows the payout as `submitted`. Its claim stays
reserved and nothing is settled.

## 5. Tests added

**payout-wallet** (`cargo test -j2`; 7 new, 19 in total):

| Test | Asserts |
|---|---|
| `resend::built_and_mined_is_recorded_not_resent` | after 5 refused sends: `Skip`, one lookup by the right txid; intent `broadcast`; attempts still 5; last refusal kept; unfinished 0; same txid/raw/fee; a later pass changes nothing |
| `resend::built_and_in_mempool_is_recorded_not_resent` | `Skip`; `broadcast`, 0 attempts; same transaction |
| `resend::built_and_missing_resends_the_stored_transaction` | `Send` with the stored bytes; intent still `built`, attempts unchanged by the decision; same transaction |
| `resend::lookup_error_sends_nothing_and_changes_nothing` | `Skip`; state, attempts, `updated_at` and error byte-for-byte unchanged; the next pass with a working lookup sends |
| `resend::broadcast_intents_keep_their_behaviour` | `broadcast` intents: `Missing` gives `Send`; mined, mempool or lookup error give `Skip`; nothing recorded |
| `resend::expired_or_unsendable_intents_are_not_looked_up` | expired: `Skip` with no lookup; non-built state: `Skip`; malformed txid: `Skip` with no lookup |
| `intents::observed_on_chain_is_broadcast_without_an_attempt` | `built → broadcast` with no attempt counted; refusal kept; unfinished 0; refused for `pending`, `failed` and already-`broadcast` intents; a later accepted send still counts |

**How the seam works:**
* `ChainLookup` is a crate-private trait with one async method. Tests use a
  `FakeChain` that records lookups and answers mined, mempool, missing or
  error, together with a real `IntentStore` on a temporary SQLite file (the
  existing convention in `intents.rs`).
* No new dependencies: `tokio` and `tempfile` were already present.

**payroll** (`tests/payouts.rs`, 1 new):

`decide_wallet_acknowledgement_never_settles_early`, for both `built` and
`broadcast` wallet states:
* mined at 1–9 confirmations: `Submitted`;
* at 10 with chain verification: `Confirmed`; with local verification:
  `Submitted`;
* mempool only: `Submitted` (broadcast) or `NeedsReconciliation` (built),
  never confirmed or failed.

**Not covered by unit tests:**
* the real `remote::Client` implementation of `ChainLookup` (a thin mapping
  of `chain::fetch` plus the txid check; it needs a lightwalletd);
* `actor.rs::broadcast()` itself (unchanged).

The live fault-injection scenario C (`live_vesting_faults.sh`) exercises both
and wasn't run here, by instruction.

## 6. Regression results

All runs were on 2026-10-19 after the change. The PostgreSQL tests used local
PostgreSQL 17.10 and the disposable `hushush_test` database. Builds used
`-j2`, and `-j1` for wallet-proof.

| Command | Result |
|---|---|
| `payout-wallet: cargo test -j2` | **PASS 19** (12 existing + 7 new) |
| `payout-wallet: cargo clippy --all-targets -- -D warnings`, and the same `--features fault-injection` | clean / clean |
| `payout-wallet: cargo fmt --check` | only the 12 pre-existing drift sites (`actor.rs` 9, `http.rs` 1, `intents.rs` 2; same count as `HEAD`); none in changed code |
| `payroll: cargo test -j2 -- --include-ignored` (SQLite + PostgreSQL) | **PASS 198**, 0 ignored: unit 26, backends 17, engine 29, flow 24, **payouts 42** (+1), vesting 32, vesting_payouts 28 |
| `payroll: cargo fmt --check` | **failed on the first run**: the new test's long `assert!` lines. Fixed with `rustfmt` on `tests/payouts.rs`; only the new test's lines changed. Re-run: clean, and `tests/payouts.rs` re-run: 42/42 |
| `payroll: cargo clippy --all-targets -- -D warnings` | clean (re-run after the formatting fix) |
| `payroll-api: cargo test -j2` / clippy / fmt | **PASS 5** / clean / clean |
| `wallet-proof: cargo test --offline -j1` | **PASS 24**, 3 live tests ignored by design |
| `web: npm run typecheck` | clean |
| `web: npm run lint` | 0 errors; 4 pre-existing warnings, all in `scripts/check-db.mjs` |
| `web: npm test` (real payroll-api and PostgreSQL, mock wallet) | **PASS 84/84** |
| `web: npm run build` | success |

## 7. Remaining risks and next step

* **Liveness when lookups fail persistently.** If `GetTransaction` keeps
  failing (not "not found") while `SendTransaction` would work, a `built`
  transaction that was never broadcast won't be sent.
  * Before the fix it would have been sent blind.
  * That isn't a payment risk: payroll keeps the payout in
    `requires_reconciliation` and the claim reserved. Should the transaction
    expire unsent, payroll's existing expiry rule (margin, absence from the
    mempool, and the wallet scan agreeing) fails it safely.
  * It is a delay, logged as a warning on every sync.
* **Server truthfulness.** A server falsely saying "mined" or "in the mempool"
  would make the wallet stop re-sending.
  * Payroll still doesn't settle without chain-served bytes verified at
    depth, so the payout would sit `submitted` or reconciling, not paid.
  * This trust is the same as the existing `broadcast`-intent check.
  * The branch/network check on lightwalletd (every sync) is unchanged.
* **Binaries are stale.**
  * `payout-wallet/target/debug/hushush-payout-wallet.exe` and
    `hushush-payout-wallet-fault.exe` were built before this change (SHA-256
    prefixes `cd4ba50d…` / `7640b0c4…`).
  * Rebuild them both before any live run: `cargo build -j2 --features
    fault-injection`, copy to `-fault.exe`, then `cargo build -j2`.
* **Recommended next step:** after review, rebuild both binaries and re-run
  live scenario C only (`live_vesting_faults.sh`, or a C-only variant). Expect
  `broadcast_attempts` ≤ 1, a log line "transaction already on the network;
  recorded as broadcast without resending", and the intent `broadcast` on the
  first sync after restart. The wallet now has its own log file, so the
  evidence will survive. That run needs explicit testnet authorization.

# Vesting Phase 4: live fault injection and recovery

Follows [VESTING_PHASE3_TESTNET_VALIDATION.md](VESTING_PHASE3_TESTNET_VALIDATION.md).
Phase 3 proved one normal vesting withdrawal live. This phase drives the
remaining payout failure scenarios for vesting through the **existing**
fault-injection build against the real wallet on the NU7 testnet. Run date:
2026-10-19.

**Outcome:**
* Every scenario that can be executed safely passed live (run `v4run2`):
  * five scenarios;
  * four paid withdrawals and one proven failure;
  * every transaction independently verified on chain.
* The concurrency scenario (F) passed on SQLite and PostgreSQL, including one
  new test for a real coverage gap.
* **No product defect was found.** No product code changed.

## 1. Fault-injection architecture and versions

**Mechanism:**
* `payout-wallet` has a test-only cargo feature, `fault-injection`. With it,
  the wallet actor calls `fault_point(name)` at four real boundaries in its
  payment path.
* When `WALLET_FAULT` matches, the process **exits with code 86** right there:
  `std::process::exit`, no cleanup, nothing simulated in a database.
* Without the feature, `fault_point` compiles to nothing. The normal binary
  doesn't contain the `FAULT INJECTION` string; the fault binary does.

| Fault point (`payout-wallet/src/actor.rs`) | What has happened when the process dies |
|---|---|
| `crash_before_build` (l. 876) | Durable intent `pending` written; no proposal, no signing |
| `crash_after_wallet_store` (l. 889) | Transaction proven, signed and stored in the wallet DB; the intent still `pending` (no txid, no bytes) |
| `crash_after_build` (l. 910) | Intent `built` with txid and raw bytes; never broadcast |
| `crash_after_broadcast` (l. 676) | lightwalletd **accepted** the transaction; the wallet died before recording it (the response is lost) |

**Recovery paths exercised (unchanged code):**
* **Wallet start-up `recover()`:** finds a `pending` intent's transaction by
  memo, or marks it `failed` when none exists.
* **Wallet `rebroadcast()`:** re-sends the *stored bytes* of `built` intents.
* **payroll-api:** `submit_payout` turns any wallet error into
  `requires_reconciliation`. The worker's `reconcile_decision` then waits
  while the wallet is unreachable, and afterwards follows the wallet's record
  plus chain verification.

**Reuse:** the four points are the ones streaming Phase 4 used live
(scenarios C–F in [PHASE4_REPORT.md](PHASE4_REPORT.md) §2.3). Vesting payouts
use the same `payouts` table, state machine, worker and wallet, so they
applied directly. Only the driver is new.

**Binaries and provenance:**
* The existing `hushush-payout-wallet-fault.exe` was **stale**: built
  2026-10-14 10:48, before Phase 6 (`57a924f`, per-company accounts). It was
  kept as `hushush-payout-wallet-fault.stale-20261014.exe` and **not used**.
* Rebuilt from the current tree (payout-wallet and wallet-proof sources are
  unmodified versus `HEAD` `4d86e47`) with
  `cargo build -j2 --features fault-injection`, copied to
  `hushush-payout-wallet-fault.exe`. Then `cargo build -j2` restored the
  normal binary.

| Binary | SHA-256 (prefix) | Built from |
|---|---|---|
| `hushush-payout-wallet.exe` (normal) | `cd4ba50d6836635f` | working tree, 2026-10-19 19:58 |
| `hushush-payout-wallet-fault.exe` | `7640b0c4042bc307` | working tree, `--features fault-injection`, 2026-10-19 19:58 |
| `hushush-payroll-api.exe` | `ac500ccfbaf5171d` | working tree (Vesting Phases 1–3), 2026-10-19 17:37; no source newer |
| web `.next` | build 2026-10-19 17:47 | working tree; no source newer |

**Network guards, checked independently before the run:**
* `wallet-proof/src/lib.rs` pins `NETWORK = TestNetwork` at compile time;
* lightwalletd reported `chain=test`, `branch_id=77190ad9` (the script aborts
  otherwise);
* the recipient (carol's `utest1…` Unified Address) passes the unchanged
  shielded-only `payout_recipient` policy, which requires an Orchard
  receiver; there is no transparent path.

## 2. Live run

**Authorization:** the operator explicitly authorized this run, with at most
five testnet broadcasts: one faucet top-up and four 20,000-zat withdrawals,
with scenario A designed to broadcast nothing.

**Setup (`payout-wallet/scripts/live_vesting_faults.sh`):**
* Isolation:
  * a fresh database per run;
  * temporary wallet, payroll-api and web on ports 7979/7980/3300;
  * none of the operator's services were running or touched.
* Data:
  * one company funded by one faucet top-up (1,000,000 zats);
  * one employee (a member through a worker link);
  * **one agreement per scenario**: 1 ZEC over 1 day, no cliff, activated
    before its start, carol's address locked.
* Payment terms:
  * withdrawal 20,000 zats;
  * fee estimate 10,000; `max_fee` 20,000 (estimate + 10,000; hard cap
    100,000);
  * 10 confirmations.

**Expected transitions and recovery criteria** were coded as assertions
before the run (the script header lists them). The rule throughout: while the
wallet is down, a payout stays `requires_reconciliation` with its claim
`reserved`, and nothing is paid or released.

### Run `v4run1`: aborted at setup, nothing broadcast

* **What happened:** the faucet top-up failed inside the wallet: its pre-build
  sync to lightwalletd timed out (`TimeoutExpired`).
* **Wallet response:** the wallet recorded the payment as `failed` with no
  transaction, and the script stopped.
* **Verification:** 0 transactions carry that payment's memo; the treasury
  was unchanged at 11,277,167 zats; ports were clean.
* **What it shows:** a network failure before construction fails safely.
* **Logs:** `payout-wallet/evidence/live-v4run1*.log`.

### Run `v4run2`: all scenarios PASSED

Times are UTC on 2026-10-19. Evidence:
* [`live-v4run2.log`](../payout-wallet/evidence/live-v4run2.log);
* [`vesting-phase4-evidence/`](vesting-phase4-evidence/): DB dump,
  offline verification, ZecBlock records.

#### A: crash before transaction construction (live)

* **19:24:26** — withdrawal through the fault wallet
  (`crash_before_build`):
  * the wallet exited with **86**;
  * intent `pending`, no txid, no bytes;
  * payroll: `requires_reconciliation` ("submission outcome unknown"),
    claim `reserved`.
* **19:24:35** — **payroll-api hard-killed and restarted** with the wallet
  still down. 45 s later: still `requires_reconciliation`, claim `reserved`,
  paid 0.
  * The worker doesn't release a claim while it can't rule out a late
    submission.
* **19:25:42** — normal wallet restarted. Start-up recovery found no
  transaction for the memo, so the intent became `failed` ("interrupted before
  a transaction was built").
* **19:25:43** — payroll followed the wallet's record:
  * payout `failed`, claim **`released`**;
  * reserved 0, and the 20,000 returned to available.
* **Late-submission probe:** the same payment id was POSTed straight to the
  wallet with the original parameters. The wallet returned the `failed`
  intent; the intent was unchanged; **0 transactions** exist.
* **Retry:** the original idempotency key returned the same `failed` payout
  (200, `duplicate`).
* **History:** `reserved > requires_reconciliation > failed`.

#### D: proven-safe failure release (live; same payout as A)

* **Why it's proven:** the failure here comes from the wallet itself. The
  intent was `pending` with nothing built and no transaction found by memo.
  The intent store only allows `pending → failed` before a build, and
  `built → failed` isn't a legal transition.
* **Released exactly once:**
  * one `failed` event;
  * one claim `released`;
  * the ledger afterwards satisfies vested = paid (0) + reserved (0) +
    available.
* **Ambiguous outcomes didn't take this path:** B1, B2 and C each spent
  minutes in `requires_reconciliation`, with the wallet down and payroll-api
  polling, and **none** produced a `failed` event. Across the run, the only
  `failed` event is A's.

#### B1: signed and stored, process dies before recording (live)

* **19:26:54** — `crash_after_wallet_store`:
  * exit 86;
  * intent `pending` with no txid (the signed transaction exists only in the
    wallet DB);
  * payroll `requires_reconciliation`, unchanged 30 s later.
* **19:27:49** — normal wallet restarted. Recovery found the transaction **by
  memo**, recorded it as built, and broadcast it: `4f290c10…`, 1 attempt. No
  new transaction was built.
* **19:28:26** — mined (4,519,633), then `confirmed` 10/10 and claim
  `settled`.

#### B2: built, never broadcast (live)

* **19:30:48** — `crash_after_build`:
  * exit 86;
  * intent `built`, txid `f18f8d54…`, raw bytes stored, **0 broadcast
    attempts**;
  * lightwalletd: **"transaction not found"**, so the stored transaction
    existed and hadn't been broadcast;
  * payroll `requires_reconciliation`, no replacement built, unchanged 30 s
    later.
* **19:31:34** — normal wallet restarted and re-sent the **stored bytes**
  (same txid, 1 attempt).
* **19:33:34** — mined (4,519,645), then `confirmed` and `settled`.

#### C: broadcast accepted, response lost (live), with E2: cancellation during it

* **19:41:06** — `crash_after_broadcast`:
  * exit 86 after lightwalletd accepted `3fc96b8f…`;
  * intent still `built`, 0 recorded attempts;
  * payroll `requires_reconciliation`.
* **19:41:14** — our keyless observer **sees the transaction on the network**
  (v6, 9,166 bytes, not yet mined) while the wallet is down. The outcome is
  ambiguous from payroll's side, and it was **not** treated as a failure.
* **19:41:47** — **the employer cancels agreement C** while its payout is
  unresolved:
  * `cancelled`, vested frozen at 1,646,990;
  * reserved 20,000; available 1,626,990.
* **Afterwards:** 20 s later the payout was still `requires_reconciliation`
  and the claim `reserved`. Nothing was released or settled early.
* **19:42:16** — normal wallet restarted. Payroll reconciled using the same
  txid and the wallet's record, verified against the chain.
* **Result:**
  * mined at 4,519,666;
  * `confirmed` 10/10, claim **`settled`** once;
  * paid 20,000.
* **After cancellation:** vested stayed at **1,646,990**, 440 s later.

#### E: cancellation while submitted (live)

* **19:43:48** — normal-wallet withdrawal, `submitted` (txid `87110fa2…`).
* **19:43:50** — **employer cancels agreement E**:
  * `cancelled`, vested frozen at 1,799,768;
  * the payout stays `submitted`, the claim `reserved`;
  * paid 0, available 1,779,768.
* A new withdrawal request on E while in flight returned **the same payout**.
* **Result:** mined at 4,519,673, then `confirmed` 10/10 and claim `settled`
  once.
* **After cancellation:** vested unchanged at 1,799,768 (321 s later);
  available 1,779,768 (still owed and withdrawable); unvested 98,200,232
  (forfeited).

### Transaction evidence (NU7 testnet, 10 confirmations required)

| Scenario | Payout | Txid | Height | Amount | Fee (cap) | Events |
|---|---|---|---|---|---|---|
| top-up | faucet `f38da9b2` | `a865055405c25775e2def63869e9b7c3664d682dcbcfeb6f5612799281aeab34` | 4,519,609 | 1,000,000 | 10,000 | — |
| A / D | `bd9ab0b5` | **none** | — | 20,000 requested | — | reserved > requires_reconciliation > failed |
| B1 | `2873eb22` | `4f290c10fcd829e1fee15bff8a9bc903ab5b4ab317c28c12d787ab83ee9ade03` | 4,519,633 | 20,000 | 10,000 (20,000) | reserved > requires_reconciliation > submitted > confirmed |
| B2 | `f6ef362b` | `f18f8d5467c184444619afba4054eb6db18508f83ee2ed8c7b941996c8e31e0a` | 4,519,645 | 20,000 | 10,000 (20,000) | reserved > requires_reconciliation > submitted > confirmed |
| C | `d91782bb` | `3fc96b8f496f948efaf3c7959ddeb64db8903e093e4120d42b7df3d3b962a02d` | 4,519,666 | 20,000 | 10,000 (20,000) | reserved > requires_reconciliation > submitted > confirmed |
| E | `f76cd89b` | `87110fa2172bf461d43e5eacaf75a043ce50e8af4e63783753e6065d1694c0cb` | 4,519,673 | 20,000 | 10,000 (20,000) | reserved > submitted > confirmed |

Explorer: `https://testnet.zecblock.com/tx/<txid>`.

**Independent verification (ZecBlock, the Phase 4 verifier):** ALL PASS for
all 5 transactions:
* confirmed and canonical, tx v6 with the NU7 branch;
* 0 transparent inputs and outputs; 2 Ironwood actions; fee 10,000;
* ZecBlock and the zec.rocks observer agree;
* no plaintext amount, memo, payout id or address in the bytes;
* **the raw bytes on chain are byte-identical (SHA-256) to the bytes the
  wallet persisted**. For B1, B2 and C that is the transaction signed
  *before* the crash, so recovery re-sent the original and never built a
  replacement.

Records: `vesting-phase4-evidence/independent-*.json`.

### Payout uniqueness and accounting after recovery

**Payout uniqueness:**
* **Treasury-wallet transactions per payout memo:** exactly 1 for B1, B2, C
  and E; **0 for A**.
* **Carol (her own wallet, synced):** received each of B1, B2, C and E
  **exactly once**, 20,000 zats, in the expected txid; nothing for A. Total
  2,017,999 → 2,097,999 (Δ 80,000 = 4 × 20,000).

**Account deltas:**
* **Company payout account:** 1,000,000 → 880,000 (Δ 120,000 = 4 × (20,000
  + 10,000)). A cost nothing.
* **Treasury account 0:** 11,277,167 → 10,267,167 (Δ 1,010,000 = top-up +
  its fee). Withdrawals don't touch it.
* **Wallet:** 0 unfinished payments.

**Database (`v4run2-db-final.json`):**
* five payouts, all `compensation_type = vesting`, stream references NULL;
* every settled claim's `settlement_ref` equals its payout's txid;
* `vested_at_cancel_zats` is 1,646,990 for C and 1,799,768 for E;
* 0 stream claims;
* schema v6.

**Final server balances** (`GET /v1/vesting/{id}`, zats):

| Agreement | Status | Total | Vested | Unvested | Paid | Reserved | Available |
|---|---|---|---|---|---|---|---|
| A | active | 100,000,000 | 2,148,148 | 97,851,852 | 0 | 0 | 2,148,148 |
| B1 | active | 100,000,000 | 2,150,462 | 97,849,538 | 20,000 | 0 | 2,130,462 |
| B2 | active | 100,000,000 | 2,153,935 | 97,846,065 | 20,000 | 0 | 2,133,935 |
| C | **cancelled** | 100,000,000 | 1,646,990 (= at cancel) | 98,353,010 | 20,000 | 0 | 1,626,990 |
| E | **cancelled** | 100,000,000 | 1,799,768 (= at cancel) | 98,200,232 | 20,000 | 0 | 1,779,768 |

* **Ledger rules held for every row:**
  * vested = paid + reserved + available;
  * total = vested + unvested;
  * no reservation is left over.
* **Cancellation:** the cancelled agreements froze at their cancel values,
  while A, B1 and B2 (not cancelled) kept vesting.

### What could not be executed live, and why

| Item | Why not live | Evidence instead |
|---|---|---|
| payroll-api crash **between** reservation commit and the wallet call | There's no fault point in payroll-api. The route also quotes through the wallet *before* reserving, so the wallet can't simply be down at that moment. Killing payroll-api by timing (streaming Phase 3 B) lets the wallet finish and pay, which is a different scenario. | Mock: `crash_after_reservation_before_build` (lease expiry → wallet fence → failed → released; a late submit builds nothing), both backends. The live wallet-side boundary is scenario A. |
| Crash *inside* the settlement transaction | It can't be timed in a live process. | Mock: `crash_after_confirmation_before_settlement`, both backends. |
| Concurrent real withdrawals (F) | Deliberately not raced against a real wallet, to avoid any risk of duplicate broadcasts. | Automated tests (§3). |

## 3. F: concurrent withdrawal requests (automated, SQLite + PostgreSQL)

**Existing coverage:**
* `vesting.rs::concurrent_claims`: 16 threads at the reservation layer, mixing
  a shared key with distinct keys and over-available demand; exactly 6
  reserved, the rest refused, ledger checked.
* `vesting_payouts.rs::withdraw_idempotent_and_concurrent`: same key, a
  conflicting amount on the same key, another key while in flight, and 8
  concurrent distinct-key `withdraw_vesting` calls → 1 payout, 1 transaction.
* `concurrent_claims_and_cancel`.

**Gap found:** nothing raced the **payout-creating** path (`withdraw_vesting`)
with a shared key, distinct keys and **amounts above the available balance**
together.

**Added:** `vesting_payouts.rs::withdraw_concurrent_mixed_and_overdraw`.
* **Load:** 12 threads behind a barrier, in three groups:
  * 4 sharing one key;
  * 4 distinct keys that fit;
  * 4 distinct keys asking for available + 1.
* **Asserts:**
  * exactly one payout created;
  * every other result is that same payout or `InsufficientAvailable`;
  * one vesting claim;
  * reserved = the created amount, and available reduced by exactly that;
  * vested = paid + reserved + available;
  * after submit and confirm: paid = amount, reserved 0, **1 transaction
    built**.
* **Result:** passed 5 times in a row on each backend (10/10) and in the full
  suite.

No existing test was changed or weakened.

## 4. Regression results

All runs were after the code changes, on 2026-10-19. The PostgreSQL tests
used local PostgreSQL 17.10 and the disposable `hushush_test` database.
Builds used `-j2` (`-j1` for wallet-proof), the constraint established in
Phase 3.

| Command | Result |
|---|---|
| `payroll: cargo test -j2 -- --include-ignored` (SQLite + PostgreSQL) | **PASS 197**, 0 ignored: unit 26, backends 17, engine 29, flow 24, payouts 41, vesting 32, **vesting_payouts 28** (was 26: +1 scenario × 2 backends); covers recovery, cancellation, concurrency and both migration-upgrade tests |
| `payroll: cargo fmt --check`; `cargo clippy --all-targets -- -D warnings` | clean |
| `payroll-api: cargo test -j2` / clippy `-D warnings` / `fmt --check` | **PASS 5** / clean / clean |
| `payout-wallet: cargo test -j2` | **PASS 12** |
| `payout-wallet: cargo clippy --features fault-injection -- -D warnings` | clean |
| `wallet-proof: cargo test --offline -j1` | **PASS 24**, 3 live ignored by design |
| `web: npm run typecheck` | clean |
| `web: npm run lint` | 0 errors; 4 warnings, all in `scripts/check-db.mjs` (pre-existing); the changed `scripts/live-vesting.mjs` is clean |
| `web: npm test` (real payroll-api and PostgreSQL, mock wallet) | **PASS 84/84** |
| `web: npm run build` | success |
| `payout-wallet`, `wallet-proof: cargo fmt --check` | pre-existing drift in files unchanged since `HEAD` (as in Phases 2–3); not reformatted |
| Live `v4run2` + offline verification + ZecBlock | **PASS** (§2) |

## 5. Defects, observations and fixes

**Product defects: none.** No product code was changed. Every recovery path
behaved as designed, and exactly-once held.

**Observations:**
1. **OBS-1 (low, open): C needed 6 broadcast attempts.**
   * After the C restart, the wallet's re-sends of the stored bytes were
     refused several times before one was recorded as accepted (6 attempts
     in all, ending at 19:44:30).
   * The node's refusal text was lost (see OBS-2).
   * Probable cause: lightwalletd's reply for an already-known or
     already-mined transaction doesn't contain the word `already`, which is
     the only refusal the wallet treats as an acknowledgement.
   * **No safety impact:**
     * every attempt was the same signed transaction (one txid);
     * payroll settled from the chain-verified wallet record, not from the
       broadcast result;
     * the intent ended `broadcast`.
   * **Next step:** capture the refusal text in a wallet-only log and, if
     confirmed, recognise it as an acknowledgement. This is a wallet change,
     not made in this phase.
2. **OBS-2 (tooling, fixed): the combined services log lost lines.**
   * Processes appending to one file under Git Bash on Windows overwrite
     each other's lines: the wallet's lines from 19:24 to 19:44 are mostly
     missing.
   * The primary evidence (run log, exit codes, intent rows, database,
     chain) doesn't depend on that file.
   * The script now gives the wallet its own log, `live-<run>.wallet.log`.
3. **OBS-3 (tooling, fixed): the script's final check crashed.**
   * Its embedded Python got a Git Bash path as `cwd` and crashed after all
     scenarios had completed.
   * Those checks were then run offline against the run's evidence (logged
     server snapshots, its database, the wallets, and the wallet service
     started alone, read-only):
     [`v4run2-offline-verify.txt`](vesting-phase4-evidence/v4run2-offline-verify.txt).
     All passed.
   * The script now passes a Windows path. It was **not** re-run live, to
     avoid needless broadcasts.
4. **v4run1:** a transient lightwalletd timeout failed the faucet top-up
   safely (above).

**Changes made in this phase (none to product code):**
* `payroll/tests/vesting_payouts.rs`: one new scenario (§3);
* `payout-wallet/scripts/live_vesting_faults.sh`: new live driver;
* `web/scripts/live-vesting.mjs`: `create`, `cancel` and `AGREEMENT=<label>`
  added;
* the fault binary rebuilt; the stale one kept, renamed.

## 6. Remaining gaps

* **Mock-only items:** a payroll-api crash between reservation commit and the
  wallet call, a crash inside settlement, and concurrent live requests (see
  §2).
* **OBS-1:** confirm the node's refusal text and decide whether the wallet
  should treat it as an acknowledgement.
* **Live coverage not yet re-run on vesting:**
  * loss of the wallet's intent store (streaming Phase 4 G);
  * two concurrent payroll-api instances (streaming A).

  Both use the same code paths as streaming, which passed live in Phase 4.
* **Throughput:** one faucet note per company per 24 h. Change must confirm
  before the next scenario can spend, so a full run takes about 35 minutes.
* **Still uncommitted:** Vesting Phases 1–4.

**Repeating the run:**
1. Build `wallet-proof` with `-j1`.
2. Build `payout-wallet` with `-j2 --features fault-injection`, copy the
   result to `-fault.exe`, then build again normally.
3. Build `payroll-api` with `-j2`.
4. Run `npm run build` in `web`.
5. Run `LIVE_AUTHORIZED=testnet RUN_ID=<id> bash payout-wallet/scripts/live_vesting_faults.sh`.

It needs `treasury` funded (about 1.05 M zats per run), `carol`, and free
ports 3300/7979/7980.

## 7. Recommendation

* **Further testnet testing: GO.**
  * Each live failure boundary of the vesting payout path was exercised with
    the real wallet: before build, after signing, after build, after an
    accepted broadcast, and cancellation during ambiguity and during
    submission.
  * Each one recovered through the established path. Nothing was paid twice,
    and the one proven failure released its claim exactly once.
  * The accounting rules held throughout, and all suites pass on both
    database backends.
  * Suggested next steps: resolve OBS-1, and optionally repeat streaming
    scenarios G and A (intent-store loss, two instances) on vesting.
* **Production / mainnet:** out of scope for this phase. The Phase 3 NO-GO
  stands for the same reasons: testnet-only network, single-host key custody,
  no own node, a shared demo treasury and faucet, uncommitted and unreviewed
  vesting code.

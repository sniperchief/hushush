# Vesting Phase 3: wallet verification and live testnet validation

Follows [VESTING_PHASE2_PAYOUTS.md](VESTING_PHASE2_PAYOUTS.md). This phase made
no product changes. It diagnosed the `wallet-proof` test failure, re-ran every
regression suite, and made **one real shielded vesting withdrawal on the Zcash
testnet**, independently verified. Run date: 2026-10-19.

**Outcome:** all suites pass. The `wallet-proof` failure was memory
exhaustion, not corruption, and is resolved without source changes. Live run
`v3run1` **PASSED**.

## 1. Environment

| Item | Value |
|---|---|
| OS | Windows 10 Pro 10.0.19045, 4 logical CPUs, **8 GB RAM** |
| Page file | system-managed `C:\pagefile.sys`, 24 GB allocated; commit limit 32.6 GB, **4.3 GB free** when checked |
| Disk | C: 238 GB, ~22 GB free (91 % used) |
| Rust | rustc 1.99.0 (b940084d7 2026-09-28), cargo 1.99.0, `stable-x86_64-pc-windows-msvc` |
| Node / Next.js | Node v24.18.0, Next.js 16.4.0 |
| PostgreSQL | 17.10, local, port 54329 (the only service running at the start) |
| Zcash network | testnet, `testnet.zec.rocks:443` lightwalletd v0.5.4, `chain=test`, NU7 branch `77190ad9` |
| Code | `main` at `4d86e47` plus the uncommitted Vesting Phase 1+2 working tree (unchanged by this phase; see §9) |

The working tree was inspected first. Nothing was reset, stashed, discarded or
committed.

## 2. The `wallet-proof` failure (`E0786`)

**Symptom (Phase 2, reproduced here):** `cargo test` in `wallet-proof` failed
with:
* `E0786 found invalid metadata files for crate hushush_wallet_proof`;
* `E0463 can't find crate for age`.

**Diagnosis.** The full diagnostic shows the real cause underneath `E0786`:

```
failed to mmap file '...\libhushush_wallet_proof-518e6c1d6e94f96b.rlib':
The paging file is too small for this operation to complete. (os error 1455)
```

* Windows error 1455 means the system **commit limit** is exhausted. The rlib
  isn't corrupt; rustc couldn't map it into memory.
* The machine has 8 GB of RAM and only ~4.3 GB of commit free. Other
  user processes held several GB: a Python process ~2.4 GB, node ~1.2 GB, a
  screen recorder ~1.1 GB.
* `wallet-proof` builds every dependency at `opt-level = 3`, and cargo runs 4
  rustc jobs at once. Together they pushed commit past the limit.
* That also explains why Phase 2's attempts all failed the same way: its own
  `target/`, after `cargo clean`, and a fresh directory.

Raw log: [`vesting-phase3-evidence/wallet-proof-e0786-before.log`](vesting-phase3-evidence/wallet-proof-e0786-before.log).

**What this rules in and out:**

| Hypothesis | Verdict |
|---|---|
| Out of commit memory (page file too small for parallel rustc) | **Confirmed**: os error 1455; fixed by lower build parallelism alone |
| Corrupted build artifacts | Ruled out: the same `target/` built and passed with `-j1`; a clean isolated build also passed (below) |
| Disk or filesystem fault | Not indicated: no I/O errors; 22 GB free |
| Toolchain problem | Ruled out: the same rustc built the crate and payout-wallet (which depends on it) |
| Source / compiler incompatibility | Ruled out: no source change was needed |

**Fix (no source change):** build with fewer parallel jobs:
`cargo test -j1` here, `-j2` for the other crates (or `CARGO_BUILD_JOBS=2`).

**Results:**
* `-j1`, existing `target/`: **24/24 offline tests pass** (12 unit, 6
  `mocked_chain`, 6 `recovery`). The 3 `testnet_live` tests are `#[ignore]`d
  by design.
* This also rebuilt the CLI binary, which Phase 2's `cargo clean` had
  removed.
* Log: [`wallet-proof-j1-pass.log`](vesting-phase3-evidence/wallet-proof-j1-pass.log).
* Clean, isolated build (new `CARGO_TARGET_DIR` in a scratch directory,
  `-j2`, every dependency rebuilt from scratch): **24/24 pass**, 3 live
  ignored.
  * 405 crates compiled in about 50 minutes.
  * Log: [`wallet-proof-clean-isolated-pass.log`](vesting-phase3-evidence/wallet-proof-clean-isolated-pass.log).
  * The scratch target directory was deleted afterwards.

## 3. Commands and results

All runs were on 2026-10-19. The PostgreSQL tests used
`PAYROLL_TEST_DATABASE_URL=<local>/hushush_test`, and the web tests used
`TEST_DATABASE_URL=<local>/hushush_test` (a disposable test database).

| Command | Result |
|---|---|
| `wallet-proof: cargo test --offline -j1` | **PASS** 24 passed, 3 live ignored |
| `wallet-proof: CARGO_TARGET_DIR=<scratch> cargo test --offline -j2` (clean) | **PASS** 24 passed, 3 live ignored |
| `payroll: cargo test -j2 -- --include-ignored` (SQLite **and** PostgreSQL) | **PASS 195** (unit 26, backends 17, engine 29, flow 24, payouts 41, vesting 32, vesting_payouts 26), 0 ignored |
| `payroll-api: cargo test -j2` | **PASS** 5 |
| `payout-wallet: cargo test -j2` | **PASS** 12 |
| `payroll`, `payroll-api: cargo clippy --all-targets -- -D warnings` | clean |
| `payroll`, `payroll-api: cargo fmt --check` | clean |
| `payout-wallet`, `wallet-proof: cargo fmt --check` | drift (pre-existing; both crates are unmodified in git; not reformatted) |
| `web: npm run typecheck` | clean |
| `web: npm run lint` | 0 errors, 4 warnings, all in `scripts/check-db.mjs` (pre-existing) |
| `web: npm test` (real payroll-api and PostgreSQL, mock wallet) | **PASS** 84/84, 7 files |
| `web: npm run build` | success; all `/api/vesting/*` and `/app/vesting/*` routes built |
| `payroll-api: cargo build -j2` | rebuilt (the binary in `target/` predated the vesting code) |
| `payout-wallet: cargo build -j2` | already up to date |
| `LIVE_AUTHORIZED=testnet RUN_ID=v3run1 bash payout-wallet/scripts/live_vesting.sh` | **PASS** (§5) |
| Independent ZecBlock verification of the run's 2 transactions | **ALL PASS** (§6) |

## 4. Database migration and upgrade

* **Automated upgrade tests** (`upgrade_keeps_stream_payouts`, SQLite v4→v5 and
  PostgreSQL v5→v6) pass. They start from a populated previous version (a
  confirmed stream payout and an in-flight one) and check that, after the
  upgrade:
  * history and foreign keys are intact;
  * the guards still fire;
  * the in-flight payout settles exactly once;
  * a vesting withdrawal then works on the same database.
* **Live run:** payroll-api migrated a fresh database `hushush_live_v3run1` to
  schema v6 on start (`0001_init` … `0005_vesting`, `0006_vesting_payouts`).
  The web migrations applied up to `0005_open_accounts.sql`.
* **Isolation:** migrations went only to that new database and to the
  disposable `hushush_test`. The user's dev and try databases were not
  touched.

## 5. Live testnet run `v3run1`

**Authorization:** the operator explicitly authorized exactly two testnet
broadcasts in this session (the faucet top-up and one withdrawal).

**Pre-flight checks (all held):**
* **Network:**
  * `NETWORK` is the compile-time `TestNetwork`, and `MainNetwork` appears
    only in tests that check mainnet addresses are refused;
  * lightwalletd reported `chain=test` and branch `77190ad9`, and the script
    aborts otherwise.
* **Funds:** treasury 12,287,167 zats.
* **Destination:**
  * carol's `utest1…` Unified Address, accepted by the wallet's
    `payout_recipient` policy (UA with an Orchard receiver; Sapling-only,
    transparent, TEX, mainnet and garbage are refused; no transparent
    fallback exists);
  * the real wallet refused an invalid address live with 422.
* **Binaries:** payroll-api freshly built from the working tree;
  payout-wallet verified current; web rebuilt.
* **Isolation:** a fresh database and temporary services on free ports
  3300/7980/7979. The user's services weren't running and weren't touched.
* **Terms decided before broadcasting:**
  * withdrawal of 30,000 zats;
  * fee cap = estimate + 10,000, wallet hard cap 100,000;
  * 10 confirmations;
  * expected end state: payout `confirmed`, claim `settled` with
    `settlement_ref` = txid, `paid = 30,000`, `reserved = 0`.

**Flow (real web routes, real cookies, software passkeys)**

All times are UTC on 2026-10-19.

1. **Setup (16:49:58):**
   * employer signs up and creates a company;
   * the testnet faucet moves 1,000,000 zats from treasury account 0 to the
     company's own account (`f9110c84…`);
   * the employee joins through a worker link, which is what makes them a
     member;
   * the employer creates a linear agreement: **0.05 ZEC over 1 day, no
     cliff**, starting 90 s later, id `85df3a12…`;
   * the employer activates it before the start (`active`, `not_started`).
2. **Address and pre-start checks:**
   * invalid address → **422** `invalid_recipient`;
   * carol's address → locked;
   * a change → **409**;
   * a withdrawal before the start → **409** `insufficient_available` (0
     available), with no payout created.
3. **Vesting and funding (17:00:12):**
   * the server reports `vested 30,324`, `available 30,324`;
   * company account spendable 1,000,000;
   * quote: fee **10,000**, `valid_recipient`, `sufficient`.
4. **Withdrawal (17:00:17–17:00:58):**
   * `POST /api/vesting/{id}/withdraw` with 0.0003 ZEC → **201**, payout
     `e43b1e7a…`, `compensation_type = vesting`, `submitted`, txid
     `8571ed19…`, `max_fee` 20,000;
   * the same key again → **200**, `duplicate: true`, the same payout;
   * a new key while in flight → **200**, the same payout;
   * no other transaction was built.
5. **Restart (17:01:10–17:01:11):** the run's own payroll-api was stopped
   while the payout was `submitted` and started again. Every later
   `payout reconciled` log line comes from the new process.
6. **Confirmation (17:05:43):**
   * mined at height **4,519,265**;
   * confirmed at **10/10**;
   * the claim was settled by the worker, through the normal reconcile path.

**Transaction evidence**

| | |
|---|---|
| Network | Zcash **testnet** (NU7 chain, branch `0x77190AD9`) |
| Withdrawal txid | `8571ed19b8ea556b59a7349a8655666fd1d46e30e74258a780b5c70e2f19be93` |
| Faucet top-up txid | `f9110c845d4309b2dd9dc5ab9cf22cc12e34f1cf663e6e4089e4aed576e424db` (height 4,519,236) |
| Requested amount | 30,000 zats (0.0003 TAZ) |
| Fee | estimate 10,000; **actual 10,000**; cap 20,000 |
| Mined height | 4,519,265 |
| Confirmations | app 10/10 at settlement; carol's wallet via lightwalletd: 10 (tip 4,519,274); ZecBlock: 13 when checked |
| Timestamps (UTC) | reserved 17:00:19, submitted 17:00:58, confirmed/settled 17:05:34 |
| Explorer | https://testnet.zecblock.com/tx/8571ed19b8ea556b59a7349a8655666fd1d46e30e74258a780b5c70e2f19be93 |

**Before and after (server-side accounting, `GET /v1/vesting/{id}` as the employee), in zats:**

| | total | vested | unvested | reserved | paid | available |
|---|---|---|---|---|---|---|
| Before withdrawal (17:00:17) | 5,000,000 | 30,613 | 4,969,387 | 0 | 0 | 30,613 |
| In flight (17:01:05) | 5,000,000 | 33,391 | 4,966,609 | 30,000 | 0 | 3,391 |
| After confirmation (17:05:44) | 5,000,000 | 49,537 | 4,950,463 | 0 | **30,000** | 19,537 |

* **Invariants held at every snapshot:**
  * vested = paid + reserved + available;
  * total = vested + unvested.
* **Vesting continued** during the run (≈ 57.9 zats/s), so "available" grows
  after the payment.

**Final database rows** ([`v3run1-db-final.json`](vesting-phase3-evidence/v3run1-db-final.json)):
* **one payout:**
  * `compensation_type = vesting`, `state = confirmed`;
  * `vesting_claim_id` and `vesting_agreement_id` set, `claim_id` and
    `stream_id` NULL;
  * `fee_zats` 10,000, `max_fee_zats` 20,000;
* **events:** exactly `reserved → submitted → confirmed`;
* **one vesting claim:** `settled`, `settlement_ref` = the txid;
* **zero stream claims** (the membership stream was never touched).

**Wallet-side checks:**
* **Carol:**
  * received **exactly once**: 30,000 zats in `8571ed19…`, with memo
    `hushush payout e43b1e7a…`;
  * balance 1,987,999 → 2,017,999 (Δ 30,000).
* **Treasury wallet:** exactly **1** transaction carries the payout's memo.
* **Company account:** 1,000,000 → 960,000 (Δ 40,000 = withdrawal + fee).
* **Treasury account 0:** 12,287,167 → 11,277,167 (Δ 1,010,000 = top-up +
  its fee; withdrawals don't touch it).

**Pages:**
* the employee page shows "Paid ✓", the txid and their own address;
* the employer page shows "Paid ✓" and the txid, and never the address.

**Logs:**
* [`payout-wallet/evidence/live-v3run1.log`](../payout-wallet/evidence/live-v3run1.log);
* services: `live-v3run1.services.log`, checked for secrets: no seed,
  mnemonic, key or address key appears.

## 6. Independent verification (ZecBlock)

**Method:** the Phase 4 verifier
([`phase4-evidence/independent_verify.py`](phase4-evidence/independent_verify.py)),
restricted to this run's two transactions. It compares:
* ZecBlock (`api.testnet.zecblock.com`, a separately operated node and
  indexer);
* our keyless observer on zec.rocks;
* the bytes the payout wallet persisted before broadcast.

**Records:** `vesting-phase3-evidence/independent-*.json`.

| Check | Withdrawal `8571ed19…` | Top-up `f9110c84…` |
|---|---|---|
| Confirmed and canonical on ZecBlock, txid matches | yes | yes |
| Tx v6, NU7 branch (parsed from raw bytes) | yes | yes |
| Transparent inputs / outputs | 0 / 0 | 0 / 0 |
| Shielded shape | 2 Ironwood actions, 0 Orchard, 0 Sapling | same |
| Fee (public) | 10,000 | 10,000 |
| Height, size, fee and actions agree (ZecBlock vs zec.rocks observer) | yes | yes |
| ZecBlock raw bytes == wallet's pre-broadcast bytes (SHA-256) | **identical** | **identical** |
| Plaintext amount, memo, payout id or recipient UA in the bytes | none | none |

**Result: ALL PASS.**

**Note on pools:** on the NU7 testnet the wallet builds Orchard-family
**Ironwood** actions (as in every live run since Phase 4).
* The Orchard-receiver-only recipient policy is unchanged.
* The single-pool proposal rule is unchanged.
* There is no transparent path.

## 7. Recovery and idempotency evidence

| Property | Live testnet evidence (`v3run1`) | Mock-wallet evidence (automated, SQLite + PostgreSQL) |
|---|---|---|
| Same idempotency key returns the original payout | **yes**: 200 `duplicate`, same id, no new tx | `withdraw_idempotent_and_concurrent`, `crash_after_*` |
| Another request while one is in flight returns it (one open withdrawal) | **yes**: new key → same payout | same, plus 8 concurrent threads → 1 payout, 1 tx; web: concurrent and duplicate withdraw |
| Restarted worker continues the existing payout, no new one | **yes**: payroll-api restarted while `submitted`; the new process confirmed and settled it; 1 tx carries the memo | `crash_after_confirmation_before_settlement` (new connection and lease settles once) |
| Claim can't be released or settled independently while its payout is attached | not exercised live (needs direct engine/SQL calls) | `attached_claim_paths` (engine and SQL trigger) |
| Confirmed payout settles the claim exactly once | **yes**: one `confirmed` event; claim `settled` once, `settlement_ref` = txid; paid 30,000 | `withdraw_submit_confirm` (repeat reconcile is a no-op) |
| Proven failure releases only through the failure path | not exercised live (would need a deliberate failure) | `crash_after_reservation_before_build` (fence → failed → released; late submit builds nothing), `crash_after_broadcast` (lost request, wallet refusal) |
| Crash before reservation commit; after signing; after broadcast | not exercised live (by design: no deliberate disruption of a real tx) | `crash_before_reservation_commits`, `crash_after_signed_tx_stored`, `crash_after_broadcast` |
| Cancellation doesn't invalidate a payout in flight | not exercised live | `cancellation_with_payouts`; web: cancellation with an in-flight withdrawal |
| Accounting invariants | **yes**: checked on every snapshot (§5) | `check_ledger` across `tests/vesting.rs` |
| Stream and vesting claims stay apart | **yes**: vesting payout carries only the vesting references; 0 stream claims touched | `compensation_types_stay_apart`, `payout_reference_integrity` |

**No gap required a new test.** The Phase 2 suites already cover each item
asked for, on both backends, and their assertions were left unchanged.

## 8. Test tooling added (not product code)

* [`payout-wallet/scripts/live_vesting.sh`](../payout-wallet/scripts/live_vesting.sh):
  * the live run above;
  * refuses to run without `LIVE_AUTHORIZED=testnet`;
  * aborts unless lightwalletd reports the NU7 testnet;
  * refuses busy ports;
  * uses a new database per run.
* [`web/scripts/live-vesting.mjs`](../web/scripts/live-vesting.mjs): its
  driver.
  * Mutations go through the web route handlers.
  * Reads come from payroll-api as the employee.
* Evidence: [`docs/vesting-phase3-evidence/`](vesting-phase3-evidence/) and
  `payout-wallet/evidence/live-v3run1*.log`.

## 9. Remaining gaps

1. **Live coverage is the happy path plus a restart.**
   * Crash recovery at the build, sign and broadcast points, cancellation
     during an in-flight payout, and failure release are proven only against
     the mock wallet for vesting.
   * The same machinery was proven live for streams in Phase 4 (scenarios
     A–G).
   * To cover them live, reuse the Phase 4 fault-injection build
     (`hushush-payout-wallet-fault.exe`, `fault-injection` feature) with a
     vesting agreement.
2. **Concurrent taps were not run live** (deliberately: one broadcast only).
   They are covered by automated tests and by Phase 5 live for streams.
3. **Build machine memory.**
   * On this machine, librustzcash crates must be built with `-j1`/`-j2`
     (or `CARGO_BUILD_JOBS=2`), or with fewer memory-heavy applications open.
   * To reproduce the failure: run `cargo test` (default jobs = 4) in
     `wallet-proof` with less than ~5 GB of commit free.
4. **Pre-existing `cargo fmt` drift** in `payout-wallet` and `wallet-proof`.
5. **Phase 2 gaps still stand:**
   * vesting withdrawals don't appear on the employer's "Payments" card;
   * no vesting-only invitation;
   * no four-eyes control;
   * the fee quote isn't the final fee (bounded by `max_fee`).
6. **Still uncommitted:** Vesting Phases 1–3. The live binaries were built
   from that working tree.

**Repeating the live run:**
* stop anything on 3300/7980/7979;
* build `wallet-proof` (`-j1`), `payout-wallet` and `payroll-api` (`-j2`);
* run `npm run build` in `web`;
* then run `LIVE_AUTHORIZED=testnet RUN_ID=<id> bash payout-wallet/scripts/live_vesting.sh`.

It needs a funded `treasury` (about 1.05 M zats per run) and `carol`. It takes
about 20 minutes.

## 10. Recommendation

* **Further testnet testing: GO.**
  * The vesting withdrawal path worked end to end against the real wallet on
    testnet.
  * It paid exactly once, settled exactly once after 10 confirmations,
    survived a payroll-api restart, and was independently verified on chain.
  * All regression suites pass on both database backends.
  * Next useful step: live fault-injection scenarios for vesting (gap 1).
* **Production / mainnet: NO-GO.** Nothing here changes the Phase 4–6
  position:
  * the network is hard-coded to testnet;
  * key custody is a seed on one host (no KMS/PCZT signing);
  * no own Zebra/lightwalletd;
  * a shared demo treasury and faucet;
  * no four-eyes control on vesting;
  * Vesting Phases 1–3 are uncommitted and unreviewed.

  See [MAINNET_OPTION_B_SELF_CUSTODY.md](MAINNET_OPTION_B_SELF_CUSTODY.md).

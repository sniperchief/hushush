# OBS-1: live validation of scenario C (`obs1c1`)

**Context:** validates the wallet fix in
[OBS1_WALLET_RECOVERY_FIX.md](OBS1_WALLET_RECOVERY_FIX.md) against real
lightwalletd on the Zcash NU7 testnet. It reuses the fault point from Vesting
Phase 4 scenario C
([VESTING_PHASE4_FAULT_INJECTION.md](VESTING_PHASE4_FAULT_INJECTION.md)).
Run on 2026-10-20 by the operator, after a no-broadcast preflight and an
explicit authorization of a plan with one broadcast and no faucet top-up.

**Result: PASS.**
* The wallet restarted after the network had already **mined** its
  transaction.
* It found the transaction by its original txid, recorded the intent as
  broadcast **without a send** (attempts 0 → 0), and never sent it again.
* Payroll settled the claim once, only at 10 confirmations with chain
  verification.
* Exactly one transaction was broadcast. There was no faucet top-up.

## 1. Preflight (no broadcasts)

| Check | Result |
|---|---|
| Fix present in the working tree | `payout-wallet/src/resend.rs` (new); `actor.rs:659` `rebroadcast()` calls `resend::step`; `intents.rs:469` `mark_observed_on_chain` |
| Wallet binaries rebuilt from that source | the binaries built before the fix (19:58, before the source edits) were kept as `*.prefix-obs1.exe`. Rebuilt 2026-10-20: `cargo build -j2 --features fault-injection`, copied to `-fault.exe`, then `cargo build -j2` |
| Normal binary `hushush-payout-wallet.exe` | SHA-256 `a4e7ae7284ce9102c4f32b5e0a325dffdf2eb54d85449f033b71d98ba0e72a24`; OBS-1 code present, fault-injection code absent |
| Fault binary `hushush-payout-wallet-fault.exe` | SHA-256 `bf5dc27f800f3364d5c10970be5c5b0a26dd9cef21c8f9fdee410be649c4d3b0`; OBS-1 code and fault-injection code both present |
| payroll-api binary | `ac500ccfbaf5171d…`, unchanged since the Phase 4 runs (no payroll source changed for OBS-1) |
| Network | compile-time `TestNetwork`; lightwalletd `testnet.zec.rocks:443` reported `chain=test`, branch `77190ad9` (the script aborts otherwise); no endpoint override in `dev.env` |
| Running services | no hushush processes; only the operator's PostgreSQL on :54329. The script refuses to start if 3300, 7980 or 7979 is busy |
| Fault point can't hit another transaction | treasury intent store: 17 `broadcast`, 2 `failed`, 0 unfinished; the script refuses to start the fault wallet otherwise |
| Funding | no faucet needed: the run reuses the `v4run2` company's payout account (880,000 zats) via a **copy** of that run's database; the v4run2 sessions are valid until 2026-11-18 |

**Harness:** the Phase 4 script runs every scenario, so a C-only harness was
written: [`payout-wallet/scripts/live_obs1_c.sh`](../payout-wallet/scripts/live_obs1_c.sh).
* **Guard:** it requires `LIVE_AUTHORIZED=testnet`.
* **Logs:** one log file per process.
* **Timing:** it waits until the transaction is **mined** before restarting
  the wallet, which is the exact OBS-1 condition.
* **Stops (evidence kept) on any of:**
  * the transaction not mined within 15 minutes; the wallet is then not
    restarted, so there is no blind resend;
  * the intent not reaching `broadcast` with 0 attempts;
  * the observation log line missing;
  * any send, failed lookup or mismatched transaction logged for the payment.

**Command (run by the operator):**

```
LIVE_AUTHORIZED=testnet RUN_ID=obs1c1 bash payout-wallet/scripts/live_obs1_c.sh
```

## 2. What happened

Times are UTC on 2026-10-20, from
[`live-obs1c1.log`](../payout-wallet/evidence/live-obs1c1.log) and the
per-process logs.

| Time | Event |
|---|---|
| 10:17:28 | `hushush_live_obs1c1` created as a copy of `hushush_live_v4run2` (template untouched: still 5 payouts afterwards) |
| 10:20:38 | Before: treasury 10,267,167; company account `8d29e7dd…` 880,000; carol 2,097,999; wallet unfinished payments 0 |
| 10:20:41 | Agreement C2 (`ab4ab0aa…`, 1 ZEC over 1 day) created and activated; carol's address locked |
| 10:22:32 | Quote: 20,000 zats, fee 10,000, sufficient |
| 10:22:43 | Fault wallet started (`WALLET_FAULT=crash_after_broadcast`) |
| 10:23:31 | Wallet log: `transaction built and recorded … txid=100b164f… fee_zats=10000` |
| 10:23:32 | Wallet log: `FAULT INJECTION: exiting at 'crash_after_broadcast'`; exit code 86. payroll-api: `submission recorded … state="requires_reconciliation" txid=None` |
| 10:23:35 | Intent **before** recovery: `built`, txid `100b164f…`, **broadcast_attempts 0**, raw bytes stored (9,166 bytes) |
| 10:23:44 | Block 4,521,791 containing the transaction (ZecBlock `blockTime` 1792491824) |
| 10:23:57 | Keyless observer (wallet down): `mined_height 4521791`, v6, NU7 |
| 10:23:58 | Payroll while the wallet is down: `requires_reconciliation`, claim `reserved`, paid 0 |
| 10:23:59 | Fixed normal wallet starting; start-up recovery runs before it listens |
| **10:24:06** | Wallet log: `INFO hushush_payout_wallet::resend: transaction already on the network; recorded as broadcast without resending payment_id=0341c5b2-… txid=Some("100b164f…") seen=Mined(BlockHeight(4521791))` |
| 10:24:10 | Intent **after** recovery: `broadcast`, **broadcast_attempts 0**, `last_broadcast_error` null, same txid and fee |
| 10:24:16 | payroll-api: `submitted`, 2 confirmations |
| 10:25:23 | 70 s (three background syncs) later: intent unchanged, attempts 0 |
| 10:28:29 | payroll-api: `confirmed`, 10 confirmations; claim settled |
| 10:29:17 | Verification passed; services stopped by the script |

**Actual lookup result:** `seen=Mined(BlockHeight(4521791))`. There was no
lookup error and no txid mismatch: the real lookup checks that the returned
transaction's txid equals the requested one.

**The complete wallet log** has its own file, so no lines were lost:
[`live-obs1c1.wallet.log`](../payout-wallet/evidence/live-obs1c1.wallet.log).
For this payment it contains exactly three lines:
* `payment accepted; building`;
* `transaction built and recorded`;
* `transaction already on the network; recorded as broadcast without resending`.

**Whole-run counts:**

| Log line | Count |
|---|---|
| `transaction built and recorded` | 1 |
| `broadcast accepted` | 0 |
| `broadcast failed` | 0 |
| `chain lookup failed` / `lightwalletd returned transaction` | 0 |
| `recorded as broadcast without resending` | 1 |

The only network send was the original one, at 10:23:31–32. The fault point
interrupts it right after lightwalletd accepts it, so it is never logged as
accepted. The transaction itself shows it was sent.

## 3. Intent state and attempt count

| | State | Attempts | Last error | Txid | Fee |
|---|---|---|---|---|---|
| After the crash (before recovery) | `built` | 0 | — | `100b164f…` | 10,000 |
| After start-up recovery | `broadcast` | **0** | — | same | same |
| 70 s later | `broadcast` | 0 | — | same | same |
| End of run (and re-read afterwards) | `broadcast` | 0 | — | same | same |

* **Intent store:** the intent's `updated_at` moved once, at the observation.
  Afterwards the store had 18 `broadcast`, 2 `failed`, **0 unfinished**.
* **Before the fix (Phase 4 C), for comparison:** the intent stayed `built`
  through 5 refused sends of the mined transaction and reached `broadcast` only
  at attempt 6.

## 4. Transaction identity and independent verification

* **Txid:** `100b164f1343d6b8944ac5967c8308570509a2b27060abe223217dddcb89a3ff`
* **Block:** 4,521,791
* **Fee:** 10,000 zats (estimate 10,000, cap 20,000)
* **Expiry height:** 4,521,828

**ZecBlock** (the Phase 4 verifier,
[`obs1-scenario-c-evidence/independent-100b164f1343d6b8.json`](obs1-scenario-c-evidence/independent-100b164f1343d6b8.json)):
**ALL PASS.**
* Confirmed and canonical (19 confirmations when checked), txid matches.
* Tx v6, NU7 branch.
* 0 transparent inputs and outputs; 2 Ironwood actions.
* Height, size, fee and action counts agree with the zec.rocks observer.
* No plaintext amount, memo, payout id or address in the bytes.
* **The raw bytes on chain are identical (SHA-256) to the bytes the wallet
  persisted before the crash**, so no replacement transaction exists.

**Uniqueness:**
* the treasury wallet holds exactly **1** transaction carrying this payout's
  memo;
* carol received it **exactly once** (20,000 zats in `100b164f…`; total
  2,097,999 → 2,117,999).

## 5. Payroll confirmation and settlement

From the database ([`obs1c1-db.json`](obs1-scenario-c-evidence/obs1c1-db.json))
and `live-obs1c1.payroll-api.log`:
* **Payout `0341c5b2…`:** `compensation_type = vesting`, final state
  `confirmed`; fee 10,000; `max_fee` 20,000; mined 4,521,791.
* **History:** `reserved > requires_reconciliation > submitted > confirmed`.
* **While the wallet was down:** `requires_reconciliation`, claim `reserved`.
* **After the restart:** `submitted` at 2 confirmations (10:24:16), and still
  `submitted` through 4, 6, 7 and 8. The wallet's `broadcast` record did not
  settle anything.
* **`confirmed` only at 10/10** (10:28:29), using the chain-served bytes.
  Vesting claim `1000cd86…` was `settled` once, with `settlement_ref` = the
  txid.
* **Agreement C2 afterwards:** vested 449,074 = paid 20,000 + reserved 0 +
  available 429,074; total = vested + unvested.

## 6. Broadcasts and balances

| Item | Value |
|---|---|
| Transactions broadcast | **1** (the C2 withdrawal) |
| Faucet top-ups | **0** |
| Company account | 880,000 → 850,000 (Δ 30,000 = 20,000 + fee 10,000) |
| Treasury account 0 | 10,267,167 → 10,267,167 (unchanged) |
| Carol | +20,000 |

## 7. Discrepancies and open points

* **None in the behaviour under test.**
* **The mempool branch** (`Seen::Mempool`) was not exercised live: the script
  waited for mining, deliberately. It is covered by the unit test
  `resend::built_and_in_mempool_is_recorded_not_resent`.
* **The lookup-error branch** wasn't exercised live (no failure occurred). It
  is covered by `resend::lookup_error_sends_nothing_and_changes_nothing`.
* **The two one-shot send sites** (`submit` replay, `adopt()`) are unchanged
  by design (see the fix report). They weren't involved in this run.
* **Stray console line:** the operator's console shows two
  `schemerz: Migrating everything` lines. Their timing matches the
  wallet-proof CLI calls that open wallet databases (carol's address, and the
  treasury history), so they are most likely that, not the service. They
  aren't in any service log, and they are harmless.
* **Logs:** no credentials, tokens, seeds, keys or addresses appear in the four
  `live-obs1c1*` logs (checked).

## 8. Files

* **New:** `payout-wallet/scripts/live_obs1_c.sh`; `payout-wallet/evidence/live-obs1c1{,.wallet,.payroll-api,.web}.log`;
  `docs/obs1-scenario-c-evidence/`; this report.
* **Binaries (not tracked):** the pre-fix wallet binaries are kept as
  `payout-wallet/target/debug/*.prefix-obs1.exe`.
* **Databases:** `hushush_live_obs1c1` (new, local test database) remains for
  inspection.
* **Not committed.** Mainnet is untouched and disabled.

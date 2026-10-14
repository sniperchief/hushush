# hushush Phase 4 — Security Hardening and End-to-End Verification

Date: 2026-10-14 · Network: Zcash **testnet** only · Scope: review, verify and harden Phases 0–3. No new product features; no mainnet code path, funds or support were added.

Evidence: [`docs/phase4-evidence/`](phase4-evidence/) (checks, independent verification) and [`payout-wallet/evidence/`](../payout-wallet/evidence/) (`live-p4run1..3.*`).

## 0. Summary

* **Exactly-once held everywhere it was tested, live and mocked.** I tested 14 live testnet transactions (12 payouts) across 7 crash, concurrency and loss scenarios. No payment was ever made twice. A payout that was never built was failed safely. A separately operated node holds, for every payout, **byte-for-byte the signed transaction the wallet persisted before broadcast**, so recovery resent existing transactions and never built new ones.
* **Three exactly-once gaps were found by code review and fixed, with regression tests.** None had occurred in practice:
  - **Intent-store loss could fence a paid payout.** With the wallet's intent store lost, an existing transaction could have been fenced and its claim released.
  - **Expiry failure relied on one data source.** A server on a stale fork could have caused a mined payout to be declared failed.
  - **Crash points were untested.** Three crash points had never been exercised.
* **Security fixes:**
  - **Address probing (privacy).** Org managers could probe a contributor's payout address.
  - **Dev mail outbox.** Enabling it on a public production deployment would log and store live login/invite tokens.
  - **payroll-api bind address.** It could bind to public interfaces without warning.
  - **Upstream debug logs.** Debug logs from upstream wallet crates could be enabled.
  - **Wallet token in the harness.** The live harness leaked the wallet token into the web server's environment.
  - **Brittle start-up.** The wallet exited on a transient DNS failure at start.
* **Privacy was verified independently** against ZecBlock (formerly CipherScan), a separately operated Zebra-compatible node and decoder:
  - Amount, recipient and memo are not visible.
  - Fee (10,000 zats), shape (2 Ironwood actions), size, expiry and timing are visible.
* **Recommendation:** **GO** for Phase 5 work that stays on testnet. **NO-GO** for mainnet payouts until the prerequisites in §7 are met (§8).

## 1. Baseline

Recorded before any Phase 4 change:

* Code state: HEAD `6311af0` (Phases 0–2) plus the uncommitted Phase 3 working
  tree. Per-file SHA-256: [`baseline-manifest.txt`](phase4-evidence/baseline-manifest.txt).
* Every automated check, run by [`run-checks.sh`](phase4-evidence/run-checks.sh):
  [`checks-baseline.txt`](phase4-evidence/checks-baseline.txt) (before) and
  [`checks-final.txt`](phase4-evidence/checks-final.txt) (after).

| Check | Baseline (08:11 UTC) | Final (10:29 UTC) |
|---|---|---|
| `payroll` tests, SQLite **and** PostgreSQL (MOCK wallet/chain for payouts) | 96 passed | **100 passed** (+2 scenarios × 2 databases) |
| `payroll-api` tests | 3 passed | 3 passed |
| `payout-wallet` tests | 8 passed | 8 passed |
| `wallet-proof` (Phase 0) offline tests / live testnet tests | 24 / 3 passed | 24 / 3 passed (code untouched) |
| `web` vitest: real payroll-api + PostgreSQL, MOCK wallet | 52 passed | **56 passed** (+3 mail, +1 address probing) |
| `cargo clippy --all-targets -D warnings` (all 4 crates) | OK | OK |
| `cargo fmt --check`: payroll, payroll-api, payout-wallet | OK | OK |
| `cargo fmt --check`: wallet-proof | **diff (pre-existing since Phase 0)** | same; deliberately not reformatted, since `wallet-proof` is the preserved Phase 0 proof and its code is unchanged |
| web `typecheck`, `lint`, production `build` | OK | OK |
| `npm audit` (production deps) / `cargo audit` (4 lockfiles) | — | 0 / 0 vulnerabilities |
| Live testnet e2e | — | PASS (`p4run2` A–B, `p4run3` C–G; §5) |
| Independent privacy verification (14 transactions) | — | ALL PASS (§3) |

Code changed in Phase 4 (all outside `wallet-proof/`; no change to
transaction construction or cryptography):

- `payout-wallet`: `actor.rs` (adopt, fault points), `intents.rs` (`insert_recovered`), `chain.rs` (re-check, retry), `http.rs` (wallet view, chain gate), `main.rs` (wiring, log cap).
- `payroll`: `payout.rs` (expiry corroboration) and `tests/payouts.rs`.
- `payroll-api`: `payouts.rs` (claimant-only repeat), `main.rs` (bind guard).
- `web`: `lib/mail.ts`, tests, `tests/support/mock-wallet.ts`, `scripts/live-payout.mjs`.
- `payout-wallet/scripts/live_e2e.sh`.

## 2. Exactly-once payment

### 2.1 Lifecycle, with every durable write in order

```text
web  POST /api/claims/{id}/payout           (session + Origin check)
 └─ payroll-api
     1. claim_payout(): existing payout?  → claimant only → return it (duplicate) — wallet NOT called
     2. payout_eligibility(): claimant, claim reserved, org enabled            (engine = authority)
     3. wallet quote: real proposal → fee, sufficiency
     4. request_payout(): BEGIN; lock claim row; INSERT payouts(reserved, lease) ; COMMIT   ← D1
 └─ payout-wallet (single actor thread, one command at a time)
     5. intent exists for payout id? → return it (rebroadcast if built). Never build twice.
     6. tx in wallet DB with memo "hushush payout <id>"? → ADOPT it (Phase 4)               ← D2'
     7. INSERT intent(pending)                                                              ← D2
     8. sync; propose (single pool, fee ≤ max); unlock key; build+prove+sign;
        upstream stores the signed tx in the wallet DB                                      ← D3
     9. UPDATE intent → built (txid + raw bytes)                                             ← D4
    10. broadcast → UPDATE intent → broadcast                                                ← D5
 └─ payroll-api
    11. decide(wallet report) → UPDATE payouts (submitted | requires_reconciliation | failed),
        payout_events                                                                       ← D6
worker (every PAYOUT_POLL_SECS, any number of instances; FOR UPDATE SKIP LOCKED leases)
    12. lookup: chain status (node GetTransaction + wallet's own scan) + verification
    13. confirmed: ONE transaction updates payout → confirmed AND claim → settled(txid)     ← D7
        failed:    ONE transaction updates payout → failed    AND claim → released          ← D7
```

### 2.2 Invariants and what enforces them

| # | Invariant | Enforced by | Tests (MOCK = mock wallet/chain, LIVE = testnet) |
|---|---|---|---|
| I1 | One payout per claim, ever | `UNIQUE(claim_id)` + claim row lock; DB triggers | MOCK `duplicate_requests`, `concurrent_requests_pay_once` (12 threads); web 10 concurrent; **LIVE** 5 concurrent → 1 payout (p4run2 A) |
| I2 | One transaction per payout id, ever | intent row before any work; replay instead of rebuild; **adopt by memo when no intent exists** (Phase 4); actor serializes | MOCK `lost_response_is_reconciled`, `interrupted_submission_recovers_after_restart`; **LIVE** B, C, E, F, G + per-memo count = 1 in the treasury's own history |
| I3 | Recovery resends the *same signed bytes* | raw tx persisted at D4 before broadcast; recovery reads it (or the wallet DB copy at D3) | **LIVE + independent:** ZecBlock raw bytes SHA-256 == persisted bytes for all 12 payouts, incl. C (crash before broadcast), E (crash before D4), F (crash after broadcast), G (intent store lost) |
| I4 | A payout is failed (claim released) only when no transaction can be mined | wallet `failed` only from `pending` (never after build) or fence of an id with no intent **and no memo match**; expiry rule needs tip ≥ expiry + margin, absence from the node's mempool **and** the wallet's own scan saying `expired_unmined` (Phase 4) | MOCK `definite_failure_releases_claim`, `expiry_fails_only_when_certain`, **`expiry_needs_wallet_corroboration`** (new), `decide_never_fails_ambiguous_or_mismatched_reports`; **LIVE** D (failed, 0 transactions exist) |
| I5 | Ambiguity → `requires_reconciliation`, never a second payment | `decide()`; submit errors never retried under a new id; fence before deciding a lost request | MOCK `lost_request_is_fenced_then_failed`, `unacknowledged_broadcast`, `wallet_unavailable_waits`; **LIVE** C, E, F, G all passed through `requires_reconciliation` |
| I6 | Paid ⇔ confirmed ≥ N and verified from chain bytes | `decide()` needs chain-source verification; atomic payout+claim transaction; claims trigger allows settlement only by the confirmed payout's txid | MOCK `submit_then_confirm`, `verification_mismatch_requires_reconciliation`, `reorg_resets_confirmations`, **`settlement_is_atomic`** (new: injected crash mid-settlement → full rollback, later completes); **LIVE** all 12 settled only at 10/10 |
| I7 | Concurrent workers/instances never double-process | leases (`FOR UPDATE SKIP LOCKED`), CAS on (state, lease owner) | MOCK `concurrent_workers` (4×6); **LIVE** 2 payroll-api instances, #1 hard-killed during polling (p4run1 A, p4run2 A) |
| I8 | Restarts at any point are safe | everything above is persisted before it is relied on | **LIVE** payroll-api killed mid-submit (B), during polling (A); wallet killed before build (D), after wallet store (E), after build (C), after broadcast (F) |

### 2.3 Crash-point matrix (live, `p4run2`/`p4run3`, plus Phase 3)

| Crash point | Live scenario | Intent after crash | Recovery | Result |
|---|---|---|---|---|
| payroll-api, after payout row, during wallet call | B | built/broadcast by the wallet after its caller died | lease expiry → cancel returns existing tx → adopt | confirmed, 1 tx |
| wallet, after intent `pending`, before build | **D** (`crash_before_build`) | `pending`, no tx | start-up: no memo match → `failed` | payout failed, claim released, **0 tx** in treasury, carol received nothing |
| wallet, after wallet DB stored the signed tx, before intent `built` | **E** (`crash_after_wallet_store`) | `pending`, tx only in wallet DB | start-up: **memo match → built** → rebroadcast | confirmed, 1 tx, same bytes |
| wallet, after intent `built`, before broadcast | C (`crash_after_build`) | `built`, 0 attempts; node: "not found" | start-up rebroadcast | confirmed, 1 tx, same bytes |
| wallet, after network accepted, before intent `broadcast` | **F** (`crash_after_broadcast`) | `built`, 0 attempts (tx already on network) | rebroadcast of same bytes (accepted) | confirmed, 1 tx, same bytes |
| E + intent store LOST | **G** | none (store moved aside) | payroll cancel → **memo match → adopt** (Phase 4 fix) | confirmed 10 s after restart, 1 tx, same bytes |
| payroll-api during confirmation polling, second instance running | A | — | surviving instance's worker | confirmed once; history `reserved > submitted > confirmed` |
| during claim settlement | MOCK only (`settlement_is_atomic`) | — | DB transaction rollback | no "confirmed but unpaid" state |

A crash *inside* the settlement transaction cannot be timed in a live
process; it is covered by the injected-fault test on both SQLite and
PostgreSQL, and by the database's own atomicity.

### 2.4 Defects found and fixed (all with regression tests)

| ID | Severity | Defect (found by review; never observed live) | Fix | Regression test |
|---|---|---|---|---|
| EO-1 | **High** | If the wallet's intent store was lost or restored from an old backup, the payroll side would **fence** a payout id that already had a (possibly broadcast) transaction, then fail the payout and release the claim → the contributor could claim and be paid again. Likewise a new submit would build a second transaction. | `cancel` and new-id `submit` first search the wallet DB by memo and **adopt** an existing transaction (`IntentStore::insert_recovered`, `Actor::adopt`) | **LIVE** scenario G (intent store moved aside) — adopted, confirmed, 1 tx |
| EO-2 | **Medium** | "Expired → failed → claim released" trusted one source (`GetTransaction` on one lightwalletd). A server on a stale fork (observed in Phase 0) could hide a mined payout. The server's branch was only checked at start-up. | Expiry failure now also requires the wallet's own compact-block scan to report `expired_unmined` with no mined height; the server's network/branch is re-verified every sync and lookups return 503 (payroll waits) while it fails | `expiry_needs_wallet_corroboration` (SQLite + PostgreSQL) and the extended pure `decide` test; **mutation-checked**: fails with the rule removed |
| EO-3 | Medium | Crash recovery paths for "before build", "after wallet store" and "after broadcast" had never been exercised; memo-based recovery was unproven | fault points `crash_before_build`, `crash_after_wallet_store`, `crash_after_broadcast` (test-only feature) | **LIVE** D, E, F |
| EO-4 | Low | Settlement atomicity was assumed, not tested | — (already atomic) | `settlement_is_atomic` (both databases) |
| EO-5 | Low | First observation after mining could report 0 confirmations (tip read before the tx) — found in Phase 3, fixed then | tip ≥ mined height | live runs |
| OPS-1 | Low | Wallet exited at start-up on a transient DNS failure (seen in `p4run2`) | start-up retries lightwalletd (20 × 15 s), branch check on every attempt | `p4run3` |

## 3. Independent privacy verification

**Source.** [ZecBlock](https://testnet.zecblock.com) (formerly CipherScan,
open source at github.com/Kenbak/cipherscan): a separately operated
Zebra-compatible node, indexer and decoder, queried over HTTPS
(`api.testnet.zecblock.com/api/tx/{txid}` and `/raw`). It is on the official
NU7 chain: its block 4,465,026 hash is `000089ba…e24e78af`, the same as on
`testnet.zec.rocks`. Script: [`independent_verify.py`](phase4-evidence/independent_verify.py);
per-transaction records: `phase4-evidence/independent-*.json`.

**Transactions checked (14):** the Phase 0 payment, the Phase 3 treasury
funding, and 12 payouts (Phase 3 runs 1–2 and Phase 4 runs 1–3).

**Results:** ALL PASS. For every transaction:

| Check | Result |
|---|---|
| txid, confirmed, canonical (ZecBlock) | yes |
| header: tx v6, NU7 branch `0x77190ad9` (parsed from the raw bytes) | yes |
| transparent inputs / outputs | 0 / 0 (ZecBlock: `inputCount 0`, `outputCount 0`, `totalOutput 0`) |
| shielded shape | 2 Ironwood actions; 0 Orchard; 0 Sapling spends/outputs |
| fee | 10,000 zats = Ironwood value balance (public) |
| ZecBlock vs zec.rocks + our keyless observer | identical height, size, fee and action counts |
| raw bytes (ZecBlock) vs bytes the wallet persisted before broadcast | **identical SHA-256** for all 12 payouts |
| plaintext amount (8-byte LE `150000`), memo text, payout id, recipient UA string in the raw bytes | none present |

**What a public observer sees** (no keys): that the transaction exists; its
block height and time; size (9,166 bytes); version/branch; **fee** (10,000
zats, which implies the ZIP 317 action count); **shape** (2 Ironwood actions —
padded, so it bounds but does not reveal the recipient count); expiry height
(≈ mined height + 38, so roughly *when it was built*); Ironwood anchor (roughly
which height the spent notes were proven against); nullifiers and note
commitments (unlinkable without keys).

**What they don't see:** the amount, the sender and recipient addresses, the
memo, or which notes were spent.

**Metadata that remains visible or correlatable (not claimed private):**

* **Timing.** Each payout is broadcast when the contributor clicks "Send".
  The live runs show payouts from the treasury every few minutes; anyone
  watching block times can see bursts of 2-action Ironwood transactions. They
  can't link them to one wallet from the chain alone, but a cadence that
  matches a company's payroll activity is a signal.
* **Funding.** The treasury was funded by a fully shielded Ironwood transaction
  (also verified above); funding from a transparent address or exchange would
  be public at that step.
* **Fee and shape** are identical for every payout. That is common on testnet
  (Phase 0/3 transfers have the same shape), but it is not a mixer.
* **Infrastructure.** The lightwalletd operator (zec.rocks) sees our IP,
  broadcast timing and every txid we look up. ZecBlock now also knows which
  txids we queried (this verification).

**Limitations.** ZecBlock is independently *operated* and *decoded*. I could
not verify that its node doesn't peer with or depend on zec.rocks (all
nodes share one P2P network by design), and I did not run our own Zebra node.
"Amount and recipient not visible" is shown by absence of plaintext and by
the protocol design (encrypted notes); I did not attempt cryptanalysis.
Nothing here checks the *recipient's* side beyond carol's own wallet
receiving the exact amount and memo.

## 4. Security review findings

Reviewed:

- the browser → web → payroll-api → wallet boundaries;
- authentication and authorization paths;
- claim ownership and cross-organization access;
- sessions and CSRF;
- SQL construction;
- XSS sinks;
- secrets in files, environment, logs, errors and evidence;
- retries, replay and idempotency;
- network exposure and defaults;
- headers and CORS;
- dependencies.

Fixed findings have regression tests unless noted.

| ID | Severity | Finding | Evidence | Status |
|---|---|---|---|---|
| EO-1 | High | Intent-store loss could lead to a double payment (§2.4) | code review; live G | **Fixed** |
| SEC-1 | **Medium** | **Payout address probing.** Once a claim had a payout, `POST /payout` answered `200 duplicate` for the right address and `409` for a wrong one *before* the claimant check, so an org owner/admin (who can view the claim) could confirm a guessed contributor address. | code review | **Fixed**: only the claimant may repeat; others get an identical 403. Test: web `only the claimant can repeat a request (no address probing by managers)` |
| SEC-2 | **Medium** | **Dev mail outbox in production.** `HUSHUSH_ALLOW_DEV_OUTBOX=1` on a public deployment stored every verification/invite link in the database and **logged it to stdout** (live login and invitation tokens). | `web/src/lib/mail.ts` | **Fixed**: allowed in production only with a localhost `APP_URL`; mail bodies are never logged in production. Test: `web/tests/mail.test.ts` (3 cases) |
| EO-2 | Medium | Expiry decision relied on one source (§2.4) | code review | **Fixed** |
| SEC-3 | Medium | **Wallet token in the web server's environment (test harness).** `live_e2e.sh` exported `WALLET_API_TOKEN`, so the `next start` it launched inherited it — contrary to the isolation rule. Production docs never did this. | script review | **Fixed** in the harness: passed only to the wallet and payroll-api processes |
| SEC-4 | Low | payroll-api (which trusts `X-Actor-Id` from any holder of its token) could be bound to a public interface without warning | `payroll-api/src/main.rs` | **Fixed**: non-loopback refused unless `PAYROLL_API_ALLOW_NONLOCAL=1` (same rule the wallet already had) |
| SEC-5 | Low | Upstream wallet/crypto crates' debug/trace logs (which may contain addresses or note data) could be enabled via `RUST_LOG` in the payout wallet | `payout-wallet/src/main.rs` | **Fixed**: those targets are capped at `info` regardless of `RUST_LOG` |
| SEC-6 | Low | No script Content-Security-Policy (only `frame-ancestors`, `base-uri`, `form-action`) | `web/next.config.ts` | **Open** (known since Phase 2). No XSS sink found (`dangerouslySetInnerHTML`/`eval`: none; React escapes output), so the risk is defence-in-depth only |
| SEC-7 | Low | Rate limiting is per e-mail for sign-in only; no IP limits; no limits on payout quotes (each quote runs a wallet proposal in the single wallet actor, so a malicious contributor could slow payouts) | code review | **Open**: put IP/user rate limits at the proxy |
| SEC-8 | Low | Dev-only npm dependencies: 5 "high" advisories (`braces` stack-exhaustion DoS via `eslint-config-next`); npm's fix is an unrelated major downgrade | `npm audit` | **Accepted**: dev tooling only, not shipped. **Production npm dependencies: 0 vulnerabilities. Rust (all 4 lockfiles, 450 crates in the largest): 0 vulnerabilities, 0 warnings** (`cargo audit`, RustSec DB 1,310 advisories) |
| SEC-9 | Info | Spending key handling: decrypted mnemonic/seed/key exist in process memory while signing; a crash dump (Windows Error Reporting) or memory snapshot of the payout wallet could contain them | design review | **Open**: disable crash dumps for the service account, or move signing to an HSM / offline PCZT signer (§7) |
| SEC-10 | Info | Anyone holding `PAYROLL_API_TOKEN` can act as any user (by design: Next.js authenticates and asserts the actor) | design review | Documented; token only in the web server's environment, service loopback-only |

**Checked and found sound:**

- Every state-changing web route (17) goes through the Origin-checking `mutation` wrapper.
- Sessions are 256-bit, hashed at rest, `HttpOnly`, `SameSite=Lax`, with `Secure` and `__Host-` over HTTPS, idle and absolute expiry.
- Claim ownership and cross-org access are enforced in the engine and the DB (outsiders get 404; the employer can't pay out or redirect a contributor's wages).
- All SQL is parameterized (formatted SQL only interpolates constant column lists or validated names).
- No CORS headers on any service.
- Security headers present (HSTS, nosniff, frame denial, no-referrer, permissions policy), `poweredByHeader` off.
- The wallet API is token-authenticated, loopback-only by default, refuses wallet/key paths inside a git tree, and never returns addresses (fingerprints only).
- Tokens are compared in constant time.
- No secrets in the repository, evidence files, or logs: scanned for the wallet token, bearer tokens, age secret keys and private keys; addresses appear only in the Phase 3 funding log written by the CLI.
- Session cookies used by the live harness and moved-aside intent stores live under `%LOCALAPPDATA%`, outside the repository.
- No development endpoints are exposed: the dev outbox is a table, `scripts/*` are CLI tools.

## 5. End-to-end regression (live testnet)

The actual product flow, against `testnet.zec.rocks` (lightwalletd v0.5.4,
NU7), through `next start`, payroll-api, PostgreSQL and the payout wallet,
with **real TAZ**. Confirmation policy 10.

| Step | Evidence (`payout-wallet/evidence/live-p4run2.log`, `live-p4run3.log`) |
|---|---|
| 1. Employer signs in, creates an org, invites the contributor, creates a stream | `setup` (sign-up, e-mail verification via link, sign-in, org, invitation accepted, stream) |
| 2. Contributor views earned and available wages | `wages`: contributor page 200 shows earned/available `0.00600000` = payroll-api's server-computed balance; employer org page 200 |
| 3. Contributor requests a claim | `claim A …` (0.0015 ZEC) |
| 4. Employer-funded payout wallet covers claim + fee | quote: amount 150,000, fee 10,000 (real proposal), `sufficient: true` |
| 5. Real shielded payout submitted and confirmed | e.g. A: `f17b057f…0fd26847`, mined 4,500,884, 10/10 |
| 6. Contributor's wallet receives the expected amount | carol's own wallet: each payout exactly once, 150,000 zats, memo `hushush payout <id>`; run 3: 1,200,000 → 1,800,000 (+4 × 150,000) |
| 7. Paid only after policy + checks | claim stays `reserved` while `submitted`; `settled` only at 10/10 with chain verification; page "Paid · payout confirmed" |
| 8. Repeats, restarts, reconciliation create no duplicates | 5 concurrent requests → 1 payout; repeats after confirmation → duplicate; crash scenarios B–G; treasury: exactly 1 transaction per payout memo; treasury delta = payouts + fees exactly (run 3: 2,720,000 → 2,080,000) |
| 9. Transaction inspected for public visibility | §3 (independent node) |

**Run log, stated plainly:**

* `p4run1`: A **passed on chain** (payout `2cbdef9b…1f91` was confirmed by the
  second instance after the first was killed). The harness then hung because
  its wait polled the killed instance; I stopped it and fixed the harness.
* `p4run2`: A and B **passed**. C was aborted when the wallet couldn't resolve
  `testnet.zec.rocks` (transient DNS) at restart and exited, before any payout
  for C existed. This led to the OPS-1 fix.
* `p4run3` (C, D, E, F, G): all payment checks **passed**. The final page
  assertion first failed on a console-encoding bug in the harness (the data
  showed the correct label); after the fix, a resume of the same run passed.
  Its console showed "Payout failed" for D, correctly not "Paid".

## 6. Live versus mocked

| Behaviour | Live testnet (real funds) | Mocked |
|---|---|---|
| Normal payout, confirmation policy, recipient receipt + memo, treasury accounting | ✔ (12 payouts, Phases 3–4) | ✔ |
| 5 concurrent duplicate requests | ✔ | ✔ (10–12 concurrent) |
| Two payroll-api instances processing the same payouts; one killed in polling | ✔ | ✔ (4 workers) |
| payroll-api killed mid-submission | ✔ | ✔ |
| Wallet crash before build / after wallet store / after build / after broadcast | ✔ (D, E, C, F) | — (needs a real wallet) |
| Intent store lost | ✔ (G) | — |
| Same signed bytes after recovery (independent node) | ✔ (all 12) | — |
| Crash in the middle of claim settlement | — (not timeable live) | ✔ (SQLite + PostgreSQL) |
| Reorg resetting confirmations | — (none occurred) | ✔ |
| Expired-unmined transaction → failure; stale-fork-style disagreement | — (never happened) | ✔ |
| Verification mismatch (wrong amount/recipient/pool crossing/extra output) | — | ✔ |
| Lost request / lost response / wallet unavailable | partly (lost responses in C/E/F/G) | ✔ |
| Stale keep-alive connection to a restarted wallet | ✔ (wallet swapped mid-run in C–G; no failure since the Phase 3 fix) | ✔ (smoke test only; does not reproduce the original fault) |
| Authorization, CSRF, input validation, address probing, dev outbox | — | ✔ (real API + DB; mock wallet) |

## 7. Remaining risks and production prerequisites

1. **Mainnet does not exist here.** The wallet stack is pinned to librustzcash
   pre-releases; the network is fixed to testnet at compile time; mainnet NU7
   is not active. Needed: stable NU7-capable crates, a reviewed upgrade, and a
   separate mainnet build and audit.
2. **Key custody.** The spending key is an age-encrypted mnemonic + identity
   file on the service host, decrypted into process memory per payout (SEC-9).
   Needed: an HSM/KMS or an offline PCZT signer, with crash dumps disabled.
3. **Own infrastructure.** Every live action used one public lightwalletd.
   Needed: your own Zebra + lightwalletd, and a second independent source
   for confirmations (ZecBlock shows the approach works). Every intent and
   wallet DB must be backed up. EO-1 now survives intent-store loss; losing
   the **wallet DB too** before a transaction is mined would still need
   manual reconciliation.
4. **Per-organization treasuries.** A single payout wallet serves every enabled
   organization; `PAYOUT_ENABLED_ORGS=*` is testnet-only.
5. **Note management and capacity.** Payouts are serialized through one wallet
   actor; change needs 3 confirmations; quotes are not rate-limited (SEC-7).
6. **Operations.** Needed:
   - alerting on `requires_reconciliation` and on lightwalletd branch-check failures;
   - an operator runbook and tool;
   - a process supervisor (the wallet now retries at start-up but still exits after ~5 min of failure).
7. **Web.** Still missing: script CSP (SEC-6), IP rate limiting, and password reset. No real-browser pass of the payout UI has been done.
8. **Timing privacy.** Immediate broadcast on click is a product choice with a
   correlation cost (§3). Batching or delays would need product approval
   (out of scope here).

## 8. Recommendation for Phase 5

**GO for Phase 5 on testnet. NO-GO for mainnet payouts.**

The exactly-once design held under every crash, concurrency and loss scenario
I could produce live, and the gaps found by review are fixed and tested.
Privacy matches the design, as checked by an independent node and decoder.

Mainnet would still put real money behind:

- a pre-release wallet stack;
- a hot key on disk;
- one public infrastructure provider;
- one shared treasury;
- operational tooling that doesn't exist yet.

Phase 5 should address §7.1–7.6 (custody, own nodes, per-org treasuries,
monitoring and runbooks) before any mainnet funds are used.

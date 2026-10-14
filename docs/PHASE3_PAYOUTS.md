# hushush Phase 3 — Real Shielded Payouts (testnet)

Date: 2026-10-13 · Network: Zcash **testnet** only · Code: [`payout-wallet/`](../payout-wallet/), [`payroll/src/payout.rs`](../payroll/src/payout.rs), [`payroll-api/src/payouts.rs`](../payroll-api/src/payouts.rs), [`web/`](../web/)

> **Status: Phase 3 complete on testnet.** Real claims were paid out end to end
> on the official NU7 Zcash testnet through the product path: browser routes →
> Next.js → payroll-api → payout wallet → lightwalletd. Each payout was
> submitted, confirmed (10 blocks), verified from chain data and reconciled
> after deliberate crashes of payroll-api and of the wallet, with **no duplicate
> payment**. Live run `p3run2` passed all checks (§9). Mainnet is not supported
> and must not be used (§10).

> **Phase 4 update (2026-10-14):** hardened and re-verified — see
> [PHASE4_REPORT.md](PHASE4_REPORT.md). Behaviour changes:
> - the wallet adopts an existing transaction (found by memo) instead of fencing or rebuilding;
> - expiry-based failure needs the wallet's own scan to agree;
> - lookups return 503 while lightwalletd fails its branch check;
> - only the claimant may repeat a payout request;
> - payroll-api refuses non-loopback binds unless `PAYROLL_API_ALLOW_NONLOCAL=1`;
> - the dev mail outbox is refused in production except on localhost.

A contributor can now pay a reserved claim out to their own shielded testnet
address. The payout is a fully shielded Ironwood→Ironwood transaction, built
by the exact Phase 0 wallet code. The claim is marked paid only after the
transaction has the configured number of confirmations **and** the wallet has
verified recipient, amount and memo from the chain copy of the transaction.

## 1. Architecture and trust boundaries

```text
Browser ─HTTPS─▶ Next.js (web/) ─bearer A─▶ payroll-api ─bearer B─▶ payout-wallet ─TLS─▶ lightwalletd (testnet)
                  auth, CSRF, UI            payout state machine     ONLY process with      chain
                                             + worker (PostgreSQL)    spend authority
                                                                      wallet DB + intents
                                                                      age-encrypted mnemonic
```

| Component | Holds | Never holds |
|---|---|---|
| Browser | session cookie | keys, wallet token, payroll token |
| Next.js | session secrets, `PAYROLL_API_TOKEN` | `WALLET_API_TOKEN`, keys, addresses at rest |
| payroll-api + PostgreSQL | payout records, txids, **salted fingerprint** of the destination | addresses, keys, mnemonic |
| payout-wallet | wallet DB (notes, sent txs), payment intents (incl. addresses), encrypted mnemonic + separate age identity | payroll data beyond payment id + amount |

* `payout-wallet` listens on loopback only (refuses otherwise unless
  `WALLET_API_ALLOW_NONLOCAL=1` behind a TLS proxy), checks its own bearer
  token in constant time, and refuses to start if its wallet or key files are
  inside a git working tree (Phase 0 `ensure_outside_source_control`).
* The spending key exists in memory only while a transaction is signed: the
  mnemonic is decrypted with the age identity, the key derived and dropped —
  exactly as the Phase 0 `send` command does.
* No component logs mnemonics, keys or destination addresses. Logs carry
  payout ids, claim ids, txids, states and amounts.

## 2. Reuse of the Phase 0 wallet

`payout-wallet` depends on `wallet-proof` as a library (path dependency, same
pinned NU7 pre-release crates, identical `Cargo.lock` versions). `wallet-proof`
itself is unchanged. Reused verbatim:

| Need | Phase 0 code |
|---|---|
| open/migrate wallet DB, account, receive address, balances, history | `wallet::{open, sole_account, receive_address, balance, history}` |
| chain sync + memo/status enhancement | `sync::run`, `wallet::enhance` |
| shielded-only recipient check, no-transparent proposal check | `payment::{validate_recipient, propose_shielded_payment, assert_fully_shielded}` |
| ZIP 317 fee, proving with bundled params, signing, storing | `payment::{summarize, build}` (upstream `create_proposed_transactions`) |
| broadcast | `payment::broadcast` |
| seed handling (age) | `secrets::{load_mnemonic, seed_from_mnemonic}`, `wallet::spending_key` |
| lightwalletd TLS + testnet check, tx fetch, keyless observer | `remote::*`, `wallet::fetch_transaction`, `observer::observe` |

New in Phase 3 (no cryptography implemented): the per-payment intent store,
crash recovery, rebroadcast, a stricter recipient and pool policy, chain-based
verification (upstream `decrypt_transaction` with the wallet's own viewing key)
and the HTTP interface.

### Wallet service interface (`Authorization: Bearer $WALLET_API_TOKEN`)

| Route | Purpose |
|---|---|
| `GET /v1/status` | balances (spendable / pending / per pool), sync height, funding address |
| `POST /v1/recipients/validate` | payout address policy |
| `POST /v1/quotes` | builds a real proposal on current wallet state: actual ZIP 317 fee, sufficiency, pool |
| `POST /v1/payments` | pay; idempotent per `payment_id` (= payout id) |
| `GET /v1/payments/{id}` | intent state + live chain status + verification |
| `POST /v1/payments/{id}/cancel` | fence an id the wallet has not seen |
| `GET /v1/transactions/{txid}` | keyless observer view + confirmations |

## 3. Payment flow

1. **Quote** (`POST /api/claims/{id}/payout/quote`). The engine checks the
   actor is the claimant and the claim is reserved and its org is enabled; the
   amount comes from the claim. The wallet validates the address and builds a
   real proposal to get the actual fee and check `spendable ≥ amount + fee`.
   The UI shows amount, fee (paid by the employer's wallet), destination,
   confirmation policy, and what is/isn't private.
2. **Pay** (`POST /api/claims/{id}/payout`, the deliberate second click).
   * If the claim already has a payout, it is returned (`duplicate`) — same
     address required, else `409 payout_conflict`. The wallet is not called.
   * Otherwise quote again, then **record the payout `reserved` in PostgreSQL
     before any wallet call**, leased to this process, with `UNIQUE (claim_id)`
     and the claim row locked.
   * Submit to the wallet with `payment_id = payout id` and
     `max_fee = min(estimate + headroom, cap)`.
   * Record the outcome with [`decide`](../payroll/src/payout.rs).
3. **Worker** (in payroll-api, every `PAYOUT_POLL_SECS`, and at start-up)
   leases unfinished payouts (`FOR UPDATE SKIP LOCKED`), asks the wallet, and
   moves them forward until `confirmed` or `failed`.

### State machine

```text
reserved ──▶ submitted ──▶ confirmed                    (claim → settled, settlement_ref = txid)
   │  ╲          │ ╲
   │   ╲         │  ╲──▶ failed                         (claim → released)
   │    ╲        ▼
   │     ╲──▶ requires_reconciliation ──▶ submitted | confirmed | failed
   ╰──────────────────────────────────────▶ failed
```

| State | Meaning | The claim |
|---|---|---|
| `reserved` | payout recorded; wallet may or may not have been asked | reserved, unpaid, can't be cancelled |
| `submitted` | a transaction exists and the network accepted it | reserved, **unpaid** ("Sent · n/N confirmations") |
| `confirmed` | ≥ N confirmations **and** chain verification passed | **settled** = paid |
| `failed` | certain no transaction was or can be mined | released; amount available again |
| `requires_reconciliation` | outcome unknown or inconsistent | reserved, unpaid ("Checking payout") |

Enforced three times: the engine (`PayoutState::can_become`), PostgreSQL and
SQLite triggers (valid transitions, final states, write-once txid, immutable
terms, no deletes, append-only `payout_events` audit log), and a claims trigger
that lets a claim with a payout be resolved **only** by that payout (settled
with its txid when confirmed, released when failed). Manual `release_claim` /
`settle_claim` refuse claims that have a payout.

### Decision rules ([`decide`](../payroll/src/payout.rs))

| Wallet reports | Next state |
|---|---|
| different payment id / fingerprint / amount, other txid, fee > max, verification failure | `requires_reconciliation` (never failed, never confirmed) |
| `failed` (no tx built) and no txid recorded | `failed` |
| `failed` but a txid was recorded | `requires_reconciliation` |
| `pending` | wait (or `requires_reconciliation` right after submit) |
| `built` (not acknowledged by the network) | `requires_reconciliation` with txid recorded; wallet rebroadcasts |
| `broadcast`, unmined | `submitted` |
| mined, < N confirmations | `submitted` (confirmations updated; a reorg resets them) |
| mined, ≥ N, verification source = chain and passing | `confirmed` |
| unmined, not in mempool, tip ≥ expiry height + margin | `failed` (it can never be mined) |
| transport error / timeout / 5xx | `requires_reconciliation` (on submit) or wait (worker) |

## 4. Idempotency, concurrency and crash safety

No single mechanism is trusted alone:

1. **One payout per claim, ever** — `UNIQUE (claim_id)` plus a row lock on the
   claim during `request_payout`. Duplicate and concurrent requests return the
   existing payout. A failed payout releases its claim; trying again means a
   new claim (new payout id).
2. **One transaction per payout id, ever** — the wallet writes a `pending`
   intent (keyed by payout id) *before* touching the wallet, never builds twice
   for an id, and replays the stored record (and rebroadcasts the stored raw
   transaction) for repeats. Reusing an id with different parameters is `409`.
3. **Linearized wallet** — one actor thread owns the wallet DB and processes
   submit/cancel/quote/background sync one at a time; a lock file prevents a
   second service on the same wallet.
4. **Persist before you rely on it** — payout row before the wallet call; the
   built transaction (txid + raw bytes) before broadcast; txid recorded on the
   payout as soon as known.
5. **Leases** — the request owns its new payout for `submit timeout + 120 s`;
   workers lease for 120 s. Restarted or concurrent processes only touch
   payouts whose lease expired.
6. **Fencing** — a `reserved` payout whose lease expired (its submitter
   crashed) is first *cancelled* at the wallet: if the wallet never saw it, the
   id is fenced (recorded as failed) so a late request can't pay it; if it did,
   the existing transaction is adopted. Only then is anything decided.
7. **Wallet crash recovery** — at start-up every `pending` intent is resolved
   by searching the wallet for a sent transaction carrying its memo
   (`hushush payout <id>`): found → `built` (then rebroadcast), not found →
   `failed`. Built-but-unacknowledged transactions are rebroadcast every sync
   until mined or expired.
8. **Failure only when certain** — an unmined transaction is failed only when
   the chain tip is `PAYOUT_EXPIRY_MARGIN_BLOCKS` past its expiry height and it
   is in no mempool; ZIP 203 expiry makes it unmineable after that. Everything
   else ambiguous goes to `requires_reconciliation`.

## 5. Shielding and privacy

* **Shielded-to-shielded only.** Recipients must be testnet Unified Addresses
  with an Orchard receiver; transparent, TEX, Sapling-only, wrong-network
  addresses are refused. Proposals must be single-step and touch exactly one
  shielded pool (no pool crossing, which would publish the amount), and the
  Phase 0 `assert_fully_shielded` still applies. There is **no transparent
  fallback** anywhere in the code.
* **Verification** decrypts the mined transaction's outgoing outputs with the
  wallet's own viewing key: exactly one external output, to the claimant's
  address (compared via the salted fingerprint), for the claim amount, with
  this payout's memo; the keyless observer view must be fully shielded and
  single-pool, fee ≤ max.
* **Stored in PostgreSQL:** payout id, claim id, amount, fee, txid, heights,
  confirmations, state history, and `sha256("hushush-payout-recipient-v1" ‖
  payout id ‖ address)` — not the address. Fingerprints of the same address in
  two payouts differ.
* **Stored in the wallet's intent DB** (outside the repo, next to the wallet
  DB): destination address, amount, raw transaction. The wallet DB itself
  already records sent notes.
* **Memo** `hushush payout <payout id>` is encrypted to the recipient (and
  recoverable by the employer wallet's viewing key). It reveals nothing on
  chain.

### What an independent observer can see

See §9 for the live observation of the payouts. In short, from the chain
alone (no keys): the transaction exists, its time/height, its expiry height,
its size and shape (2 Ironwood actions — padded, so recipient count is only
bounded), and its **fee** (10,000 zats = ZIP 317 for 2 logical actions). Not
visible: sender, recipient, amount, memo, which notes were spent.

**Not private, and not claimed to be:**

* **Timing.** The broadcast time is when the contributor clicked "Send"; the
  lightwalletd operator sees the broadcast (and the wallet's IP) immediately.
  Correlating payout times with a company's activity is possible.
* **Funding relationships.** How the payout wallet was funded is outside
  hushush's control: funding from a transparent address or an exchange is
  public at that step. Funding that crosses pools reveals amounts.
* **Infrastructure metadata.** The lightwalletd operator sees client IP,
  tree-state requests (≈ wallet birthday), every fetched txid (so it can link
  "this client watched this txid") and broadcast timing. Use your own
  lightwalletd/Zebra (or Tor) in production.
* **Viewing key holders** (the employer, anyone with the payout wallet's
  UFVK or seed) see every payout, recipient address and memo.
* **The recipient** sees the payout id in the memo.
* **Fee and shape** are public and identical for every simple payout, which
  is good for blending in with other 2-action transactions but not a mixer.

## 6. Configuration and secrets

### payout-wallet

| Variable | Default | |
|---|---|---|
| `WALLET_API_TOKEN` | required, ≥ 32 chars | shared only with payroll-api |
| `WALLET_API_ADDR` | `127.0.0.1:7979` | non-loopback refused unless `WALLET_API_ALLOW_NONLOCAL=1` |
| `WALLET_NAME` | required | wallet created with the Phase 0 CLI |
| `WALLET_LIGHTWALLETD_URL` | **required, no default** | the server must be chosen deliberately; checked to be testnet and on the expected consensus branch |
| `WALLET_MAX_PAYMENT_ZATS` | `100000000` | per-payment cap |
| `WALLET_MAX_FEE_ZATS` | `100000` | |
| `WALLET_SYNC_SECS` | `30` | background sync + rebroadcast |
| `HUSHUSH_DATA_DIR`, `HUSHUSH_SECRETS_DIR` | `%LOCALAPPDATA%\hushush-phase0\…` | wallet DB / identity locations, must be outside git |

Files per wallet: `wallets/<name>/data.sqlite` (wallet DB),
`wallets/<name>/mnemonic.age`, `wallets/<name>/payout-intents.sqlite`,
`wallets/<name>/payout-wallet.lock`, and `secrets/<name>.age-identity.txt`
(the identity alone or the encrypted mnemonic alone grants nothing).

### payroll-api (in addition to Phase 2)

| Variable | Default | |
|---|---|---|
| `WALLET_URL` | unset → payouts disabled | `https://…`, or `http://` on localhost only |
| `WALLET_API_TOKEN` | required with `WALLET_URL` | |
| `PAYOUT_ENABLED_ORGS` | required with `WALLET_URL` | comma-separated org ids; `*` = all (testnet/dev only) |
| `PAYOUT_ADDRESS_KEY` | required with `WALLET_URL` (Phase 5) | 64 hex chars (32 random bytes): seals workers' locked payout addresses at rest (XChaCha20-Poly1305). Losing or changing it makes stored addresses unreadable: the worker's stream view and withdrawals then fail with an internal error until the key is restored (nothing is paid to any other address). Secret: payroll-api only |
| `PAYOUT_CONFIRMATIONS` | `10` | |
| `PAYOUT_EXPIRY_MARGIN_BLOCKS` | `20` | |
| `PAYOUT_MAX_FEE_ZATS` / `PAYOUT_FEE_HEADROOM_ZATS` | `100000` / `10000` | |
| `PAYOUT_POLL_SECS` | `20` | worker interval |
| `WALLET_SUBMIT_TIMEOUT_SECS` / `WALLET_TIMEOUT_SECS` | `300` / `60` | |

PostgreSQL migrations `0002_payouts` and `0003_rls` are applied by
payroll-api at start-up (SQLite: `0002` on open). The web app's
`0002_rls.sql` is applied by `npm run db:migrate`.

### Hosted PostgreSQL (e.g. Supabase)

Hosted providers often sign the database certificate with their own CA and
put a connection pooler in front. Both services support that:

| Variable | Service | |
|---|---|---|
| `PAYROLL_DATABASE_CA_PEM` (PEM text) or `PAYROLL_DATABASE_CA_FILE` (path) | payroll-api | extra trusted CA; TLS becomes mandatory and fully verified |
| `PAYROLL_DB_POOL_SIZE` | payroll-api | default `16`; connections are opened on demand; use about `5` behind a pooler |
| `DATABASE_CA_PEM` (PEM text) | web app, migrator, test harness, `scripts/smoke.mjs`, `scripts/check-db.mjs` | replaces the URL's `sslmode` with full verification against this CA |
| `DATABASE_POOL_MAX` | web app | default `10`; about `2` on serverless (Vercel) |

Literal `\n` sequences in the PEM variables are accepted (single-line secret
stores).

Every table has **row-level security enabled with no policies**. The app's own role owns the tables and bypasses RLS. Any other role sees nothing, including platform API roles such as Supabase's `anon` and `authenticated`.

On Supabase, use the **session pooler** connection string (port 5432). Don't use the transaction pooler (port 6543): payroll-api uses prepared statements.

Before deploying, run `node web/scripts/check-db.mjs` against the real database.
All of this was verified against a TLS-only PostgreSQL whose certificate is signed by a private CA: web suite 56/56, payroll PostgreSQL suites 101/101, and an API-role probe saw 0 rows in all 16 tables.

## 7. Operating the payout wallet (testnet)

```sh
# 1. Create the payout wallet (Phase 0 CLI). Back up the mnemonic offline.
cd wallet-proof && cargo build
./target/debug/hushush-wallet-proof init treasury
./target/debug/hushush-wallet-proof backup-mnemonic treasury --out <offline path>
# 2. Fund it with testnet ZEC from a SHIELDED wallet (its address is also shown
#    to org owners/admins on the organization page). Wait 10 confirmations.
# 3. Run the service (keep it running; one per wallet).
cd payout-wallet && cargo build
WALLET_API_TOKEN=… WALLET_NAME=treasury WALLET_LIGHTWALLETD_URL=https://testnet.zec.rocks:443 \
  ./target/debug/hushush-payout-wallet
# 4. Point payroll-api at it: WALLET_URL=http://127.0.0.1:7979 WALLET_API_TOKEN=… PAYOUT_ENABLED_ORGS=<org id>
```

* Do **not** run the Phase 0 CLI against the service's wallet while the service
  runs (two writers would race on note selection). Use `GET /v1/status`.
* After a payout, the wallet's change needs 3 confirmations before it is
  spendable again; quotes report "waiting for confirmations" meanwhile. Fund
  the wallet with several notes to pay several claims in quick succession.
* `requires_reconciliation` payouts are retried automatically every poll. One
  that stays there has a mismatch (wrong txid, failed verification, wallet lost
  its record): investigate with `GET /v1/payments/{id}` on the wallet and the
  `payout_events` table; never "fix" it by paying again.
* Recovery of the wallet itself: Phase 0 §6 (mnemonic + birthday). The intent
  DB can be lost without double-paying only if the payroll side's payouts are
  resolved first: every in-flight payout's txid is also in PostgreSQL.

## 8. Tests

All runs on 2026-10-13/14, Windows 10, Rust 1.99.0, Node 24.18, PostgreSQL 17.10.

| Command | Wallet / chain | Result |
|---|---|---|
| `payroll`: `cargo test -- --include-ignored` (with `PAYROLL_TEST_DATABASE_URL`) | **MOCK wallet + MOCK chain** in `tests/payouts.rs` | **96 passed**: 14 unit, 17 backend scenarios (Phase 2), 29 Phase 1 engine, 36 payout (16 scenarios × SQLite **and** PostgreSQL, + 4 pure decision tests) |
| `payroll-api`: `cargo test` | — | 3 passed (wallet client, URL policy, org allow-list) |
| `payout-wallet`: `cargo test` | real policy code and real address encodings; no chain | 8 passed (intent store transitions/fences/write-once/reopen, shared fingerprint vector, recipient policy) |
| `web`: `npm test` (with `TEST_DATABASE_URL`) | real payroll-api + PostgreSQL, **MOCK HTTP wallet** (`tests/support/mock-wallet.ts`) | **52 passed**: 42 Phase 2 (unchanged) + 10 payout |
| `web`: `npm run typecheck`, `npm run lint`, `npm run build` | — | OK |
| `web`: `node scripts/smoke.mjs` (Phase 2 smoke, payouts disabled) | — | **31/31 passed** |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` in `payroll`, `payroll-api`, `payout-wallet` | — | OK |
| `wallet-proof` (Phase 0, code unchanged): `cargo test`; `cargo test --test testnet_live -- --ignored` | mocked chain / **live testnet** | 24 passed; 3 live passed |
| `payout-wallet/scripts/live_e2e.sh` (`RUN_ID=p3run2`) | **real wallet, live NU7 testnet, real TAZ** | **PASS** (§9) |

The mocked suites cover:

- **Authorization:** only the claimant may pay; the employer gets 403, outsiders and colleagues get 404; CSRF and unauthenticated requests are refused.
- **Recipient validation:** malformed, Sapling-only and non-shielded addresses are rejected.
- **Fee and balance:** an insufficient wallet gives 409 and no payout is created.
- **Duplicate requests:** sequential repeats return the same payout; a different address is a conflict; 10–12 concurrent requests produce one transaction.
- **Concurrent workers:** 4 workers × 6 payouts, each payout processed once.
- **State transitions:** a table test, plus schema guards against raw SQL on both databases.
- **Interrupted submissions and restart recovery:** lease expiry, fence, adopt.
- **Network faults:** lost requests and responses, unacknowledged broadcasts, reorgs resetting confirmations.
- **Expiry:** a payout fails only past the margin and when absent from the mempool.
- **Verification mismatches:** wrong amount, wrong recipient, pool crossing or an extra output is never confirmed.
- **Wallet unavailable:** the worker waits and never decides.

One Phase 1 test line changed. `migrations_are_idempotent_and_state_persists`
asserted `COUNT(*) FROM schema_migrations == 1`; with migration 0002 it now
asserts `== MIGRATIONS.len()`. The test's intent (each migration recorded
exactly once) is unchanged. No other Phase 0/1/2 test was modified.

## 9. Live testnet results

Setup:

- **Server:** `testnet.zec.rocks:443` (lightwalletd v0.5.4, chain `test`, branch `77190ad9` = NU7).
- **Payout wallet `treasury`:** funded from the Phase 0 wallet `alice` with 4,000,000 zats (tx `98cb28bd…1dbc4cd3`, Ironwood→Ironwood, mined at 4,499,323).
- **Contributor wallet `carol`:** created with the Phase 0 CLI (shielded-only UA).
- **Confirmation policy:** 10.

Evidence: [`payout-wallet/evidence/`](../payout-wallet/evidence/).

### Run `p3run2`: PASS

| Scenario | What was done | Observed |
|---|---|---|
| **A** normal payout | claim 0.0015 ZEC → quote (fee 10,000 from a real proposal) → pay | `submitted` in 44 s, tx `52682f9a…3440d9`; claim stayed `reserved` (unpaid) while unconfirmed |
| A duplicate | the same request again | `200`, `duplicate: true`, same payout id; wallet not called |
| A crash | payroll-api **hard-killed** while tracking confirmations, restarted 20 s later | worker resumed; mined at 4,499,411 → `confirmed` at 10 confirmations; claim `settled` with the txid; a repeat request after confirmation → `duplicate`, `confirmed` |
| **B** interrupted submission | payroll-api **hard-killed** the instant the payout row existed (`reserved`, wallet call in flight) | client got 503 (outcome unknown, safe to retry). The wallet finished and broadcast `e804851e…9065f6` after its caller died. The restarted payroll-api waited out the dead process's lease, fenced/looked up the id and adopted the transaction → `submitted` → `confirmed` (4,499,429) |
| **C** wallet crash between build and broadcast | fault-injection build (`--features fault-injection`), `WALLET_FAULT=crash_after_build` | wallet exited (code 86) after persisting the intent as `built` (`e5f4d117…c6e71d`, 0 broadcast attempts); lightwalletd: "transaction not found"; payout → `requires_reconciliation`. Normal wallet restarted → recovery rebroadcast → `submitted` (10 s) → mined at 4,499,451 → `confirmed` |

Verification after the run:

* `carol`'s own wallet (Phase 0 CLI `sync`/`history`): each of the three payouts
  received **exactly once**, 150,000 zats, memo `hushush payout <payout id>`;
  balance 300,000 → 750,000 (+450,000).
* Payout wallet: 3,680,000 → 3,200,000 = 3 × (150,000 + 10,000 fee); exactly one
  transaction per payout id.
* Server-rendered pages: the contributor's stream page shows "Paid · payout
  confirmed"; the owner's organization page shows the payout wallet card.
* The payroll database (all 56 text columns) contains **no destination
  address**. `payout_events` histories: A and B `reserved > submitted >
  confirmed`; C `reserved > requires_reconciliation > submitted > confirmed`.
* Service logs contain no addresses, tokens or key material.

**Independent observer view.** This is keyless decoding of the bytes served by
lightwalletd, and it is identical for all three payouts:

- v6 transaction format, NU7 branch;
- 0 transparent inputs or outputs, 0 Sapling, 0 Orchard;
- **2 Ironwood actions**, with Ironwood value balance +10,000 (= the public fee);
- fully shielded.

Amount, sender, recipient and memo are not visible. Files: `live-p3run2-observe-*.json`.

What went wrong during the run:

* **The host was suspended overnight** while C had 2/10 confirmations. After
  wake-up, the script's wall-clock timeout fired and it reported FAIL. I then
  resumed the run (`RESUME=1`, same database and starting balances): all three
  payouts were confirmed and the verification above passed. The resumed
  verification used a wallet binary with one later fix (below); scenarios A–C
  themselves ran on the build before that fix.
* **A first resume reported page HTTP 500s.** A stale `next start` from the
  interrupted run still held the port with an old service token. The script
  now refuses busy ports, kills by port on exit, and asserts the page checks;
  the first version did not assert them. The rerun passed.

### Run `p3run1`: scenarios A and B passed; C exposed a bug

A (tx `31ca0061…3eb52a50`) and B (tx `ef344ccd…6a4721b4`) passed as in run 2.

In C, payroll-api's first wallet call after the wallet process was hard-killed
and replaced failed instantly ("Network Error"). No payout was created, and the
script hung waiting. A stale pooled keep-alive connection is the likely cause,
but it isn't proven: a unit test with a cleanly closing server does not
reproduce it.

Fixes:

- no connection reuse for wallet calls;
- one retry for read-only wallet calls;
- the full transport error text in logs.

Run 2 then passed C.

One more fix landed after run 2. The wallet read the chain tip before the
transaction, so a transaction mined in between could briefly report 0
confirmations. That was harmless, since 0 is never ≥ N; the tip is now at
least the mined height.

All five run-1/run-2 payouts to `carol` were single payments, and none was
duplicated: carol's total went 0 → 300,000 after run 1 → 750,000 after run 2.

## 10. Remaining production blockers

1. **Testnet-only crates.** The wallet stack is pinned to librustzcash
   pre-releases (Phase 0 §4), and the network is fixed to testnet at compile
   time. Mainnet needs stable NU7-capable releases, a reviewed dependency
   update and a deliberate mainnet build. Do not use mainnet funds.
2. **Key custody.** The spending key is an age-encrypted mnemonic plus an
   identity file, on the same host as the service. Production needs a KMS/HSM
   or an offline PCZT signer: the service would propose and prove, and the
   signer would sign.
3. **Own infrastructure.** Every live step used one public lightwalletd. Run
   your own Zebra + lightwalletd and verify against a second source. The
   public operator sees IP, timing and watched txids.
4. **One payout wallet for all enabled organizations.** There is no
   per-organization treasury or accounting segregation. `PAYOUT_ENABLED_ORGS`
   is an allow-list, and `*` is for testnet/dev only. Production needs a
   wallet (or account) per employer, with its own balance checks.
5. **Note management.** Each payout spends notes whose change needs 3
   confirmations, so back-to-back payouts from a single-note wallet have to
   wait (quotes say so). Between quote and build, another payout can consume
   the funds; that payout then fails safely and releases its claim. Production
   needs note splitting and a reservation of wallet funds per in-flight payout.
6. **Operator tooling.** There is no UI, CLI or alerting for payouts stuck in
   `requires_reconciliation`. They are retried automatically and are visible in
   logs, the API and `payout_events`. An interrupted submission takes at least
   the submit lease (timeout + 120 s) to resolve.
7. **Wallet service hardening.** It runs as a single instance per wallet,
   over plain HTTP on loopback. Anything multi-host needs mTLS. The wallet
   directory needs encrypted, tested backups, because the wallet DB and the
   intent DB hold addresses and history.
8. **Product and policy decisions:**
   - confirmation depth for mainnet;
   - per-payout caps;
   - minimum payout relative to the fee;
   - whether the employer approves payouts;
   - whether to delay or batch broadcasts to weaken timing correlation. No
     batching was built, as instructed.
9. **Still open from Phase 2.** Password reset, IP rate limiting and a script
   CSP. The payout UI was checked through server-rendered HTML and HTTP tests,
   not in a real browser; a manual browser pass on desktop and mobile is still
   recommended.
10. **Compliance.** Payroll tax, reporting and KYC obligations are out of scope
    and unaddressed.

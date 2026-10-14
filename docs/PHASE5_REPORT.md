# hushush Phase 5 — MVP flow redesign

Date: 2026-10-14 · Network: Zcash **testnet** only · Spec: [`PHASE5_FLOW_SPEC.md`](PHASE5_FLOW_SPEC.md)

## 0. Summary

The Phase 2/3 flow (e-mail + password, organizations with roles, invitations,
separate "claim" and "pay out" steps) is replaced by the spec's MVP flow:

* **Employer:** passkey sign-up → company name → **New stream** (worker name,
  total, start, end) → **worker link** (copy / QR) → home with the treasury,
  the streams and the payments.
* **Worker:** opens the link → sees "*Company* is paying you *X* ZEC from … to
  …" → signs up or signs in with a passkey → the stream binds to them →
  sets the payout address **once** (then locked) → **Withdraw** at each 20%
  milestone, and the remainder after the end.

Employer and worker accounts have a fixed kind and never share screens. An
employer who opens a worker link in their own browser is told to send it on;
they get no worker UI. This is the fix for the rehearsal's shared-tab
confusion: the problem goes away without asking anyone to use incognito.

The money path did not change: `payout-wallet/`, the payout state machine,
`decide()`, the accrual math and the exactly-once invariants are reused as they
were. Withdraw is a new engine operation that reserves a claim and records its
payout **in one transaction under the stream's row lock**. The unchanged
Phase 3 machinery then submits and confirms it.

| Suite | Before | After | Notes |
|---|---|---|---|
| `payroll` (SQLite **and** PostgreSQL; MOCK wallet/chain for payouts) | 101 | **128** | all 101 unchanged and passing; +3 unit, +24 flow scenarios (12 × 2 databases) |
| `payroll-api` | 3 | **5** | +2 address-sealing tests |
| `payout-wallet` | 8 | **8** | code unchanged |
| `web` (real payroll-api + PostgreSQL, MOCK wallet, SOFTWARE passkey) | 56 | **42** | 53 tests of removed flows replaced, mapped one by one in §6; 3 mail tests unchanged |
| `web/scripts/smoke.mjs` over HTTP against `next start` (MOCK wallet) | 31 | **31/31** | rewritten for the new flow |
| **Live testnet** (`live_flow.sh`, real wallet, real chain) | — | **PASS** (`p5run1`, §7) | 2 withdrawals, 6 concurrent taps → 1, exactly once |

## 1. Decisions

The spec's three open points were settled with the product owner at the start
of the build. Three more came up while planning:

| # | Topic | Decision |
|---|---|---|
| 1 | Treasury per employer | **(a) One shared demo treasury**, gated by `PAYOUT_ENABLED_ORGS` (`*` on testnet). The UI labels it "Shared testnet demo treasury" and explains that the balance isn't the employer's alone |
| 2 | Preview before sign-up | **Yes, minimal**: company, worker name, total, start and end only. Balances, addresses and payouts appear only after the stream binds |
| 3 | Second passkey | **No**: exactly one passkey per account (`passkeys.user_id UNIQUE`). Losing the device loses the account; acceptable on testnet (§5) |
| 4 | Storing the locked address | **Sealed at rest**: XChaCha20-Poly1305 under `PAYOUT_ADDRESS_KEY`, held only by payroll-api, with the stream id as associated data. This reverses the Phase 3 "address never stored" decision on purpose. On-demand withdrawals need the address, and the database alone still reveals none |
| 5 | Web tests of removed flows | **Replaced**, each mapped to its replacement (§6). Rust tests unchanged |
| 6 | Worker joins after the start date | **Accrual counts from the agreed start date.** The employer can cancel a stream nobody has joined, and nothing is owed on it |

## 2. What changed

### Engine (`payroll/`) — added, nothing removed

| New concept | Implementation |
|---|---|
| Employer account = company | the owner of exactly one organization (web enforces "one"; engine unchanged) |
| Stream with total + end | `stream_offers` (migration SQLite `0003_flow`, PostgreSQL `0004_flow`): worker name, `total_zats`, `start_at`, `end_at`, SHA-256 of the link token. When the offer is bound, a normal `payroll_streams` row is created with **`rate_zats = total`, `rate_period_secs = end − start`**. The unchanged accrual formula `floor(total × active / duration)` then reaches **exactly** the total at the end date, with no new rounding |
| Worker joins via link | `bind_offer`: in one transaction, lock the offer, add the worker as a member, create the stream with the offer's terms and mark the offer bound. Repeating it as the same worker returns the stream; for anyone else it is a conflict. Employers can't bind their own link |
| Locked address | `stream_payout_addresses`: `(stream_id PK, set_by, address_fingerprint, address_sealed)`. Triggers make it insert-only, by the stream's recipient only. No update, no delete, on either database |
| Withdraw | `withdraw()`, see §3 |
| Guard rails | Fixed-total streams can't be paused or resumed (that would make the total unreachable). They can't be claimed through the old claim path either, which would bypass the 20% rule and the address lock. Ordinary Phase 1–4 streams keep working as before (tested) |

Read models for the screens: `employer_streams`, `worker_streams` and
`flow_stream`. These return a `FlowStream` with the listing, the offer, the
payouts and the address. The address is included **only for the stream's
recipient**; managers see only `address_locked`.

### payroll-api

* New endpoints:
  - `POST /v1/orgs/{org}/offers` and `GET /v1/orgs/{org}/home`
  - `POST /v1/offers/{id}/cancel|relink`
  - `GET /v1/links/{hash}` (service token only, for the pre-sign-in preview) and `POST /v1/links/{hash}/bind`
  - `GET /v1/me/flow-streams` and `GET /v1/flow-streams/{id}`
  - `POST /v1/flow-streams/{id}/address` and `POST /v1/flow-streams/{id}/withdraw`
* `seal.rs`: address sealing; `PAYOUT_ADDRESS_KEY` is required whenever `WALLET_URL` is set.
* **Setting the address:** only the stream's worker reaches the wallet, which validates it with its real `payout_recipient` policy (testnet UA with an Orchard receiver). The address is then sealed and stored. It is never logged.
* **Withdraw:** checks the stream, opens the sealed address and quotes the wallet for exactly the available amount. The engine then records the withdrawal and `submit_payout` sends it (§3).
* Old endpoints (claims, quote, pay) stay so their tests keep running. The web app no longer calls them, and the engine refuses them for fixed-total streams.

### Web (`web/`)

* **Removed:**
  - password sign-up and login, e-mail verification, resend
  - invitations, members and roles
  - organizations pages, claim, quote, release and pay routes and screens
  - tables `auth_credentials`, `email_verification_tokens`, `login_attempts`, `invitations` (migration `0003_passkeys`)
* **Added:**
  - passkeys (`@simplewebauthn/server` 14.0.3, `@simplewebauthn/browser` 14.0.0)
  - `accounts(kind)`, `passkeys`, `webauthn_challenges`
  - routes: `/api/auth/passkey/{register,login}/{options,verify}`, `/api/company`, `/api/streams`, `/api/offers/{id}/{link,cancel}`, `/api/join/{token}`, `/api/streams/{id}` (read-only), `/api/streams/{id}/{address,withdraw,end}`
  - pages: `/`, `/signup`, `/login`, `/join/{token}`, `/app` (employer or worker home), `/app/streams/new`, `/app/streams/{id}`
  - QR codes via `qrcode` 1.5.4
* **Worker screens:**
  - live balance, ticking every second: the server's formula re-evaluated in the browser with BigInt, never below the server's figure
  - progress bar with 20/40/60/80% marks
  - payout address with a confirm-before-lock step, shown with 🔒 afterwards
  - Withdraw button with one idempotency key per attempt
  - status: *Sending… (≈1 min)* → *Confirming n/10* (with minutes left) → *Paid ✓* with txid
* **Employer screens:**
  - treasury card: spendable, funding address, shared-treasury label
  - streams table: worker, total, earned, withdrawn, status, end date
  - "waiting for the worker" list with *New worker link* / *Cancel*
  - payments with txids
  - stream detail with progress, payouts and *End stream*
* **Kept:** the session design (hashed tokens, HttpOnly, SameSite=Lax, 30-day absolute and 7-day idle expiry), the Origin + JSON CSRF checks, RLS on every table. SMTP and the dev outbox stay for notifications (unused by auth).

## 3. Exactly-once for Withdraw

A withdrawal is one claim plus one payout. `withdraw()` runs in one database
transaction and holds the stream's row lock (`SELECT … FOR UPDATE` on
PostgreSQL, `BEGIN IMMEDIATE` on SQLite). In order:

1. **Same idempotency key seen before:** return that withdrawal's payout, whatever its state.
2. **The stream has an unfinished payout** (`reserved`, `submitted` or `requires_reconciliation`): return it. **At most one withdrawal per stream is ever in flight**, so a refresh, a second tab or a new key cannot start another.
3. Only then are the terms validated. A repeat sends "everything available" = 0 while the first is reserved, and must still get its payout back; a regression test caught this case during the build. After that the engine checks:
   - the address lock: the fingerprint of the address payroll-api will pay must equal the stored one
   - the org allow-list
   - the amount ≤ available
   - the milestone rule: amount × 5 ≥ total, or the stream has ended
4. Insert the claim, then the `reserved` payout, leased to the request. The Phase 3 schema rules still apply: one payout per claim (`UNIQUE`), created for a reserved claim by its claimant for its exact amount (trigger), terms immutable, txid write-once.

After commit, `submit_payout` and the background reconciler take over,
unchanged from Phase 3/4. Ambiguity goes to `requires_reconciliation`, never to
a resubmission. A failure releases the claim, so the worker can withdraw again.
"Paid" appears only at `confirmed`.

The "never pay twice" and "never release funds that may already have been
broadcast" guarantees therefore rest on the same code that Phase 4 verified
live. Phase 5 adds the guarantee that no second withdrawal starts while one is
unresolved.

## 4. The 20% rule and totals

* `withdrawal_unlocked(total, amount, ended)`: `amount > 0 && (ended || amount × 5 ≥ total)`, in i128.
* The UI's milestone is `ceil(total / 5)`. A total of 1,001 zats needs 201.
* Withdraw pays *everything available*, so waiting past several milestones gives one larger withdrawal.
* After the end, natural or early, any remainder unlocks. No dust gets stuck.
* Tests:
  - totals not divisible by the duration reach the total exactly (payroll `total_is_exactly_reached`, web *the total is reached exactly at the end date*)
  - a worker who joins halfway sees exactly half (payroll `late_join_accrues_from_start`)

## 5. Security and privacy notes

* **Passkeys.**
  - Discoverable credentials are required, and so is user verification (Face ID, fingerprint, Windows Hello or device PIN). Attestation `none`.
  - The Relying Party ID defaults to `APP_URL`'s host (`localhost` in development, the Vercel host in production; override with `WEBAUTHN_RP_ID`). **Changing the production host later invalidates every passkey.**
  - Challenges are server-side, valid for 5 minutes and single-use. They are reached through a random handle in an HttpOnly, SameSite=Strict cookie, and only the handle's SHA-256 is stored.
  - The signature counter is updated compare-and-set, so a cloned or replayed assertion loses.
* **No password-guessing surface.** Every sign-in needs a fresh challenge and a signature by a registered key, so the Phase 2 login throttle has nothing left to protect. The unauthenticated ceremony endpoints do write challenge rows; expired rows are deleted opportunistically. A per-IP rate limit (e.g. at Vercel) is advisable before public exposure.
* **One passkey, no recovery** (decision 3). Synced passkeys (iCloud Keychain, Google Password Manager) survive device loss; device-bound ones don't. On testnet this costs nothing. Before real money, add a second passkey or another recovery path.
* **Worker links** are 32 random bytes and stored only as SHA-256. They are single-use once bound, and the employer can reissue them before binding. The pre-sign-in preview shows the terms only. For a used link, nobody learns more: the bound worker is redirected to their stream (bind is idempotent for them) and everyone else gets "already used".
* **Payout address.**
  - The worker sets it once, and only the worker can reach the wallet with it, so there is no probing by employers (SEC-1 still holds).
  - It is sealed at rest and returned only to the worker. Employers see "set and locked".
  - Each payout still carries its own per-payout salted fingerprint, as in Phase 3. The stream's lock fingerprint is salted with the stream id.
  - Losing `PAYOUT_ADDRESS_KEY` makes stored addresses unreadable. The worker's view and withdrawals then fail with an internal error until it is restored; nothing is paid elsewhere.
* **Account kinds.** Every web route checks the kind (`wrong_account` 403), and the engine independently checks roles and ownership.

## 6. Tests: what replaced the removed web tests

These Rust suites are **unchanged** and pass: payroll 101 (engine 29, backends 17 × both databases, payouts 41, unit 14), payroll-api 3 and payout-wallet 8. The web tests of removed features are replaced as follows. The replacement keeps the protection, or the "n/a" explains why there is nothing left to protect.

| Old web test (Phase 2–4) | What it protected | Phase 5 replacement |
|---|---|---|
| auth: password hashing (scrypt + salt) | no plaintext secrets | auth: *stores only the public key and hashes of challenge handles* (there are no passwords) |
| auth: verified e-mail required; verification links single-use | proof of control before access | auth: *challenges are single-use and need their cookie*; *rejects … without user verification* |
| auth: stores only hashes of passwords and tokens | DB leak exposes no secrets | auth: *stores only the public key and hashes of challenge handles*; worker links stored as SHA-256 (flow: *only its hash is stored*) |
| auth: wrong password / unknown e-mail same error | no account enumeration | auth: *rejects unknown passkeys, other origins and replayed or cloned counters* (discoverable credentials: there is no username to enumerate) |
| auth: doesn't reveal registered e-mail | no enumeration | n/a — no e-mail; *one passkey belongs to one account* covers duplicates |
| auth: password policy and input validation | input hygiene | auth: *validates the name and account kind* |
| auth: throttles repeated failed sign-ins | password guessing | n/a — no guessable secret; each attempt needs a fresh server challenge and a signature by the registered key (see report §5) |
| auth: logout invalidates server-side | session revocation | auth: *logout invalidates the session server-side* (same) |
| auth: absolute and idle expiry | session lifetime | auth: *expires sessions after the absolute lifetime and after idling* (same) |
| auth: forged / missing cookies | auth bypass | auth: *rejects forged and missing session cookies* (same) |
| auth: blocks cross-site / origin-less changes | CSRF | auth: *blocks cross-site and origin-less state changes*; flow: withdraw from another origin → 403 |
| auth: requires JSON bodies | CSRF via forms | auth: *requires JSON bodies* (same) |
| claims: partial claims reserve, labelled unpaid | broadcast ≠ paid | flow: *unlocks at the 20% milestone … paid only once confirmed* (claimed stays 0 until confirmed) |
| claims: full claim reserves exactly available | exact amounts | flow: same test (pays everything available); payroll `milestone_withdrawals` |
| claims: rejects above available | over-withdrawal | payroll `milestone_withdrawals` (InsufficientAvailable); flow: early withdraw refused |
| claims: rejects malformed amounts | input hygiene | flow: *validates the terms* (total: 0, negative, > 8 decimals) |
| claims: release makes funds available once | reservation correctness | flow: *a definite failure … returns the amount; the worker can withdraw again* (manual release no longer exists) |
| claims: possible on paused/ended streams | no stuck wages | flow: *after the stream ends, any remainder can be withdrawn, even below 20%* |
| claims: same idempotency key → original | exactly-once | flow: *double taps, retries and refreshes pay once* |
| claims: key reuse for a different amount rejected | idempotency integrity | payroll `withdraw_is_exactly_once` (old key returns the original payout, even after later withdrawals) |
| claims: concurrent retries → one claim | exactly-once | flow: *double taps …* (12 concurrent); payroll `withdraw_is_exactly_once` (8 threads, both DBs) |
| claims: parallel claims never exceed earned | ledger invariant | payroll `withdraw_is_exactly_once`; DB triggers unchanged (`claims_guard_insert`) |
| claims: payroll-api has no settle endpoint | no manual "paid" | flow: *has no way to mark a withdrawal paid …* |
| claims: payroll-api needs the service token | service boundary | flow: *rejects calls without the service token* |
| payouts: quote → send → submitted → confirmed | payout lifecycle | flow: *unlocks at the 20% milestone … paid only once confirmed* |
| payouts: employer sees payout + wallet summary | employer visibility | flow: *home lists streams …, payments with txids, and the shared treasury* |
| payouts: only the claimant can quote/pay | authorization | flow: *only the worker can withdraw, and only with a locked address* |
| payouts: rejects malformed / non-payable addresses | recipient validation | flow: *only the worker sets it, it is validated, and then it is locked for good* |
| payouts: wallet can't cover → refused, claim stays | treasury shortfall | flow: *refuses when the treasury can't cover amount + fee; nothing is reserved* |
| payouts: repeats same payout; different address refused | exactly-once + address integrity | flow: *double taps …*; address lock (409 on change) |
| payouts: only claimant repeats (no address probing) | SEC-1 | flow: employer `setAddress` → 403 before any wallet call; employer never sees the address (*is stored sealed, and never shown to the employer*) |
| payouts: 10 concurrent pays → one tx | exactly-once | flow: *double taps …* asserts `txs_built + 1` |
| payouts: lost wallet response reconciled, once | EO crash safety | flow: *a lost wallet response is reconciled without paying twice* |
| payouts: definite failure releases claim | no stuck funds | flow: *a definite failure sends nothing …* |
| payouts: claim with payout can't be cancelled | lock to payout | payroll `legacy_paths_are_closed` (claim path refused on fixed-total streams); engine rule unchanged and still tested in payroll `claim_locked_to_payout` |
| permissions: contributors can't manage payroll | role separation | auth: *workers can't create companies or streams …*; flow: worker can't end (403) |
| permissions: admins' limited powers | roles | n/a — no admin role in the MVP (one employer account = one company) |
| permissions: only recipient can claim | authorization | flow: *only the worker can withdraw …* |
| permissions: last owner / members with streams | membership integrity | n/a — no member management; engine rules unchanged and still tested (payroll engine/backends suites) |
| permissions: invitations accepted by invitee, once | single-use invite | flow: *the worker who joins gets the stream; the link then works for nobody else* |
| permissions: revoked invitations / duplicates | revocation | flow: *before anyone joins, the employer can issue a new link or cancel* |
| permissions: re-checks inviter authority | stale authority | flow: relink/cancel by another company → 404; payroll `cancel_and_relink` |
| permissions: org B can't see/touch org A | isolation | flow: *another company sees and touches nothing*; *the read-only stream route answers its worker and employer only* |
| permissions: can't pay outsiders / release others' claims | isolation | flow: other worker → 404 on stream, address, withdraw |
| permissions: malformed ids ignored | robustness | flow: *malformed ids are plain 404s* |
| streams: exact zatoshi rate | money exactness | flow: *the total is reached exactly at the end date*; payroll `total_is_exactly_reached` |
| streams: rejects backdated / bad end / bad amounts / periods | validation | flow: *validates the terms* |
| streams: future-dated accrues nothing before start | accrual | flow: *a future start accrues nothing until it starts* |
| streams: end date caps accrual | accrual | flow: *the total is reached exactly at the end date* |
| streams: pause stops accrual / resume continues | lifecycle | replaced by a rule: fixed-total streams can't be paused (flow: *… can't be paused*; payroll `legacy_paths_are_closed`). Pause/resume itself is still tested in payroll backends/engine |
| streams: ending is permanent, preserves wages | lifecycle | flow: *ending stops accrual and keeps what was earned …* |
| streams: full history in order | audit trail | engine `stream_events` unchanged and tested in payroll suites; not shown in the Phase 5 UI |
| streams: rate change = end + new stream | terms immutable | address change = end + new stream (spec); stream/offer terms immutable by trigger (payroll `offer_schema_guards`) |
| mail ×3 (dev outbox refused in production) | SEC-2 | unchanged, still run |

New web tests beyond the mapping cover:
* the account-kind split (a worker can't create a company, an employer can't own two)
* the employer opening their own worker link
* reissuing and cancelling links
* the sealed-at-rest address
* the read-only stream route
* the service boundary, and the old claim path being refused for fixed-total streams

New payroll scenarios (both databases) cover offers, binding, cancel and
relink, exact totals, late joins, the address lock (including raw-SQL attempts
to change, delete or forge it), milestones, exactly-once (8 concurrent
threads), retry after failure, authorization, the closed legacy paths and the
offer schema guards.

## 7. Live testnet run

### Run `p5run1`: PASS (2026-10-14, 15:58–16:12 UTC)

Evidence:
* [`payout-wallet/evidence/live-p5run1.log`](../payout-wallet/evidence/live-p5run1.log)
* `live-p5run1.services.log` (wallet, payroll-api, web)
* script: [`payout-wallet/scripts/live_flow.sh`](../payout-wallet/scripts/live_flow.sh)

Setup:
* **Live:** `next start`, payroll-api, PostgreSQL, the payout wallet on `treasury`, lightwalletd `testnet.zec.rocks` (NU7), and real TAZ. 10 confirmations.
* **Not live:** the passkey authenticator, which was software (§9).
* Stream: 0.001 ZEC over 10 minutes. The worker's address is `carol`'s Unified Address.

| Step | Result |
|---|---|
| Employer signs up (passkey), names the company, creates the stream, gets the link | ✔ |
| Worker opens the link (preview), signs up (passkey), joins | ✔ |
| An invalid address is refused **by the real wallet** | ✔ 422 "not a valid Zcash address for this network" |
| carol's address locked; a change to alice's address refused | ✔ 200 then 409 "locked" |
| Withdraw before 20% | ✔ refused (409) |
| At 20% (20,333 zats available): **6 concurrent taps** (3 sharing a key, 3 with new keys) | ✔ **1 withdrawal created, 5 got the same one back**; 20,500 zats, tx `d3b41b63…5f9b0d` |
| Employer ends the stream early: earned 32,833, remainder 12,333 (< 20%) | ✔ |
| Remainder Withdraw while #1 is unfinished | ✔ returned #1 (no second withdrawal) |
| #1 confirmed (block 4,501,874), then remainder withdrawn (2 taps → 1 withdrawal), confirmed (block 4,501,889) | ✔ tx `f487d0cc…0bad76d3` |
| Stream: earned = withdrawn = 32,833, available 0, reserved 0 | ✔ |
| carol's own wallet: each withdrawal received **exactly once**, with its payout memo | ✔ 20,500 and 12,333 zats |
| Treasury: exactly **one** transaction per payout memo | ✔ 1 and 1 |
| Accounting | ✔ carol +32,833 (expected 32,833); treasury −52,833 = withdrawals + 2 × 10,000 fee |
| Pages: worker sees *Paid ✓*, txids and the locked address; employer sees payments with txids and the shared-treasury label | ✔ |
| Employer never sees the worker's address | ✔ re-checked after the run, against the run's own database and sessions: the employer home, stream page and `/api/streams/:id` contain no part of carol's 182-character address (`address: null`, `address_locked: true`). The run's own check for this was written wrongly (always true); it is fixed in `live-flow.mjs` for future runs |
| The address appears in no log (run log, wallet / payroll-api / web logs) | ✔ 0 occurrences |

Treasury after the run: 2,027,167 zats (0.0203 TAZ). The rehearsal didn't
need any new funding.

## 8. Deploying (Vercel + Railway + Supabase)

* **Database.** The Supabase project still holds Phase 2-style tables and rehearsal accounts. **Reset it** (drop and recreate the `public` schema, then start payroll-api, then run `npm run db:migrate`). Migration `0003_passkeys` would also work on it, but the rehearsal accounts could never sign in again.
* **Railway (payroll-api):** add `PAYOUT_ADDRESS_KEY` (`openssl rand -hex 32`). Keep it as safe as the wallet token: the stored addresses depend on it. The remaining variables are as in `PHASE3_PAYOUTS.md` §6.
* **Vercel (web):** `APP_URL` must be the final production origin before the first real sign-up, because passkeys are bound to its host. `WEBAUTHN_RP_ID` only if it must differ. Add `@simplewebauthn/*` and `qrcode` (already in `package.json`).
* **SMTP** is no longer needed for sign-in. Production without `SMTP_URL` only fails if mail is actually sent, and nothing sends mail in Phase 5.

## 9. Known limitations

* **Shared demo treasury** (decision 1): every company pays from one testnet wallet. Per-company wallets remain open.
* **One passkey per account, no recovery** (decision 3).
* **Ticker refresh.** The worker's balance ticks client-side. The Withdraw button follows the server and enables within one refresh (≤ 15 s) of crossing a milestone.
* **No notifications.** E-mail is not sent for new streams or payouts.
* **The Phase 3/4 live script no longer runs.** `live_e2e.sh` and `live-payout.mjs` drove the removed web routes. They are kept as the record of the p3/p4 evidence, and the wallet and payroll-api behaviour they exercised is unchanged. `live_flow.sh` is the Phase 5 live run.
* **Not checked in a real browser with a real platform authenticator.** The passkey ceremonies were tested with a software authenticator against the real verifier. The browser side (`@simplewebauthn/browser`, Face ID / Windows Hello prompts, QR scanning) still needs a manual pass on a phone and a laptop.

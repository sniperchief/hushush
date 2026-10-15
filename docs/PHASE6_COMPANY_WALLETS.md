# Phase 6: per-company payout wallets (testnet)

**Built:** 15 Oct 2026. **Network:** Zcash testnet only.
**Mainnet alternative:** [MAINNET_OPTION_B_SELF_CUSTODY.md](MAINNET_OPTION_B_SELF_CUSTODY.md).

## 1. What changed

Before Phase 6, one shared wallet (the *treasury*) paid every company's
workers. Employers never put money in, and they all shared one balance.

Now **every company pays from its own payout account**:

- **A company account.** Each company has an account in the payout wallet with
  its own funding address and balance.
- **Workers are paid from it.** Withdrawals, and their network fees, are paid
  from that company's account **only**. If it can't cover a withdrawal, the
  withdrawal is refused before anything is reserved or built, with a clear
  message.
- **Funding it.** The employer funds the account by sending TAZ to its funding
  address from any testnet wallet.
- **Get test TAZ.** A faucet button on the dashboard sends a small top-up from
  the treasury, so testing stays one click.
- **Dashboard figures:**
  - the company wallet's spendable and settling balance
  - "Workers can withdraw now"
  - "Still to fund for every stream"
  - a warning when the wallet can't cover what workers can withdraw now

## 2. Design

### One wallet, many accounts

The payout wallet (`payout-wallet/`) still runs **one wallet DB from one
seed**. Company accounts are additional ZIP 32 accounts of that seed:

| ZIP 32 account | Role |
|---|---|
| 0 | **Treasury**: the shared demo funds and the faucet source |
| 1, 2, 3… | One per company, created on first use |

Why this design:

- **No new key material.** Every company account derives from the existing
  age-encrypted mnemonic, so the existing backup restores all of them.
  Restore by importing accounts 1…n in order, as the library recommends.
- **One scan for everyone.** `zcash_client_sqlite` scans the chain once and
  trial-decrypts for every account. The cost grows with the number of
  accounts, which is fine at testnet scale.
- **Strict separation by construction.** A payment proposal names its
  account, so input notes, the fee and change all stay inside that account.
  One company's withdrawal can never spend another company's money.
- **Crash-safe creation.**
  - A new account is created with a birthday at the current chain tip (a new
    address can't have received anything earlier, so nothing is rescanned).
  - It is named `org:<organization id>` in the wallet DB and recorded in the
    service's registry (`wallet_accounts`).
  - If the service crashes between creating and registering an account, the
    next request finds it by name instead of creating a second one.

### Which account pays a payment

- **Every payment intent records its paying account** (`payment_intents.account`).
  Older rows are `NULL`, meaning the treasury. The column is immutable,
  enforced by a trigger.
- **A repeated `payment_id` must name the same account.** If it doesn't, it is
  rejected as an idempotency conflict, like a changed address or amount.
- **payroll-api passes the organization id** when it quotes and submits a
  withdrawal (`OrgWallet`).
- **The reconciler never submits, it only looks up or fences by payment id,**
  so it can't pay from the wrong account.
- **The old Phase 3 claims path** (no longer used by the web app) still pays from
  the treasury.

### Faucet (testnet only)

A faucet top-up is an ordinary payment from the treasury to the company's
funding address, through the same exactly-once machinery: intent first, one
transaction per id, rebroadcast and recovery.

| Setting (payout-wallet env) | Default | Meaning |
|---|---|---|
| `FAUCET_ZATS` | `1000000` (0.01 TAZ) | amount per top-up; `0` turns the faucet off |
| `FAUCET_COOLDOWN_SECS` | `86400` | minimum time between a company's top-ups |
| `FAUCET_MAX_GRANTS` | `3` | top-ups per company, ever |

Grants are logged in `faucet_grants`. A top-up that **failed**, for example
because the treasury ran low, sent nothing, so it doesn't count against the
limits. A retry with the same idempotency key returns the same top-up.

A top-up becomes spendable in the company account after the wallet's
confirmation policy is met, typically a few minutes on testnet. The dashboard
shows it as "settling" until then.

## 3. Interfaces

### Payout wallet (`payout-wallet`, bearer token, only payroll-api calls it)

| Route | Purpose |
|---|---|
| `POST /v1/accounts` `{key}` | create or find the account for `key` (organization id); returns the funding address |
| `GET /v1/accounts/{key}` | that account's spendable/pending/total balance, funding address and faucet availability |
| `POST /v1/accounts/{key}/faucet` `{payment_id}` | top-up from the treasury (idempotent per payment id; `409 refused` when limits apply) |
| `POST /v1/quotes`, `POST /v1/payments` | now take an optional `account` (a key); absent means the treasury |

`GET /v1/status` still describes the treasury.

### payroll-api

| Route | Purpose |
|---|---|
| `GET /v1/orgs/{org}/payout-wallet` | the company's own account (created on first view), with `faucet` |
| `POST /v1/orgs/{org}/payout-wallet/faucet` `{idempotency_key}` | testnet top-up (company managers only) |
| `POST /v1/flow-streams/{id}/withdraw` | unchanged contract; now quoted and paid from the stream's company account |

### Web

| Route | Purpose |
|---|---|
| `GET /api/company/wallet` | the signed-in employer's company wallet (read-only) |
| `POST /api/company/faucet` `{idempotencyKey}` | "Get test TAZ" |

## 4. Existing companies and streams

- **Accounts.** A company created before Phase 6 gets its account the first time
  its dashboard (or a withdrawal) asks for it.
- **Withdrawals.** From then on its withdrawals come from that account, which
  starts empty: the employer funds it, or uses Get test TAZ.
- **In-flight payouts.** Payouts already in flight finish from the treasury
  (their intents record `NULL`).
- **The Phase 0 CLI** (`wallet-proof`) now treats a multi-account wallet's
  ZIP 32 account 0 as "the" account for `balance`, `send` and `address`. Its
  `history` covers the whole wallet.

## 5. Security notes

- **Custody.** On testnet, hushush holds the spending keys for every
  company account, derived from the treasury seed. That is option A and is
  acceptable on testnet. **It is not a mainnet design as is:** see the mainnet
  document for key management, licensing and the self-custody alternative.
- **Unlocking the seed.** It is unlocked only when building a transaction or
  creating an account, as before, and dropped immediately after.
- **Account keys** are opaque organization ids, validated as `[A-Za-z0-9_-]{1,64}`.
  Only payroll-api can call the wallet (bearer token, loopback by default).
- **Visibility.** The company wallet summary and the faucet are restricted to
  the company's payroll managers. Outsiders get "organization not found".

## 6. Tests

| Suite | Result |
|---|---|
| payout-wallet unit tests | 12 passed (4 new: registry, paying account, migration of an older intent store, faucet grant counting) |
| wallet-proof unit tests | 12 passed |
| payroll-api unit tests | 5 passed |
| web (vitest, real payroll-api + PostgreSQL + mock wallet) | 73 passed (5 new in `tests/company-wallet.test.ts`, 2 updated) |
| live testnet run (`payout-wallet/scripts/live_flow.sh`, `RUN_ID=p6run1`) | see §7 |

New web tests cover:
- each company gets its own wallet and funding address, not the treasury's
- withdrawals are paid from the company account, with the treasury untouched
- an unfunded company refuses the withdrawal, with nothing reserved or built
- the faucet: top-up, idempotent retry, cooldown refusal, and the refused
  withdrawal succeeding once funded
- only a company manager can top up

The mock wallet (`web/tests/support/mock-wallet.ts`) implements the new
account and faucet API with the same limits as the real wallet.

## 7. Live testnet run

`RUN_ID=p6run1 bash payout-wallet/scripts/live_flow.sh` runs the Phase 5 flow
end to end on the NU7 testnet, now with a company account:

- the company account is funded by the faucet
- the run waits until that top-up is spendable
- milestone and remainder withdrawals are paid from the company account
  (10 confirmations each)

Accounting checked:
- the treasury drops by exactly the top-up plus its fee
- the company account ends at top-up − withdrawals − their fees
- carol receives each withdrawal exactly once
- one transaction per payout memo

### p6run1: FAIL, a real bug found and fixed

- **The failure.** At the 20% milestone, six concurrent taps created exactly
  one withdrawal and one transaction (money-safe). But two of the five repeat
  taps got `409 insufficient_wallet_balance` instead of the in-flight
  withdrawal.
- **Cause.** A fresh company account holds a single note. The first
  withdrawal spent it, and its change (924,500 zats) was still confirming, so
  the wallet's quote for the late taps saw spendable 0 and refused before the
  engine could answer "duplicate". The treasury never showed this because it
  holds many notes.
- **Fix** (`payroll-api/src/flow.rs`). When a quote is insufficient, re-read
  the stream: if a withdrawal is already unfinished, answer with it (200,
  `duplicate: true`).
- **Regression test.** In `tests/company-wallet.test.ts`; it fails without
  the fix.
- **The run's one withdrawal.** Transaction `ef26f345…` (65,500 zats) went
  out from that run's company account.

Evidence: `payout-wallet/evidence/live-p6run1.log`.

### p6run2: PASS (15 Oct 2026, 15:35–15:56 UTC)

| Check | Result |
|---|---|
| Faucet top-up treasury → company account | 1,000,000 zats spendable after confirmations (one transaction carrying its memo) |
| 6 concurrent taps at 20% | one withdrawal, one transaction `41086baf…` (78,500 zats), the other five got it back (all 200) |
| Employer ends early → remainder | 11,166 zats in one transaction `434eb6b5…`, the second tap got it back |
| Confirmations | 10/10 at block 4,505,284 and 11/10 at block 4,505,299 |
| Stream | ended, earned = claimed = 89,666 zats, nothing reserved or available |
| Carol | received each withdrawal exactly once with its memo; balance +89,666 exactly |
| Treasury | 13,297,167 → 12,287,167: −1,010,000 = top-up + its fee only (no withdrawals) |
| Company account | 890,334 zats = 1,000,000 − 78,500 − 11,166 − 2 × 10,000 fees: exact |

The script stopped after the treasury check because `live-flow.mjs show` didn't
print `organization_id`. That's fixed now. The company-account check was then
run by hand against the real wallet (`GET /v1/accounts/{org}`): 890,334, a
match.

Evidence: `payout-wallet/evidence/live-p6run2.log`.

## 8. Not done / next

- **Withdrawing leftover funds.** There is no way yet to withdraw a company's
  leftover balance back to the employer. On testnet it can stay; for mainnet
  it's a must.
- **Mainnet custody** (option A hardening or option B): see the mainnet document.
- **Faucet capacity.** It is limited by the treasury's balance (about 0.14 TAZ
  at the start of Phase 6, so roughly a dozen top-ups). Top up the treasury
  from a testnet faucet on the NU7 chain when it runs low.

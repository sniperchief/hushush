# hushush Phase 0 — Shielded Payment Feasibility Report

Date: 2026-10-13 · Network: Zcash **testnet** only · Code: [`wallet-proof/`](../wallet-proof/)

> **Status: Phase 0 complete — live end-to-end shielded payment PASSED on the
> official (NU7) Zcash testnet on 2026-10-13.** Payment txid
> `e466d8713b0d9b6af40fb38dd2a547c5fc4eea1b3adc98fdfe000dc74b0898bb`
> (height 4,498,165): received funds → sent 150,000 zats with memo →
> confirmed → recipient received exact amount + memo → keyless observer
> view fully shielded → payer restored from mnemonic with identical balance.
> Evidence: [`wallet-proof/evidence/run3.console`](../wallet-proof/evidence/run3.console).

## 1. Stack selected

| Layer | Component | Version | Why |
|---|---|---|---|
| Wallet logic, note selection, tx construction | `zcash_client_backend` (ECC, librustzcash) | **=0.25.0-pre.1** | Official Rust wallet SDK; same core used by Zashi and the Android/iOS SDKs |
| Wallet persistence | `zcash_client_sqlite` | **=0.23.0-pre.1** | Official storage backend with schema migrations |
| Tx format / consensus | `zcash_primitives` / `zcash_protocol` | =0.31.0-pre.1 / =0.11.0-pre.0 | First release with the testnet NU7 activation (see §4) |
| Proving | `zcash_proofs` (bundled Sapling params), `orchard` 0.16.0 (Halo 2), `sapling-crypto` 0.9.0 | =0.31.0-pre.1 | No trusted-setup download; no custom crypto |
| Keys / addresses / ZIP 321 | `zcash_keys` =0.17.0-pre.1, `zcash_address` =0.14.0-pre.1, `zip321` =0.10.0-pre.1, `zip32` 0.3.0, `bip0039` 0.14.2 | | ZIP 32 HD derivation from BIP 39 seed |
| Seed-at-rest encryption | `age` 0.12.1 (X25519) | | Audited, standard; no custom crypto |
| Chain access | `lightwalletd` at `testnet.zec.rocks:443` (server v0.5.4), gRPC via `tonic` 0.14.6 + `rustls` 0.23 (ring) | | Public testnet infrastructure, TLS |
| Toolchain | Rust 1.99.0, MSVC (VS Build Tools 2022), Windows 10 | | |

Alternatives considered: **zcash-devtool** (ECC reference CLI — explicitly "not
production", and currently pinned to the pre-NU7 crates so it cannot build valid
testnet transactions today); **Zallet** (ECC full-node wallet, actively developed;
requires running a local Zebra node — a good Phase 1+ option); **zingolib** (third-party, also on librustzcash). Building directly
on `zcash_client_backend` gives the smallest, most production-relevant surface.

## 2. What works end to end

| Capability | Evidence | Real or mocked |
|---|---|---|
| Connect to testnet lightwalletd, verify chain = `test`, tip past NU7 | `live_server_is_testnet_and_current` | **LIVE** |
| Create wallet (24-word BIP 39, ZIP 32 account, birthday from tree state), shielded-only UA (Orchard+Sapling, no transparent receiver) | `init alice`, `init bob` on testnet | **LIVE** |
| Sync chain (Sapling, Orchard, Ironwood trees) | `sync alice`, `live_fresh_wallet_sync_and_persistence` | **LIVE** |
| Persist across restarts; resume sync; schema migration across library upgrade | `live_fresh_wallet_sync_and_persistence`; alice's DB created on 0.22 schema, migrated to 0.23-pre on open, same address | **LIVE** + offline |
| Keyless decoding of real current-format (v6 / NU7) testnet txs | `live_observer_decodes_recent_transactions` (31 txs decoded) | **LIVE** |
| Propose → prove → sign a shielded payment with memo; recipient decrypts exact amount + memo; stranger key learns nothing; sender balance correct after mining | `mocked_orchard_to_orchard_payment`, `mocked_ironwood_active_payment_from_ironwood_note`, `mocked_nu7_active_payment_uses_v6_nu7_format` | **MOCKED chain**, real proofs |
| Reject transparent / TEX / wrong-network recipients; reject any proposal touching the transparent pool | unit tests, `assert_fully_shielded` | offline |
| Insufficient funds and unconfirmed funds rejected | `mocked_insufficient_funds_is_rejected`, `mocked_unconfirmed_funds_not_spendable_under_default_policy` | **MOCKED** |
| Recovery: mnemonic → identical keys/UFVK/address; encrypted mnemonic + identity → spend authority; wrong seed rejected | `tests/recovery.rs` | offline |
| **Receive shielded funds** — 12,500,000 zats into Ironwood (funding tx `90b7d881…5b2643b8`, height 4,498,135), seen in mempool before mining via `pending` | `evidence/run3.console`, `history alice` | **LIVE** |
| **Build, prove, broadcast a shielded payment; node accepts v6 / NU7 tx** — 150,000 zats, fee 10,000, Ironwood→Ironwood, tx `e466d871…4b0898bb` | `scripts/testnet_e2e.sh` run3 | **LIVE** |
| **Confirmation tracking** — mined at 4,498,165; 3 confirmations ~2.5 min after broadcast | run3 | **LIVE** |
| **Recipient receipt** — bob's wallet: +150,000 exactly, memo `hushush phase0 testnet payout 20261013T142033Z` decrypted | run3, `history bob` | **LIVE** |
| **Payer accounting** — alice 12,500,000 → 12,340,000 (= −150,000 −10,000 fee) | run3 | **LIVE** |
| **Recovery from mnemonic on live chain** — fresh wallet restored at birthday 4,497,786 rescans to identical balance (12,340,000) and identical address | run3 | **LIVE** |

Test results (2026-10-13): unit 12/12, mocked 6/6, recovery 6/6, live 3/3 — all
pass; funded end-to-end script: **PASS** (run3).

Bugs found only by the live run (fixed): a 1 MiB Windows main-thread stack
overflowed during Halo 2 proving (CLI now runs on 64 MiB threads — tests had
masked this because test threads get larger stacks); a transient TLS drop from
the public lightwalletd killed run1 (script now retries); the `history` query
used a non-public view column.

## 3. What remains blocked or unverified, and why

1. **No independent third-party explorer on the NU7 chain was available.**
   zexplorer.app is on the stale fork (below); testnet.zcashexplorer.app
   returned 404. The observer evidence is our keyless decoder applied to the
   raw transaction bytes served by `testnet.zec.rocks`, independent of the
   wallet's keys and database, but sharing that one server.
2. **Single server.** Every live step used `testnet.zec.rocks`; the other
   public testnet lightwalletd endpoints probed were dead. Multi-server or
   self-hosted verification is a Phase 1 item.
3. **Scale untested:** one payee per tx, one payment. Batching, concurrent
   payouts and note management were not exercised (out of scope for Phase 0).
4. The first faucet funding (`0c3546cd…851370d`, height 4,476,732, fully
   shielded, 2 Ironwood actions) landed on the **stale pre-NU7 fork** and is
   not spendable on the official testnet; a second funding on the NU7 chain
   was used for the passing run.

### Testnet chain split (verified 2026-10-13)

| Height | `testnet.zec.rocks` (Zebra 7.0.0-rc.0, branch `0x77190ad9`, tip ≈4,498,100) | zexplorer.app / faucet chain (tip ≈4,476,760) |
|---|---|---|
| 4,465,025 `0000326c…a5d6f9` | present | present (common ancestor) |
| 4,465,026 `000089ba…4e78af` (NU7 activation) | present | **"not in the indexed best chain"** |
| 4,476,732 `00000941…d916c7` | present | absent — its own 4,476,732 holds the faucet tx |

The block counts since activation match 25 s spacing (NU7, ZIP 218) on the
zec.rocks chain and 75 s spacing on the other. The Zcash Foundation's Zebra
7.0.0-rc.0 release activated NU7 on public testnet at 4,465,026 and told
operators to upgrade to stay on the NU7 chain, so the zec.rocks chain is the
official one; the faucet and explorer are running nodes that did not upgrade.
Other public testnet lightwalletd endpoints probed
(`lightwalletd.testnet.electriccoin.co`, `testnet.lightwalletd.com`,
`zcash-testnet.chainsafe.dev`, and others) were unreachable, missing from DNS,
or returning HTTP errors.

**Lesson for production:** around network upgrades, block explorers, faucets
and third-party services can silently sit on a dead fork. hushush must verify
the chain it is on (branch id + checkpoint hashes from its own node), and must
not treat a third-party explorer as proof of payment.

## 4. Important finding: the stable Rust wallet crates cannot transact on testnet today

Testnet activated **NU7 at height 4,465,026** (ZIP 259, branch id `0x77190AD9`),
~9 days before this test. The latest *stable* librustzcash releases
(`zcash_client_backend` 0.24.0 / `zcash_protocol` 0.10.6) only know NU7 as an
unstable placeholder (`0xFFFFFFFF`). With them our wallet could **sync**, but
could not parse current testnet transactions (`invalid consensus branch id
0x77190ad9`) and would build NU6.3-branch transactions that nodes reject. The
only NU7-capable releases are the **pre-releases** published 2026-10-01…06,
which this proof pins exactly. Mainnet has no NU7 activation height yet.

Also new this cycle: **NU6.3 "Ironwood"** (testnet 4,134,000) introduced a third
shielded pool; Orchard is now a one-way "turnstile" and new value lands in
Ironwood. The library handles this automatically.

## 5. What an independent observer can see

From the raw transaction alone (no keys), as implemented in
[`observer.rs`](../wallet-proof/src/observer.rs) and asserted in the mocked tests:

**Live observation of the payment tx `e466d871…4b0898bb`** (v6, branch Nu7,
9,166 bytes): 0 transparent inputs/outputs, 0 Sapling, 0 Orchard, **2 Ironwood
actions**, Ironwood value balance +10,000 = the public fee, expiry 4,498,202,
lock time 0, one Ironwood anchor. Nothing in it reveals the 150,000-zat amount,
alice, bob, or the memo.

**Hidden:** sender and recipient addresses, payment amount, memo, which notes
were spent (nullifiers are unlinkable), number of real recipients (actions are
padded to ≥2). Mocked tests assert the amount does not appear in the bytes and
an unrelated key decrypts nothing.

**Visible / potential leaks:**
- **Fee** (= net value balance) — public, and implies the ZIP 317 action count.
- **Transaction shape** — counts of Sapling spends/outputs and Orchard/Ironwood actions.
- **Pool crossings** — when funds move Sapling→Orchard/Ironwood (or into the
  Ironwood turnstile), the *crossing amount* is public. Verified:
  `mocked_sapling_funds_to_unified_recipient_reveals_pool_crossing`. Keep the
  payroll treasury and payees in one pool (Ironwood) to avoid this.
- **Timing** — mined height/time, expiry height, lock time; anchors reveal
  roughly which height the spent notes were proven against.
- **Transparent components** — none in our transactions (enforced). Funding the
  treasury *from* a transparent source (exchange, faucet, coinbase) is public at
  that step: many recent testnet txs observed live have transparent inputs.
- **Infrastructure metadata** — the lightwalletd operator sees the client IP,
  the wallet birthday (tree-state request), every txid fetched (memo
  enhancement), and every broadcast tx with its timing. Running our own
  lightwalletd/Zebra (or Tor, which the library supports) removes this.
- **Outgoing viewing key** — we use `OvkPolicy::Sender`, so the sender's
  wallet can later decrypt what it paid (needed for audit). Anyone who obtains
  the sender's viewing key sees all payouts.

## 6. Persistence and recovery requirements

- **State:** one sqlite file per wallet; all notes, witnesses, tx history.
  Survives restarts (every CLI command is a separate process). Schema
  migrations run on open.
- **To recover funds you need:** the **24-word mnemonic** and the **birthday
  height** (earlier is safe but slower). The wallet DB is a cache and can be
  rebuilt by rescanning; losing it loses only memos/sent-tx metadata not
  re-derivable from chain (outgoing memos are recoverable via OVK).
- **Spend authority at rest** = `mnemonic.age` **plus** the separate age
  identity file. Either alone is useless. Losing both and the mnemonic backup =
  permanent loss of funds.
- **View-only access** (auditors, accounting): export the UFVK (`ufvk`
  command); it cannot spend.

## 7. Is this stack suitable for production shielded payroll?

**Yes as the foundation, not yet as-is.** librustzcash is the official,
actively maintained core used by Zcash's own wallets (its Orchard/Halo 2
components have been externally audited); it handled
Ironwood and NU7 correctly in mocked tests. Conditions before mainnet:

1. **Wait for (or pin and review) stable releases** carrying NU7, and track
   network upgrades closely — this cycle shipped two in ~2 months and the stable
   crates fell behind testnet.
2. **Key custody:** replace the age-identity file with a KMS/HSM-backed
   secret, or use the library's **PCZT** flow so an offline/HSM signer holds the
   spending key and the online service only proposes and proves.
3. **Own infrastructure:** run Zebra + lightwalletd (or Zallet) for privacy
   and availability instead of a public server.
4. **Payroll mechanics to design in Phase 1:** batching many payees per tx
   (ZIP 321 multi-payment), note splitting so concurrent payouts don't wait on
   change confirmations (default policy: 3 trusted / 10 untrusted
   confirmations), idempotent send with crash-safe txid tracking, expiry and
   rebroadcast handling.
5. Treat the current pre-release crates as **testnet-only**.

## 8. What I need from you next

Nothing further for Phase 0. For Phase 1:

1. **Key custody decision** — cloud KMS/HSM vendor, or an offline PCZT signer.
2. **Infrastructure** — a host
   to run our own Zebra + lightwalletd (size it from Zebra's current published
   requirements), so we stop depending on a single public
   server and on third-party explorers.
3. **Payroll model decision** — continuous off-chain accrual with on-demand
   private withdrawal (recommended) vs scheduled batched payouts.
4. More TAZ on the NU7 testnet (~1 TAZ) for batching and concurrency tests.

No mainnet funds are needed or should be used until stable NU7-capable
librustzcash releases ship and mainnet NU7 is scheduled.

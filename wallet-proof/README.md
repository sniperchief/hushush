# hushush Phase 0 — shielded payment feasibility proof (TESTNET ONLY)

A minimal Rust CLI on the official Zcash wallet stack (librustzcash:
`zcash_client_backend` + `zcash_client_sqlite`), talking to a public testnet
`lightwalletd`. It can create/restore wallets, sync, send shielded payments,
track confirmations, and show what a keyless observer sees. The network is
hard-coded to testnet; there is no mainnet code path.

Findings and recommendations: [../docs/PHASE0_REPORT.md](../docs/PHASE0_REPORT.md).

## Prerequisites

- Rust ≥ 1.88 (tested with 1.99.0, MSVC toolchain on Windows 10).
- Windows: Visual Studio Build Tools 2022 (C++), for `ring`/`libsqlite3-sys`.
- Network access to `testnet.zec.rocks:443` (or set `HUSHUSH_LIGHTWALLETD`).
- Python 3 and bash (Git Bash on Windows) for the end-to-end script.

## Build

```sh
cd wallet-proof
cargo build            # dependencies are built optimised even in dev profile
```

First build takes ~15–25 min (zk-proof crates). The Sapling proving parameters
are bundled into the binary (`zcash_proofs/bundled-prover`); nothing is
downloaded at runtime.

## Where state and secrets live

| What | Default location | Override |
|---|---|---|
| Wallet DB (`data.sqlite`), encrypted mnemonic (`mnemonic.age`) | `%LOCALAPPDATA%\hushush-phase0\wallets\<name>\` | `HUSHUSH_DATA_DIR` |
| age identity that decrypts the mnemonic | `%LOCALAPPDATA%\hushush-phase0\secrets\<name>.age-identity.txt` | `HUSHUSH_SECRETS_DIR` |

The tool refuses to place either inside a git working tree. Nothing secret is
logged; the mnemonic is only ever output by `backup-mnemonic`, to a new file.

## Commands

```sh
W=./target/debug/hushush-wallet-proof
$W info                                   # server, chain, tip, branch id
$W init alice                             # new wallet; prints shielded-only UA
$W address alice
$W sync alice                             # scan chain + fetch full txs; prints balance
$W balance alice
$W send alice --to <utest1…> --zats 150000 --memo "payout #1" [--dry-run]
$W status alice <txid>                    # mined height + confirmations
$W history bob                            # txs, outputs, decrypted memos
$W observe <txid>                         # keyless observer view (no wallet needed)
$W birthday alice
$W backup-mnemonic alice --out <path-outside-repo>
$W restore alice2 --mnemonic-file <path> --birthday <height>
```

## Tests

```sh
cargo test                                                   # unit + MOCKED + offline recovery
cargo test --test testnet_live -- --ignored --test-threads=1 # LIVE testnet, no funds needed
bash scripts/testnet_e2e.sh                                  # LIVE funded payment flow
```

| Suite | Network | What is real | What is mocked |
|---|---|---|---|
| `src/**` unit tests | none | address validation, age encryption, block cache | — |
| `tests/mocked_chain.rs` (`mocked_*`) | none | note decryption, witnesses, Halo 2 / Groth16 proofs, tx serialisation, wallet DB | chain: fake compact blocks generated in-process; no node validation, mempool or mining |
| `tests/recovery.rs` | none | sqlite persistence, seed→key/address recovery, encrypted-seed recovery | chain state (offline birthday) |
| `tests/testnet_live.rs` (`live_*`) | **testnet** | lightwalletd, chain sync, persistence across reopen, keyless decoding of real txs | — (no funds moved) |
| `scripts/testnet_e2e.sh` | **testnet** | the full funded flow: receive → send → confirm → recipient receipt + memo → observer view → restore from mnemonic | — |

The end-to-end script needs the payer wallet (`alice`) funded with testnet ZEC
(TAZ). It waits for funds, then runs unattended, logging to `evidence/`.

# hushush payout wallet (Phase 3, TESTNET ONLY)

The only hushush process with spend authority. It wraps the Phase 0 wallet
proof ([`../wallet-proof/`](../wallet-proof/), used unchanged as a library)
behind a narrow, authenticated HTTP API that payroll-api calls to quote, pay
and track shielded payouts.

Full documentation (configuration, secrets, states, recovery, test results):
[../docs/PHASE3_PAYOUTS.md](../docs/PHASE3_PAYOUTS.md).

```sh
cargo build                                   # first build: ~20–30 min (zk crates)
cargo test                                    # unit tests (no network)
WALLET_API_TOKEN=… WALLET_NAME=treasury \
WALLET_LIGHTWALLETD_URL=https://testnet.zec.rocks:443 ./target/debug/hushush-payout-wallet

# Live end-to-end run (real testnet funds; see the doc for prerequisites):
cargo build --features fault-injection && cp target/debug/hushush-payout-wallet.exe target/debug/hushush-payout-wallet-fault.exe && cargo build
RUN_ID=myrun bash scripts/live_e2e.sh         # SCENARIOS=ABC by default; RESUME=1 to continue a run
```

Never deploy a build with `--features fault-injection`.

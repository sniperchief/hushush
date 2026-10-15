# hushush

Private, streaming ZEC payroll.

| Directory | What | Docs |
|---|---|---|
| [`wallet-proof/`](wallet-proof/) | Phase 0: shielded testnet payment proof on librustzcash (unchanged since Phase 0; reused as a library by `payout-wallet/`) | [docs/PHASE0_REPORT.md](docs/PHASE0_REPORT.md) |
| [`payroll/`](payroll/) | Payroll engine (Rust): accrual, streams, claim reservations, payout state machine, Phase 5 stream offers / locked addresses / milestone withdrawals; SQLite and PostgreSQL backends | [docs/PHASE1_PAYROLL.md](docs/PHASE1_PAYROLL.md), [docs/PHASE3_PAYOUTS.md](docs/PHASE3_PAYOUTS.md), [docs/PHASE5_REPORT.md](docs/PHASE5_REPORT.md) |
| [`payroll-api/`](payroll-api/) | Internal HTTP service exposing the engine on PostgreSQL to the web app; drives payouts; seals locked payout addresses | [docs/PHASE2_WEB.md](docs/PHASE2_WEB.md), [docs/PHASE3_PAYOUTS.md](docs/PHASE3_PAYOUTS.md), [docs/PHASE5_REPORT.md](docs/PHASE5_REPORT.md) |
| [`payout-wallet/`](payout-wallet/) | Phase 3: the payout wallet service — the only process with spend authority (TESTNET ONLY) | [docs/PHASE3_PAYOUTS.md](docs/PHASE3_PAYOUTS.md) |
| [`web/`](web/) | Next.js app: passkey accounts (employer / worker), streams with worker links, payout address lock, Withdraw | [docs/PHASE5_REPORT.md](docs/PHASE5_REPORT.md) (Phase 2 background: [docs/PHASE2_WEB.md](docs/PHASE2_WEB.md)) |

An employer signs up with a passkey, creates a stream (total, start, end) and
shares its worker link. The worker signs up with a passkey through the link,
locks a shielded Zcash **testnet** payout address once, and withdraws at each
20% milestone (and the remainder after the end). A withdrawal is marked paid
only after its transaction is confirmed on chain and verified, and is never
paid twice. There is no mainnet code path.

Each company pays from its own payout account, funded by the employer (or, on
testnet, with a "Get test TAZ" faucet top-up):
[docs/PHASE6_COMPANY_WALLETS.md](docs/PHASE6_COMPANY_WALLETS.md).

Security review, exactly-once verification and independent privacy checks:
[docs/PHASE4_REPORT.md](docs/PHASE4_REPORT.md).

## Roadmap: v2, non-custodial on mainnet

v1 holds testnet payout keys so the product can be tested end to end. **v2
makes hushush non-custodial:** employers keep their funds in their own wallet
(Zashi or Keystone) with their own seed phrase. hushush gets a view-only key,
prepares each shielded payout, and the employer approves it on their own
device. hushush can see payroll but can never move funds:
[docs/V2_NONCUSTODIAL_ROADMAP.md](docs/V2_NONCUSTODIAL_ROADMAP.md).

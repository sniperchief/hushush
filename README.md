# hushush

Private, streaming ZEC payroll.

| Directory | What | Docs |
|---|---|---|
| [`wallet-proof/`](wallet-proof/) | Phase 0: shielded testnet payment proof on librustzcash (unchanged since Phase 0; reused as a library by `payout-wallet/`) | [docs/PHASE0_REPORT.md](docs/PHASE0_REPORT.md) |
| [`payroll/`](payroll/) | Payroll engine (Rust): accrual, streams, claim reservations, payout state machine; SQLite and PostgreSQL backends | [docs/PHASE1_PAYROLL.md](docs/PHASE1_PAYROLL.md), [docs/PHASE3_PAYOUTS.md](docs/PHASE3_PAYOUTS.md) |
| [`payroll-api/`](payroll-api/) | Internal HTTP service exposing the engine on PostgreSQL to the web app; drives payouts | [docs/PHASE2_WEB.md](docs/PHASE2_WEB.md), [docs/PHASE3_PAYOUTS.md](docs/PHASE3_PAYOUTS.md) |
| [`payout-wallet/`](payout-wallet/) | Phase 3: the payout wallet service — the only process with spend authority (TESTNET ONLY) | [docs/PHASE3_PAYOUTS.md](docs/PHASE3_PAYOUTS.md) |
| [`web/`](web/) | Next.js app: auth, organizations, invitations, streams, claims, payouts | [docs/PHASE2_WEB.md](docs/PHASE2_WEB.md), [docs/PHASE3_PAYOUTS.md](docs/PHASE3_PAYOUTS.md) |

Contributors can pay a reserved claim out to their shielded Zcash **testnet**
address. A claim is marked paid only after its transaction is confirmed on
chain and verified. There is no mainnet code path.

Security review, exactly-once verification and independent privacy checks:
[docs/PHASE4_REPORT.md](docs/PHASE4_REPORT.md).

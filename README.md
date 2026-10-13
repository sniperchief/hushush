# hushush

Private, streaming ZEC payroll.

| Directory | What | Docs |
|---|---|---|
| [`wallet-proof/`](wallet-proof/) | Phase 0: shielded testnet payment proof on librustzcash (unchanged since Phase 0) | [docs/PHASE0_REPORT.md](docs/PHASE0_REPORT.md) |
| [`payroll/`](payroll/) | Payroll engine (Rust): accrual, streams, claim reservations; SQLite and PostgreSQL backends | [docs/PHASE1_PAYROLL.md](docs/PHASE1_PAYROLL.md) |
| [`payroll-api/`](payroll-api/) | Internal HTTP service exposing the engine on PostgreSQL to the web app | [docs/PHASE2_WEB.md](docs/PHASE2_WEB.md) |
| [`web/`](web/) | Next.js app: auth, organizations, invitations, streams, claims | [docs/PHASE2_WEB.md](docs/PHASE2_WEB.md) |

Claims are accounting reservations only. No wallet is connected to the app,
and no ZEC payouts are made.

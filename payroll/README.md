# hushush-payroll

Payroll engine for hushush: organizations, members, ZEC payroll streams with
server-side accrual, and claim reservations (accounting only, no payouts).

```sh
cargo test                                         # 41 offline tests
cargo clippy --all-targets -- -D warnings
cargo run --bin payroll-migrate -- <db.sqlite>     # apply schema migrations
```

Schema, accrual formula, claim rules and test results:
[../docs/PHASE1_PAYROLL.md](../docs/PHASE1_PAYROLL.md).

Vesting compensation (agreements, schedule, claims, cancellation policy):
[../docs/VESTING_PHASE1_ACCOUNTING.md](../docs/VESTING_PHASE1_ACCOUNTING.md)
and payouts, API and UI: [../docs/VESTING_PHASE2_PAYOUTS.md](../docs/VESTING_PHASE2_PAYOUTS.md).

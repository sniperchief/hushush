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

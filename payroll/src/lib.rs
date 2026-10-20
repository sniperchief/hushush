//! hushush payroll engine.
//!
//! Organizations pay members through *payroll streams*: a fixed ZEC rate that
//! accrues over active time, computed only on the server from the server clock.
//! Recipients *claim* earned wages; a claim is an accounting reservation. A
//! reserved claim can then be paid out through the shielded payout wallet
//! ([`payout`]); this crate never holds keys or talks to the chain itself —
//! it drives the wallet service through the [`payout::WalletApi`] contract.
//!
//! Business rules are written once in [`engine`], generic over a storage
//! [`store::Backend`]. Two backends exist: [`sqlite`] (the Phase 1 tested
//! storage layer) and [`pg`] (PostgreSQL, used by the deployed web app via the
//! `payroll-api` service). Both enforce the same invariants again in their
//! schemas (CHECK constraints and triggers).
//!
//! Money is integer zatoshis everywhere (see [`money`]); accrual is integer
//! arithmetic (see [`accrual`]).

pub mod accrual;
pub mod clock;
pub mod engine;
pub mod error;
pub mod flow;
pub mod migrations;
pub mod model;
pub mod money;
pub mod payout;
pub mod pg;
pub mod sqlite;
pub mod store;
pub mod vesting;

pub use engine::{MAX_RATE_PERIOD_SECS, Payroll};
pub use error::{PayrollError, Result};
pub use flow::{BindOutcome, FlowStream, NewOffer, WithdrawTerms};
pub use model::*;
pub use payout::{PayoutPolicy, PayoutTerms};
pub use pg::PgPayrollDb;
pub use sqlite::PayrollDb;
pub use vesting::{NewVesting, VestingDetail, VestingWithdrawTerms};

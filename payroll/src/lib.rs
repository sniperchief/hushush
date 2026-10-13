//! hushush payroll engine.
//!
//! Organizations pay members through *payroll streams*: a fixed ZEC rate that
//! accrues over active time, computed only on the server from the server clock.
//! Recipients *claim* earned wages; a claim is purely an accounting reservation
//! — nothing here talks to the Zcash wallet.
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
pub mod migrations;
pub mod model;
pub mod money;
pub mod pg;
pub mod sqlite;
pub mod store;

pub use engine::{MAX_RATE_PERIOD_SECS, Payroll};
pub use error::{PayrollError, Result};
pub use model::*;
pub use pg::PgPayrollDb;
pub use sqlite::PayrollDb;

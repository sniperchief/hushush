//! Applies pending payroll schema migrations.
//!
//! Usage: payroll-migrate <path-to-payroll.sqlite>

use std::sync::Arc;

use hushush_payroll::clock::{Clock, SystemClock};
use hushush_payroll::migrations;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: payroll-migrate <db-path>")?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let mut conn = rusqlite::Connection::open(&path)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    let applied = migrations::migrate(&mut conn, clock.now())?;
    if applied.is_empty() {
        println!(
            "{path}: up to date (schema v{})",
            migrations::latest_version()
        );
    } else {
        println!(
            "{path}: applied {applied:?}; now at schema v{}",
            migrations::latest_version()
        );
    }
    Ok(())
}

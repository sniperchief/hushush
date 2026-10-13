//! The server clock is the only source of time for accrual. Callers (and
//! frontends) can never supply a timestamp.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

pub trait Clock: Send + Sync {
    /// Current unix time in seconds (UTC).
    fn now(&self) -> i64;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

/// A settable clock for tests and simulations. Clones share the same time.
#[derive(Clone, Debug, Default)]
pub struct ManualClock(Arc<AtomicI64>);

impl ManualClock {
    pub fn at(t: i64) -> Self {
        Self(Arc::new(AtomicI64::new(t)))
    }
    pub fn set(&self, t: i64) {
        self.0.store(t, Ordering::SeqCst);
    }
    pub fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

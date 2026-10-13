//! Server-authoritative wage accrual. Pure integer arithmetic; no floats.
//!
//! ```text
//! active_secs(t) = banked_active_secs
//!                + max(0, min(t, end_at) - active_since)   if active, else 0
//! earned(t)      = min(MAX_MONEY, floor(rate_zats * active_secs(t) / rate_period_secs))
//! ```
//!
//! The floor is taken once over the TOTAL active time (never per interval), so
//! pausing and resuming never loses fractional zatoshis, and `earned` is
//! monotonically non-decreasing in `t`.

use crate::money::MAX_MONEY_ZATS;

/// Accrual-relevant state of a stream, as persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccrualState {
    pub rate_zats: i64,
    pub rate_period_secs: i64,
    pub start_at: i64,
    pub end_at: Option<i64>,
    pub banked_active_secs: i64,
    /// Start of the currently open active interval; `None` when paused/ended.
    pub active_since: Option<i64>,
}

impl AccrualState {
    /// Total active seconds up to time `t`.
    pub fn active_secs_at(&self, t: i64) -> i64 {
        let cap = self.end_at.map_or(t, |e| t.min(e));
        let running = self.active_since.map_or(0, |since| (cap - since).max(0));
        self.banked_active_secs.saturating_add(running)
    }

    /// Zatoshis earned up to time `t`.
    pub fn earned_at(&self, t: i64) -> i64 {
        earned_for(
            self.rate_zats,
            self.rate_period_secs,
            self.active_secs_at(t),
        )
    }

    /// True once the scheduled end date has been reached.
    pub fn schedule_finished_at(&self, t: i64) -> bool {
        self.end_at.is_some_and(|e| t >= e)
    }

    /// State after pausing (or ending) at `t`: the open interval is banked.
    pub fn closed_at(&self, t: i64) -> AccrualState {
        AccrualState {
            banked_active_secs: self.active_secs_at(t),
            active_since: None,
            ..*self
        }
    }

    /// State after resuming at `t`. Accrual restarts at `t`, or at `start_at`
    /// if the stream has not started yet.
    pub fn resumed_at(&self, t: i64) -> AccrualState {
        AccrualState {
            active_since: Some(t.max(self.start_at)),
            ..*self
        }
    }
}

/// `floor(rate_zats * active_secs / rate_period_secs)`, saturating at the
/// Zcash money supply. Computed in i128 so the product cannot overflow.
pub fn earned_for(rate_zats: i64, rate_period_secs: i64, active_secs: i64) -> i64 {
    debug_assert!(rate_zats > 0 && rate_period_secs > 0 && active_secs >= 0);
    let earned = (rate_zats as i128 * active_secs.max(0) as i128) / rate_period_secs as i128;
    earned.min(MAX_MONEY_ZATS as i128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(rate: i64, period: i64, start: i64, end: Option<i64>) -> AccrualState {
        AccrualState {
            rate_zats: rate,
            rate_period_secs: period,
            start_at: start,
            end_at: end,
            banked_active_secs: 0,
            active_since: Some(start),
        }
    }

    #[test]
    fn nothing_before_start_and_exactly_at_start() {
        let st = s(3600, 3600, 1_000, None);
        assert_eq!(st.earned_at(0), 0);
        assert_eq!(st.earned_at(999), 0);
        assert_eq!(st.earned_at(1_000), 0);
        assert_eq!(st.earned_at(1_001), 1);
    }

    #[test]
    fn floors_and_never_rounds_up() {
        // 1 zat per 3 seconds.
        let st = s(1, 3, 0, None);
        assert_eq!(st.earned_at(2), 0);
        assert_eq!(st.earned_at(3), 1);
        assert_eq!(st.earned_at(5), 1);
        assert_eq!(st.earned_at(6), 2);
    }

    #[test]
    fn stops_exactly_at_end_date() {
        let st = s(10, 1, 100, Some(200));
        assert_eq!(st.earned_at(199), 990);
        assert_eq!(st.earned_at(200), 1_000);
        assert_eq!(st.earned_at(10_000), 1_000);
        assert!(st.schedule_finished_at(200) && !st.schedule_finished_at(199));
    }

    #[test]
    fn pause_resume_loses_no_fractional_zats() {
        // 1 zat per 3s. Ten 1-second active intervals = 10s = 3 zats, even
        // though each interval alone would floor to 0.
        let mut st = s(1, 3, 0, None);
        let mut t = 0;
        for _ in 0..10 {
            t += 1;
            st = st.closed_at(t);
            t += 100; // paused time does not count
            st = st.resumed_at(t);
        }
        st = st.closed_at(t);
        assert_eq!(st.active_secs_at(t), 10);
        assert_eq!(st.earned_at(t + 1_000_000), 3);
    }

    #[test]
    fn paused_stream_holds_earnings() {
        let st = s(5, 1, 0, None).closed_at(10);
        assert_eq!(st.earned_at(10), 50);
        assert_eq!(st.earned_at(1_000_000), 50);
    }

    #[test]
    fn resume_before_start_waits_for_start() {
        let st = s(1, 1, 1_000, None).closed_at(10).resumed_at(20);
        assert_eq!(st.active_since, Some(1_000));
        assert_eq!(st.earned_at(1_005), 5);
    }

    #[test]
    fn monotonic_over_time() {
        let st = s(7, 13, 50, Some(5_000));
        let mut prev = 0;
        for t in 0..6_000 {
            let e = st.earned_at(t);
            assert!(e >= prev);
            prev = e;
        }
    }

    #[test]
    fn huge_values_saturate_instead_of_overflowing() {
        let st = s(MAX_MONEY_ZATS, 1, 0, None);
        assert_eq!(st.earned_at(i64::MAX / 2), MAX_MONEY_ZATS);
    }

    #[test]
    fn clock_before_active_since_counts_zero() {
        let st = s(1, 1, 100, None);
        assert_eq!(st.active_secs_at(50), 0);
    }
}

//! Vesting compensation scenarios (agreements, vesting schedule, claims,
//! cancellation, authorization, schema guards), run against BOTH storage
//! backends. No wallet is involved: vesting claims are accounting only so far.
//!
//! * `sqlite::*` always runs.
//! * `postgres::*` needs `PAYROLL_TEST_DATABASE_URL` and is `#[ignore]`d by
//!   default (`cargo test --test vesting -- --include-ignored`).

use std::sync::{Arc, Barrier};

use hushush_payroll::clock::{Clock, ManualClock};
use hushush_payroll::money::COIN;
use hushush_payroll::store::Backend;
use hushush_payroll::*;
use std::result::Result;

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
/// 4 ZEC over 400 days with a 100-day cliff: 1,000,000 zats per day; the
/// cliff releases exactly 1 ZEC.
const TOTAL: i64 = 4 * COIN;
const DURATION: i64 = 400 * DAY;
const CLIFF: i64 = 100 * DAY;

// ---- world --------------------------------------------------------------------

trait Factory: Fresh + Send + Sync + 'static {
    type B: Backend;
    fn open(&self, clock: &ManualClock) -> Payroll<Self::B>;
    /// Runs raw SQL as a direct database user would; returns the error text.
    fn raw(&self, sql: &str) -> Result<(), String>;
}

trait Fresh {
    fn fresh() -> Self;
}

struct World<F: Factory> {
    f: Arc<F>,
    clock: ManualClock,
    db: Payroll<F::B>,
    /// Owner of `org`.
    employer: String,
    /// Admin of `org`.
    admin: String,
    /// Member of `org`; receives the grants.
    employee: String,
    /// Another plain member of `org`.
    colleague: String,
    /// Member of no organization.
    stranger: String,
    /// Owner of `rival_org`.
    rival: String,
    org: String,
    rival_org: String,
}

fn world<F: Factory>(f: F) -> World<F> {
    let f = Arc::new(f);
    let clock = ManualClock::at(T0);
    let mut db = f.open(&clock);
    let mut user = |n: &str| db.create_user(&format!("{n}@a.test"), n).unwrap();
    let (employer, admin, employee, colleague, stranger, rival) = (
        user("employer"),
        user("admin"),
        user("employee"),
        user("colleague"),
        user("stranger"),
        user("rival"),
    );
    let org = db.create_organization(&employer, "Acme").unwrap();
    let rival_org = db.create_organization(&rival, "Rival Co").unwrap();
    db.add_member(&employer, &org, &admin, Role::Admin).unwrap();
    db.add_member(&employer, &org, &employee, Role::Member)
        .unwrap();
    db.add_member(&employer, &org, &colleague, Role::Member)
        .unwrap();
    World {
        f,
        clock,
        db,
        employer,
        admin,
        employee,
        colleague,
        stranger,
        rival,
        org,
        rival_org,
    }
}

impl<F: Factory> World<F> {
    fn new_vesting(&self, total: i64, start_in: i64, duration: i64, cliff: i64) -> NewVesting {
        NewVesting {
            organization_id: self.org.clone(),
            employee_user_id: self.employee.clone(),
            total_zats: total,
            start_at: Some(self.clock.now() + start_in),
            duration_secs: duration,
            cliff_secs: Some(cliff),
            method: VestingMethod::Linear,
        }
    }

    fn draft(&mut self, total: i64, start_in: i64, duration: i64, cliff: i64) -> VestingAgreement {
        let req = self.new_vesting(total, start_in, duration, cliff);
        let e = self.employer.clone();
        self.db.create_vesting(&e, req).unwrap()
    }

    /// Created and activated now, starting `start_in` seconds from now.
    fn grant(&mut self, total: i64, start_in: i64, duration: i64, cliff: i64) -> VestingAgreement {
        let a = self.draft(total, start_in, duration, cliff);
        let e = self.employer.clone();
        self.db.activate_vesting(&e, &a.id).unwrap()
    }

    /// The standard grant (TOTAL / DURATION / CLIFF), starting now.
    fn standard(&mut self) -> VestingAgreement {
        self.grant(TOTAL, 0, DURATION, CLIFF)
    }

    fn bal(&mut self, id: &str) -> VestingBalance {
        let e = self.employee.clone();
        self.db.vesting_balance(&e, id).unwrap()
    }

    fn claim(&mut self, id: &str, amount: i64, key: &str) -> hushush_payroll::Result<VestingClaim> {
        let e = self.employee.clone();
        self.db
            .reserve_vesting_claim(&e, id, amount, key)
            .map(|o| o.claim)
    }

    fn settle(&mut self, claim: &str) -> VestingClaim {
        let e = self.employer.clone();
        self.db
            .settle_vesting_claim(&e, claim, "paid-out-of-band")
            .unwrap()
    }
}

fn insufficient<T: std::fmt::Debug>(r: hushush_payroll::Result<T>) -> i64 {
    match r {
        Err(PayrollError::InsufficientAvailable { available, .. }) => available,
        other => panic!("expected InsufficientAvailable, got {other:?}"),
    }
}

fn check_ledger(b: &VestingBalance) {
    assert_eq!(b.vested + b.unvested, b.total, "{b:?}");
    assert_eq!(b.available, b.vested - b.reserved - b.paid, "{b:?}");
    assert!(b.available >= 0 && b.vested <= b.total, "{b:?}");
}

// ---- vesting schedule ------------------------------------------------------------

/// The schedule as seen through the engine, at every boundary.
fn schedule_boundaries<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.grant(TOTAL, DAY, DURATION, CLIFF);
    let start = T0 + DAY;
    let at = |w: &mut World<F>, t: i64| {
        w.clock.set(t);
        let b = w.bal(&a.id);
        check_ledger(&b);
        (b.phase, b.vested)
    };
    use VestingPhase::*;
    assert_eq!(at(&mut w, T0), (NotStarted, 0));
    assert_eq!(at(&mut w, start), (InCliff, 0), "zero elapsed time");
    assert_eq!(
        at(&mut w, start + CLIFF - 1),
        (InCliff, 0),
        "just before the cliff"
    );
    assert_eq!(
        at(&mut w, start + CLIFF),
        (Vesting, COIN),
        "exactly at the cliff"
    );
    // 4e8 * (CLIFF + 1) / DURATION = 1e8 + 11.57…
    assert_eq!(
        at(&mut w, start + CLIFF + 1),
        (Vesting, COIN + 11),
        "just after the cliff"
    );
    assert_eq!(
        at(&mut w, start + DURATION / 2),
        (Vesting, 2 * COIN),
        "midway"
    );
    // One second short: 4e8 / DURATION ≈ 11.57 zats unvested, floored to 12.
    assert_eq!(at(&mut w, start + DURATION - 1), (Vesting, TOTAL - 12));
    assert_eq!(
        at(&mut w, start + DURATION),
        (FullyVested, TOTAL),
        "exactly at the end"
    );
    assert_eq!(
        at(&mut w, start + 10 * DURATION),
        (FullyVested, TOTAL),
        "after the end"
    );
    assert_eq!(w.bal(&a.id).unvested, 0);
}

/// Tiny allocations floor and still reconcile exactly; the whole allocation
/// can be claimed and paid, and not one zatoshi more.
fn small_allocation_reconciles<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.grant(3, 0, 10, 0);
    let mut seen = vec![];
    for t in 0..=11 {
        w.clock.set(T0 + t);
        seen.push(w.bal(&a.id).vested);
    }
    assert_eq!(seen, [0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3]);
    // Claim as it vests: 1 at t=4 (paid), 1 more at t=7, the last at t=10.
    w.clock.set(T0 + 4);
    let c1 = w.claim(&a.id, 1, "a").unwrap();
    w.settle(&c1.id);
    assert_eq!(insufficient(w.claim(&a.id, 1, "b")), 0);
    w.clock.set(T0 + 7);
    w.claim(&a.id, 1, "b").unwrap();
    w.clock.set(T0 + 10);
    w.claim(&a.id, 1, "c").unwrap();
    assert_eq!(insufficient(w.claim(&a.id, 1, "d")), 0);
    let b = w.bal(&a.id);
    assert_eq!((b.vested, b.paid, b.reserved, b.available), (3, 1, 2, 0));
}

/// One-second and 100-year schedules through the engine.
fn short_and_long_durations<F: Factory>(f: F) {
    let mut w = world(f);
    let short = w.grant(5, 0, 1, 0);
    assert_eq!(w.bal(&short.id).vested, 0);
    w.clock.advance(1);
    assert_eq!(w.bal(&short.id).vested, 5);
    let long = w.grant(TOTAL, 0, MAX_RATE_PERIOD_SECS, 0);
    w.clock.advance(MAX_RATE_PERIOD_SECS / 2);
    assert_eq!(w.bal(&long.id).vested, TOTAL / 2);
    w.clock.advance(MAX_RATE_PERIOD_SECS / 2);
    let b = w.bal(&long.id);
    assert_eq!((b.vested, b.phase), (TOTAL, VestingPhase::FullyVested));
    // Bounds.
    let e = w.employer.clone();
    for (duration, cliff) in [(0, 0), (MAX_RATE_PERIOD_SECS + 1, 0), (10, 11), (10, -1)] {
        let req = w.new_vesting(TOTAL, 0, duration, cliff);
        assert!(
            matches!(
                w.db.create_vesting(&e, req),
                Err(PayrollError::Validation(_))
            ),
            "{duration}/{cliff}"
        );
    }
    for total in [0, -1, hushush_payroll::money::MAX_MONEY_ZATS + 1] {
        let req = w.new_vesting(total, 0, DAY, 0);
        assert!(matches!(
            w.db.create_vesting(&e, req),
            Err(PayrollError::Validation(_))
        ));
    }
    let mut req = w.new_vesting(TOTAL, 0, DAY, 0);
    req.start_at = Some(w.clock.now() - 1);
    assert!(
        matches!(
            w.db.create_vesting(&e, req),
            Err(PayrollError::Validation(_))
        ),
        "no backdating"
    );
}

// ---- claims ------------------------------------------------------------------------

fn claims_against_vested<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.standard();
    // Unvested compensation is never claimable: before the cliff, nothing.
    w.clock.advance(CLIFF - 1);
    assert_eq!(insufficient(w.claim(&a.id, 1, "early")), 0);
    w.clock.advance(1);
    // Vested 1 ZEC. Claim less than available.
    let c1 = w.claim(&a.id, 30_000_000, "k1").unwrap();
    assert_eq!(c1.compensation_type, CompensationType::Vesting);
    assert_eq!(
        (c1.agreement_id.as_str(), c1.status),
        (a.id.as_str(), ClaimStatus::Reserved)
    );
    let b = w.bal(&a.id);
    check_ledger(&b);
    assert_eq!((b.reserved, b.available), (30_000_000, 70_000_000));
    // With some already reserved, more than what is left is rejected…
    assert_eq!(insufficient(w.claim(&a.id, 70_000_001, "k2")), 70_000_000);
    // …and the unvested part is never claimable.
    assert_eq!(insufficient(w.claim(&a.id, TOTAL, "k2")), 70_000_000);
    // Exactly the rest succeeds; then nothing is left.
    w.claim(&a.id, 70_000_000, "k2").unwrap();
    assert_eq!(insufficient(w.claim(&a.id, 1, "k3")), 0);
    let b = w.bal(&a.id);
    assert_eq!(
        (b.vested, b.reserved, b.available, b.unvested),
        (COIN, COIN, 0, 3 * COIN)
    );
    // More vests over time.
    w.clock.advance(DAY);
    assert_eq!(w.bal(&a.id).available, 1_000_000);
    // Bad inputs.
    for (amount, key) in [(0, "x"), (-5, "x"), (1, ""), (1, "with space")] {
        assert!(matches!(
            w.claim(&a.id, amount, key),
            Err(PayrollError::Validation(_))
        ));
    }
}

/// A failed or abandoned reservation is released; its amount becomes
/// available again, exactly once.
fn release_failed_reservation<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.standard();
    w.clock.advance(CLIFF);
    let c = w.claim(&a.id, COIN, "k1").unwrap();
    assert_eq!(w.bal(&a.id).available, 0);
    // The employee releases it (e.g. the payment attempt failed).
    let emp = w.employee.clone();
    let r = w.db.release_vesting_claim(&emp, &c.id).unwrap();
    assert_eq!(r.status, ClaimStatus::Released);
    assert_eq!(w.bal(&a.id).available, COIN);
    // One-way: neither release nor settle again.
    assert!(matches!(
        w.db.release_vesting_claim(&emp, &c.id),
        Err(PayrollError::InvalidState(_))
    ));
    let e = w.employer.clone();
    assert!(matches!(
        w.db.settle_vesting_claim(&e, &c.id, "x"),
        Err(PayrollError::InvalidState(_))
    ));
    // A retry with the released claim's key returns that claim; it does not
    // reserve again.
    let again = w.db.reserve_vesting_claim(&emp, &a.id, COIN, "k1").unwrap();
    assert!(again.duplicate && again.claim.status == ClaimStatus::Released);
    assert_eq!(w.bal(&a.id).reserved, 0);
    // A manager may release too; a colleague may not even see the claim.
    let c2 = w.claim(&a.id, COIN, "k2").unwrap();
    let col = w.colleague.clone();
    assert!(matches!(
        w.db.release_vesting_claim(&col, &c2.id),
        Err(PayrollError::NotFound("vesting claim"))
    ));
    let adm = w.admin.clone();
    w.db.release_vesting_claim(&adm, &c2.id).unwrap();
    assert_eq!(w.bal(&a.id).available, COIN);
}

/// Duplicate requests and retries never reserve twice.
fn duplicates_and_retries<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.standard();
    w.clock.advance(CLIFF);
    let emp = w.employee.clone();
    let first =
        w.db.reserve_vesting_claim(&emp, &a.id, 40_000_000, "pay-1")
            .unwrap();
    let again =
        w.db.reserve_vesting_claim(&emp, &a.id, 40_000_000, "pay-1")
            .unwrap();
    assert!(!first.duplicate && again.duplicate);
    assert_eq!(first.claim.id, again.claim.id);
    assert!(matches!(
        w.db.reserve_vesting_claim(&emp, &a.id, 40_000_001, "pay-1"),
        Err(PayrollError::IdempotencyConflict)
    ));
    assert_eq!(w.bal(&a.id).reserved, 40_000_000);
    // Keys are per agreement: the same key on another agreement is independent.
    let b = w.grant(TOTAL, 0, DAY, 0);
    w.clock.advance(DAY);
    w.claim(&b.id, 40_000_000, "pay-1").unwrap();
    assert_eq!(w.bal(&a.id).reserved, 40_000_000);

    // Concurrent duplicates: exactly one claim.
    const THREADS: usize = 8;
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let (fac, clock, emp, id, barrier) = (
                w.f.clone(),
                w.clock.clone(),
                emp.clone(),
                a.id.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                let mut db = fac.open(&clock);
                barrier.wait();
                db.reserve_vesting_claim(&emp, &id, 10_000_000, "same")
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(results.iter().all(|r| r.is_ok()), "{results:?}");
    assert_eq!(
        results
            .iter()
            .filter(|r| !r.as_ref().unwrap().duplicate)
            .count(),
        1
    );
    let claims = w.db.list_vesting_claims(&emp, &a.id).unwrap();
    assert_eq!(
        claims
            .iter()
            .filter(|c| c.idempotency_key == "same")
            .count(),
        1
    );
    assert_eq!(w.bal(&a.id).reserved, 50_000_000);
}

/// Concurrent claims never reserve more than is vested.
fn concurrent_claims<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.grant(TOTAL, 0, DURATION, 0);
    w.clock.advance(DURATION / 2); // 2 ZEC vested
    const THREADS: usize = 16;
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|i| {
            let (fac, clock, emp, id, barrier) = (
                w.f.clone(),
                w.clock.clone(),
                w.employee.clone(),
                a.id.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                let mut db = fac.open(&clock);
                barrier.wait();
                // Half the threads reuse one key (client retries), half are distinct.
                let key = if i % 2 == 0 {
                    "retry".to_string()
                } else {
                    format!("t{i}")
                };
                db.reserve_vesting_claim(&emp, &id, 30_000_000, &key)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let created = results
        .iter()
        .filter(|r| matches!(r, Ok(o) if !o.duplicate))
        .count();
    // 8 distinct keys + 1 shared key = 9 candidates; only 6 × 0.3 fit in 2 ZEC.
    assert_eq!(created, 6, "{results:?}");
    for r in &results {
        assert!(
            matches!(r, Ok(_) | Err(PayrollError::InsufficientAvailable { .. })),
            "{r:?}"
        );
    }
    let b = w.bal(&a.id);
    check_ledger(&b);
    assert_eq!((b.reserved, b.available), (180_000_000, 20_000_000));
}

/// A settled claim is final: its amount never becomes claimable again.
fn settled_is_final<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.standard();
    w.clock.advance(CLIFF);
    let c = w.claim(&a.id, COIN, "k1").unwrap();
    let s = w.settle(&c.id);
    assert_eq!(
        (s.status, s.settlement_ref.as_deref()),
        (ClaimStatus::Settled, Some("paid-out-of-band"))
    );
    let b = w.bal(&a.id);
    assert_eq!((b.paid, b.reserved, b.available), (COIN, 0, 0));
    let (emp, e) = (w.employee.clone(), w.employer.clone());
    assert!(matches!(
        w.db.release_vesting_claim(&emp, &c.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        w.db.settle_vesting_claim(&e, &c.id, "again"),
        Err(PayrollError::InvalidState(_))
    ));
    // A retry of the settled request returns it and reserves nothing.
    let again = w.db.reserve_vesting_claim(&emp, &a.id, COIN, "k1").unwrap();
    assert!(again.duplicate && again.claim.status == ClaimStatus::Settled);
    assert_eq!(insufficient(w.claim(&a.id, 1, "k2")), 0);
    // Only managers settle.
    let c2 = {
        w.clock.advance(DAY);
        w.claim(&a.id, 1_000_000, "k3").unwrap()
    };
    assert!(matches!(
        w.db.settle_vesting_claim(&emp, &c2.id, "self"),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        w.db.settle_vesting_claim(&e, &c2.id, " padded "),
        Err(PayrollError::Validation(_))
    ));
    // Even a direct database write cannot reopen a settled claim.
    let e =
        w.f.raw(&format!(
            "UPDATE vesting_claims SET status = 'reserved', resolved_at = NULL, resolved_by = NULL,
                 settlement_ref = NULL WHERE id = '{}'",
            c.id
        ))
        .unwrap_err();
    assert!(e.contains("invalid claim status transition"), "{e}");
    assert_eq!(w.bal(&a.id).paid, COIN);
}

// ---- lifecycle --------------------------------------------------------------------

fn lifecycle_and_audit<F: Factory>(f: F) {
    let mut w = world(f);
    // Starts in a day; activated before then (activation after the start is
    // refused, see tests/vesting_payouts.rs).
    let a = w.draft(TOTAL, DAY, DURATION, CLIFF);
    let start = T0 + DAY;
    assert_eq!(a.status, VestingStatus::Draft);
    assert_eq!(a.created_by, w.employer);
    // A draft vests nothing and the employee cannot see or claim it.
    w.clock.advance(DAY / 2);
    let emp = w.employee.clone();
    assert!(matches!(
        w.db.get_vesting(&emp, &a.id),
        Err(PayrollError::NotFound("vesting agreement"))
    ));
    assert!(matches!(
        w.db.reserve_vesting_claim(&emp, &a.id, 1, "k"),
        Err(PayrollError::NotFound(_))
    ));
    assert!(w.db.list_my_vesting(&emp).unwrap().is_empty());
    let adm = w.admin.clone();
    assert_eq!(w.db.vesting_balance(&adm, &a.id).unwrap().vested, 0);

    // Activation (by an admin) applies the agreed schedule from start_at.
    let act = w.db.activate_vesting(&adm, &a.id).unwrap();
    assert_eq!(
        (act.status, act.activated_by.as_deref(), act.activated_at),
        (
            VestingStatus::Active,
            Some(adm.as_str()),
            Some(T0 + DAY / 2)
        )
    );
    assert!(matches!(
        w.db.activate_vesting(&adm, &a.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert_eq!(w.db.list_my_vesting(&emp).unwrap().len(), 1);
    w.clock.set(start + CLIFF);
    assert_eq!(w.bal(&a.id).vested, COIN);

    w.clock.advance(DAY);
    let e = w.employer.clone();
    let c = w.db.cancel_vesting(&e, &a.id).unwrap();
    assert_eq!(c.status, VestingStatus::Cancelled);
    assert_eq!(c.vested_at_cancel_zats, Some(COIN + 1_000_000));
    assert!(matches!(
        w.db.cancel_vesting(&e, &a.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        w.db.activate_vesting(&e, &a.id),
        Err(PayrollError::InvalidState(_))
    ));

    let events = w.db.vesting_events(&emp, &a.id).unwrap();
    let got: Vec<_> = events
        .iter()
        .map(|e| {
            (
                e.kind.as_str(),
                e.actor_user_id.as_str(),
                e.occurred_at,
                e.vested_zats_after,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("created", w.employer.as_str(), T0, 0),
            ("activated", w.admin.as_str(), T0 + DAY / 2, 0),
            (
                "cancelled",
                w.employer.as_str(),
                start + CLIFF + DAY,
                COIN + 1_000_000
            ),
        ]
    );
    // The audit log is append-only.
    for sql in [
        format!(
            "UPDATE vesting_events SET vested_zats_after = 0 WHERE agreement_id = '{}'",
            a.id
        ),
        format!("DELETE FROM vesting_events WHERE agreement_id = '{}'", a.id),
    ] {
        let err = w.f.raw(&sql).unwrap_err();
        assert!(err.contains("append-only"), "{err}");
    }
}

/// Cancellation stops vesting, keeps vested-but-unpaid compensation owed,
/// forfeits the rest, and leaves reserved and settled claims as they were.
fn cancellation_preserves_vested<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.standard();
    w.clock.advance(DURATION / 2); // 2 ZEC vested
    let paid = w.claim(&a.id, 50_000_000, "paid").unwrap();
    w.settle(&paid.id);
    let in_flight = w.claim(&a.id, 60_000_000, "in-flight").unwrap();
    let to_fail = w.claim(&a.id, 30_000_000, "to-fail").unwrap();

    let e = w.employer.clone();
    let c = w.db.cancel_vesting(&e, &a.id).unwrap();
    assert_eq!(c.vested_at_cancel_zats, Some(2 * COIN));
    assert_eq!(c.cancelled_by.as_deref(), Some(e.as_str()));
    let b = w.bal(&a.id);
    check_ledger(&b);
    assert_eq!(b.phase, VestingPhase::Cancelled);
    assert_eq!(
        (b.vested, b.unvested, b.paid, b.reserved, b.available),
        (2 * COIN, 2 * COIN, 50_000_000, 90_000_000, 60_000_000)
    );

    // Vesting has stopped for good.
    w.clock.advance(10 * DURATION);
    assert_eq!(w.bal(&a.id).vested, 2 * COIN);

    // Reserved claims still resolve normally: one is paid, one fails.
    let s = w.settle(&in_flight.id);
    assert_eq!(s.status, ClaimStatus::Settled);
    let emp = w.employee.clone();
    w.db.release_vesting_claim(&emp, &to_fail.id).unwrap();
    let b = w.bal(&a.id);
    assert_eq!(
        (b.paid, b.reserved, b.available),
        (110_000_000, 0, 90_000_000)
    );

    // The vested, unpaid remainder is still claimable — and nothing beyond it.
    assert_eq!(insufficient(w.claim(&a.id, 90_000_001, "late")), 90_000_000);
    w.claim(&a.id, 90_000_000, "late").unwrap();
    assert_eq!(insufficient(w.claim(&a.id, 1, "more")), 0);
    let b = w.bal(&a.id);
    check_ledger(&b);
    assert_eq!(b.reserved + b.paid, b.vested);
}

/// Cancelling before the start or during the cliff forfeits everything;
/// cancelling a draft voids it; a fully vested agreement can't be cancelled.
fn cancellation_edge_cases<F: Factory>(f: F) {
    let mut w = world(f);
    let e = w.employer.clone();

    let not_started = w.grant(TOTAL, DAY, DURATION, 0);
    let c = w.db.cancel_vesting(&e, &not_started.id).unwrap();
    assert_eq!(c.vested_at_cancel_zats, Some(0));

    let in_cliff = w.standard();
    w.clock.advance(CLIFF - 1);
    let c = w.db.cancel_vesting(&e, &in_cliff.id).unwrap();
    assert_eq!(c.vested_at_cancel_zats, Some(0));
    w.clock.advance(1 + DURATION);
    assert_eq!(w.bal(&in_cliff.id).vested, 0, "the cliff never arrives");
    assert_eq!(insufficient(w.claim(&in_cliff.id, 1, "k")), 0);

    let draft = w.draft(TOTAL, 0, DURATION, 0);
    w.clock.advance(DAY);
    let c = w.db.cancel_vesting(&e, &draft.id).unwrap();
    assert_eq!(
        (c.status, c.activated_at, c.vested_at_cancel_zats),
        (VestingStatus::Cancelled, None, Some(0))
    );
    let emp = w.employee.clone();
    assert!(
        matches!(
            w.db.get_vesting(&emp, &draft.id),
            Err(PayrollError::NotFound(_))
        ),
        "a voided draft stays invisible to the employee"
    );

    let full = w.grant(TOTAL, 0, DAY, 0);
    w.clock.advance(DAY);
    assert!(matches!(
        w.db.cancel_vesting(&e, &full.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert_eq!(w.bal(&full.id).available, TOTAL);
}

/// Claims racing a cancellation: the ledger stays consistent whichever wins.
fn concurrent_claims_and_cancel<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.grant(TOTAL, 0, DURATION, 0);
    w.clock.advance(DURATION / 2); // 2 ZEC vested, frozen at 2 ZEC by the cancel
    const CLAIMERS: usize = 8;
    let barrier = Arc::new(Barrier::new(CLAIMERS + 1));
    let mut handles = vec![];
    for i in 0..CLAIMERS {
        let (fac, clock, emp, id, barrier) = (
            w.f.clone(),
            w.clock.clone(),
            w.employee.clone(),
            a.id.clone(),
            barrier.clone(),
        );
        handles.push(std::thread::spawn(move || {
            let mut db = fac.open(&clock);
            barrier.wait();
            db.reserve_vesting_claim(&emp, &id, 30_000_000, &format!("t{i}"))
                .map(|_| ())
        }));
    }
    let (fac, clock, e, id, b2) = (
        w.f.clone(),
        w.clock.clone(),
        w.employer.clone(),
        a.id.clone(),
        barrier.clone(),
    );
    let cancel = std::thread::spawn(move || {
        let mut db = fac.open(&clock);
        b2.wait();
        db.cancel_vesting(&e, &id).map(|_| ())
    });
    for h in handles {
        let r = h.join().unwrap();
        assert!(
            matches!(r, Ok(()) | Err(PayrollError::InsufficientAvailable { .. })),
            "{r:?}"
        );
    }
    cancel.join().unwrap().unwrap();
    let b = w.bal(&a.id);
    check_ledger(&b);
    assert_eq!((b.phase, b.vested), (VestingPhase::Cancelled, 2 * COIN));
    assert_eq!(b.reserved, 180_000_000, "6 of 8 fit");
}

fn clock_skew_is_refused<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.standard();
    w.clock.set(T0 - 10);
    let e = w.employer.clone();
    assert!(matches!(
        w.db.cancel_vesting(&e, &a.id),
        Err(PayrollError::ClockSkew)
    ));
    let d = {
        w.clock.set(T0);
        w.draft(TOTAL, 0, DURATION, 0)
    };
    w.clock.set(T0 - 1);
    assert!(matches!(
        w.db.activate_vesting(&e, &d.id),
        Err(PayrollError::ClockSkew)
    ));
}

// ---- authorization ------------------------------------------------------------------

fn authorization<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.standard();
    w.clock.advance(CLIFF);
    let c = w.claim(&a.id, 10_000_000, "k1").unwrap();
    let (emp, col, stranger, rival, adm, e) = (
        w.employee.clone(),
        w.colleague.clone(),
        w.stranger.clone(),
        w.rival.clone(),
        w.admin.clone(),
        w.employer.clone(),
    );

    // Create: owners/admins only, for members only, never for themselves.
    let req = w.new_vesting(TOTAL, 0, DURATION, 0);
    for (who, want) in [
        (&emp, "forbidden"),
        (&col, "forbidden"),
        (&stranger, "nf"),
        (&rival, "nf"),
    ] {
        let r = w.db.create_vesting(who, req.clone());
        match want {
            "forbidden" => assert!(matches!(r, Err(PayrollError::Forbidden(_))), "{r:?}"),
            _ => assert!(
                matches!(r, Err(PayrollError::NotFound("organization"))),
                "{r:?}"
            ),
        }
    }
    let mut to_self = req.clone();
    to_self.employee_user_id = adm.clone();
    assert!(matches!(
        w.db.create_vesting(&adm, to_self),
        Err(PayrollError::Forbidden(_))
    ));
    for outsider in [&stranger, &rival, "not-a-uuid"] {
        let mut r = req.clone();
        r.employee_user_id = outsider.to_string();
        assert!(matches!(
            w.db.create_vesting(&e, r),
            Err(PayrollError::Validation(_))
        ));
    }
    // An org id supplied by the caller must be one the caller manages.
    let mut other_org = req.clone();
    other_org.organization_id = w.rival_org.clone();
    assert!(matches!(
        w.db.create_vesting(&e, other_org),
        Err(PayrollError::NotFound("organization"))
    ));

    // View: the employee and the org's managers; everyone else sees nothing.
    for who in [&emp, &adm, &e] {
        w.db.get_vesting(who, &a.id).unwrap();
        w.db.vesting_balance(who, &a.id).unwrap();
        w.db.vesting_events(who, &a.id).unwrap();
        w.db.list_vesting_claims(who, &a.id).unwrap();
    }
    for who in [&col, &stranger, &rival] {
        assert!(matches!(
            w.db.get_vesting(who, &a.id),
            Err(PayrollError::NotFound(_))
        ));
        assert!(matches!(
            w.db.vesting_balance(who, &a.id),
            Err(PayrollError::NotFound(_))
        ));
        assert!(matches!(
            w.db.vesting_events(who, &a.id),
            Err(PayrollError::NotFound(_))
        ));
        assert!(matches!(
            w.db.list_vesting_claims(who, &a.id),
            Err(PayrollError::NotFound(_))
        ));
        assert!(matches!(
            w.db.cancel_vesting(who, &a.id),
            Err(PayrollError::NotFound(_))
        ));
        assert!(matches!(
            w.db.reserve_vesting_claim(who, &a.id, 1, "x"),
            Err(PayrollError::NotFound(_))
        ));
        assert!(matches!(
            w.db.release_vesting_claim(who, &c.id),
            Err(PayrollError::NotFound("vesting claim"))
        ));
        assert!(matches!(
            w.db.settle_vesting_claim(who, &c.id, "x"),
            Err(PayrollError::NotFound("vesting claim"))
        ));
    }
    assert!(matches!(
        w.db.get_vesting(&e, "nope"),
        Err(PayrollError::NotFound(_))
    ));

    // Only managers cancel/activate; only the employee claims.
    assert!(matches!(
        w.db.cancel_vesting(&emp, &a.id),
        Err(PayrollError::Forbidden(_))
    ));
    for who in [&e, &adm] {
        assert!(matches!(
            w.db.reserve_vesting_claim(who, &a.id, 1, "mgr"),
            Err(PayrollError::Forbidden(_))
        ));
    }

    // Listings stay inside the organization.
    assert_eq!(
        w.db.list_org_vesting(&adm, &w.org.clone()).unwrap().len(),
        1
    );
    assert!(
        w.db.list_org_vesting(&rival, &w.rival_org.clone())
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        w.db.list_org_vesting(&rival, &w.org.clone()),
        Err(PayrollError::NotFound("organization"))
    ));
    assert!(matches!(
        w.db.list_org_vesting(&emp, &w.org.clone()),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(w.db.list_my_vesting(&col).unwrap().is_empty());
    let mine = w.db.list_my_vesting(&emp).unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(
        (mine[0].organization_name.as_str(), mine[0].balance.reserved),
        ("Acme", 10_000_000)
    );

    // Members with vesting agreements can't be removed (history stays intact).
    assert!(matches!(
        w.db.remove_member(&e, &w.org.clone(), &emp),
        Err(PayrollError::Conflict(m)) if m.contains("vesting")
    ));
}

/// Stream and vesting claims live apart: an id of one kind is unknown to the
/// other's operations (including the payout path), and vesting operations
/// never change a stream's accounting.
fn compensation_types_stay_apart<F: Factory>(f: F) {
    let mut w = world(f);
    let (e, emp) = (w.employer.clone(), w.employee.clone());
    let s =
        w.db.create_stream(
            &e,
            NewStream {
                organization_id: w.org.clone(),
                recipient_user_id: emp.clone(),
                rate_zats: COIN,
                rate_period_secs: DAY,
                start_at: None,
                end_at: None,
            },
        )
        .unwrap();
    let a = w.grant(TOTAL, 0, 2 * DAY, 0);
    w.clock.advance(DAY);
    let sc =
        w.db.reserve_claim(&emp, &s.id, COIN / 2, "same-key")
            .unwrap()
            .claim;
    let vc = w.claim(&a.id, COIN / 2, "same-key").unwrap();
    let stream_before = w.db.balance(&emp, &s.id).unwrap();

    assert!(matches!(
        w.db.release_vesting_claim(&emp, &sc.id),
        Err(PayrollError::NotFound("vesting claim"))
    ));
    assert!(matches!(
        w.db.release_claim(&emp, &vc.id),
        Err(PayrollError::NotFound("claim"))
    ));
    assert!(matches!(
        w.db.payout_eligibility(&emp, &vc.id, &|_| true),
        Err(PayrollError::NotFound("claim"))
    ));
    assert!(matches!(
        w.db.balance(&emp, &a.id),
        Err(PayrollError::NotFound("stream"))
    ));
    assert!(matches!(
        w.db.vesting_balance(&emp, &s.id),
        Err(PayrollError::NotFound("vesting agreement"))
    ));

    w.settle(&vc.id);
    w.db.cancel_vesting(&e, &a.id).unwrap();
    assert_eq!(w.db.balance(&emp, &s.id).unwrap(), stream_before);
    assert_eq!(w.db.list_claims(&emp, &s.id).unwrap(), vec![sc]);
}

// ---- schema -----------------------------------------------------------------------

/// The schema enforces the same rules as the engine, against direct writes.
fn schema_guards<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.standard();
    let draft = w.draft(TOTAL, 0, DURATION, 0);
    w.clock.advance(CLIFF);
    let c = w.claim(&a.id, COIN / 2, "k1").unwrap();
    let (emp, e, org) = (w.employee.clone(), w.employer.clone(), w.org.clone());
    let uid = || uuid::Uuid::new_v4().to_string();
    let now = w.clock.now();
    let claim_sql = |agreement: &str, by: &str, amount: i64, vested: i64| {
        format!(
            "INSERT INTO vesting_claims (id, agreement_id, requested_by, idempotency_key,
                 amount_zats, status, vested_at_reservation_zats, created_at)
             VALUES ('{}', '{agreement}', '{by}', 'raw-{}', {amount}, 'reserved', {vested}, {now})",
            uid(),
            uid()
        )
    };
    let id = &a.id;
    for (sql, want) in [
        (format!("UPDATE vesting_agreements SET total_zats = 1 WHERE id = '{id}'"), "immutable"),
        (format!("UPDATE vesting_agreements SET cliff_secs = 0 WHERE id = '{id}'"), "immutable"),
        (format!("DELETE FROM vesting_agreements WHERE id = '{id}'"), "cannot be deleted"),
        (format!("UPDATE vesting_agreements SET status = 'draft', activated_at = NULL, activated_by = NULL WHERE id = '{id}'"),
         "invalid vesting agreement transition"),
        // Rejected by the transition guard or (from v5) the no-late-activation guard.
        (format!("UPDATE vesting_agreements SET activated_at = activated_at + 1 WHERE id = '{id}'"),
         "invalid vesting agreement transition|activated before its start"),
        // Cancelling below what is already claimed.
        (format!("UPDATE vesting_agreements SET status = 'cancelled', cancelled_at = {now},
                      cancelled_by = '{e}', vested_at_cancel_zats = 1, updated_at = {now} WHERE id = '{id}'"),
         "freeze less"),
        // Over-claiming: 0.5 reserved + 0.6 > 1 ZEC vested.
        (claim_sql(id, &emp, 60_000_000, COIN), "exceed vested"),
        // Claiming more than the agreement can ever vest.
        (claim_sql(id, &emp, 1, TOTAL + 1), "within what can vest"),
        // Claims by someone other than the employee, or on a draft.
        (claim_sql(id, &e, 1, COIN), "by the employee"),
        (claim_sql(&draft.id, &emp, 1, COIN), "activated agreement"),
        // SQLite may fire either of two triggers first.
        (format!("UPDATE vesting_claims SET amount_zats = 1 WHERE id = '{}'", c.id),
         "immutable|invalid claim status transition"),
        (format!("DELETE FROM vesting_claims WHERE id = '{}'", c.id), "cannot be deleted"),
        (format!(
            "INSERT INTO vesting_agreements (id, organization_id, employee_user_id, created_by, total_zats,
                 start_at, duration_secs, cliff_secs, method, status, created_at, updated_at,
                 activated_at, activated_by)
             VALUES ('{}', '{org}', '{emp}', '{e}', 1, {now}, 10, 0, 'linear', 'active', {now}, {now}, {now}, '{e}')",
            uid()
        ), "must be created as drafts"),
    ] {
        let err = w.f.raw(&sql).unwrap_err();
        assert!(want.split('|').any(|x| err.contains(x)), "{sql}\n=> {err}");
    }
    // Plain CHECK constraints (backend-specific wording): float or negative
    // amounts, a cliff longer than the duration, an unknown method.
    for sql in [
        format!(
            "INSERT INTO vesting_agreements (id, organization_id, employee_user_id, created_by, total_zats,
                 start_at, duration_secs, cliff_secs, method, status, created_at, updated_at)
             VALUES ('{}', '{org}', '{emp}', '{e}', 1, {now}, 10, 11, 'linear', 'draft', {now}, {now})",
            uid()
        ),
        format!(
            "INSERT INTO vesting_agreements (id, organization_id, employee_user_id, created_by, total_zats,
                 start_at, duration_secs, cliff_secs, method, status, created_at, updated_at)
             VALUES ('{}', '{org}', '{emp}', '{e}', 1, {now}, 10, 0, 'milestone', 'draft', {now}, {now})",
            uid()
        ),
        format!(
            "INSERT INTO vesting_agreements (id, organization_id, employee_user_id, created_by, total_zats,
                 start_at, duration_secs, cliff_secs, method, status, created_at, updated_at)
             VALUES ('{}', '{org}', '{e}', '{e}', 1, {now}, 10, 0, 'linear', 'draft', {now}, {now})",
            uid()
        ),
        claim_sql(id, &emp, -1, COIN),
    ] {
        assert!(w.f.raw(&sql).is_err(), "{sql}");
    }
    // A cancelled agreement is final and caps new claims at the frozen amount.
    w.db.cancel_vesting(&e, id).unwrap();
    for (sql, want) in [
        (
            format!(
                "UPDATE vesting_agreements SET vested_at_cancel_zats = {TOTAL} WHERE id = '{id}'"
            ),
            "final",
        ),
        (claim_sql(id, &emp, 1, COIN + 1), "within what can vest"),
    ] {
        let err = w.f.raw(&sql).unwrap_err();
        assert!(err.contains(want), "{sql}\n=> {err}");
    }
    // The ledger is untouched by all of the above.
    let b = w.bal(id);
    check_ledger(&b);
    assert_eq!((b.vested, b.reserved), (COIN, COIN / 2));
}

macro_rules! scenarios {
    ($factory:ty $(, #[$attr:meta])?) => {
        #[test] $(#[$attr])? fn schedule_boundaries() { super::schedule_boundaries(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn small_allocation_reconciles() { super::small_allocation_reconciles(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn short_and_long_durations() { super::short_and_long_durations(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn claims_against_vested() { super::claims_against_vested(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn release_failed_reservation() { super::release_failed_reservation(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn duplicates_and_retries() { super::duplicates_and_retries(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn concurrent_claims() { super::concurrent_claims(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn settled_is_final() { super::settled_is_final(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn lifecycle_and_audit() { super::lifecycle_and_audit(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn cancellation_preserves_vested() { super::cancellation_preserves_vested(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn cancellation_edge_cases() { super::cancellation_edge_cases(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn concurrent_claims_and_cancel() { super::concurrent_claims_and_cancel(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn clock_skew_is_refused() { super::clock_skew_is_refused(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn authorization() { super::authorization(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn compensation_types_stay_apart() { super::compensation_types_stay_apart(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn schema_guards() { super::schema_guards(<$factory>::fresh()) }
    };
}

mod sqlite {
    use super::*;
    use hushush_payroll::sqlite::SqliteBackend;

    pub struct Sqlite {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
    }
    impl Fresh for Sqlite {
        fn fresh() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("p.sqlite");
            Self { _dir: dir, path }
        }
    }
    impl Factory for Sqlite {
        type B = SqliteBackend;
        fn open(&self, clock: &ManualClock) -> PayrollDb {
            PayrollDb::open(&self.path, Arc::new(clock.clone())).unwrap()
        }
        fn raw(&self, sql: &str) -> Result<(), String> {
            let c = rusqlite::Connection::open(&self.path).map_err(|e| e.to_string())?;
            c.execute_batch("PRAGMA foreign_keys = ON;")
                .map_err(|e| e.to_string())?;
            c.execute_batch(sql).map_err(|e| e.to_string())
        }
    }

    scenarios!(Sqlite);
}

mod postgres {
    use super::*;
    use hushush_payroll::pg::PgBackend;

    pub struct Pg {
        url: String,
        schema: String,
    }
    impl Fresh for Pg {
        fn fresh() -> Self {
            let url = std::env::var("PAYROLL_TEST_DATABASE_URL")
                .expect("set PAYROLL_TEST_DATABASE_URL to run PostgreSQL tests");
            let schema = format!("t_{}", uuid::Uuid::new_v4().simple());
            Self { url, schema }
        }
    }
    impl Factory for Pg {
        type B = PgBackend;
        fn open(&self, clock: &ManualClock) -> PgPayrollDb {
            PgPayrollDb::connect(&self.url, Some(&self.schema), Arc::new(clock.clone())).unwrap()
        }
        fn raw(&self, sql: &str) -> Result<(), String> {
            let mut c =
                hushush_payroll::pg::connect_client(&self.url).map_err(|e| e.to_string())?;
            hushush_payroll::pg::use_schema(&mut c, &self.schema).map_err(|e| e.to_string())?;
            c.batch_execute(sql).map_err(|e| {
                e.as_db_error()
                    .map_or_else(|| e.to_string(), |d| d.message().to_string())
            })
        }
    }
    impl Drop for Pg {
        fn drop(&mut self) {
            if let Ok(mut c) = hushush_payroll::pg::connect_client(&self.url) {
                let _ = c.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.schema));
            }
        }
    }

    scenarios!(Pg, #[ignore = "requires PAYROLL_TEST_DATABASE_URL"]);
}

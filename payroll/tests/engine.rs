//! Integration tests against real SQLite files. No network, no wallet.

use std::sync::{Arc, Barrier};

use hushush_payroll::clock::ManualClock;
use hushush_payroll::money::COIN;
use hushush_payroll::*;

const T0: i64 = 1_800_000_000; // fixed "now" for deterministic tests
const HOUR: i64 = 3_600;

struct Fx {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    clock: ManualClock,
    db: PayrollDb,
    owner: String,
    admin: String,
    worker: String,
    other: String,
    org: String,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payroll.sqlite");
    let clock = ManualClock::at(T0);
    let mut db = PayrollDb::open(&path, Arc::new(clock.clone())).unwrap();
    let owner = db.create_user("owner@example.com", "Owner").unwrap();
    let admin = db.create_user("admin@example.com", "Admin").unwrap();
    let worker = db.create_user("worker@example.com", "Worker").unwrap();
    let other = db.create_user("other@example.com", "Other").unwrap();
    let org = db.create_organization(&owner, "Acme").unwrap();
    db.add_member(&owner, &org, &admin, Role::Admin).unwrap();
    db.add_member(&admin, &org, &worker, Role::Member).unwrap();
    Fx {
        _dir: dir,
        path,
        clock,
        db,
        owner,
        admin,
        worker,
        other,
        org,
    }
}

/// 1 ZEC per hour stream for the worker.
fn hourly(f: &mut Fx, start: Option<i64>, end: Option<i64>) -> Stream {
    f.db.create_stream(
        &f.admin,
        NewStream {
            organization_id: f.org.clone(),
            recipient_user_id: f.worker.clone(),
            rate_zats: COIN,
            rate_period_secs: HOUR,
            start_at: start,
            end_at: end,
        },
    )
    .unwrap()
}

fn earned(f: &mut Fx, s: &Stream) -> i64 {
    f.db.balance(&f.worker, &s.id).unwrap().earned
}

// ---- accrual boundaries -------------------------------------------------------

#[test]
fn accrual_starts_exactly_at_start_time() {
    let mut f = fx();
    let s = hourly(&mut f, Some(T0 + 100), None);
    assert_eq!(earned(&mut f, &s), 0);
    f.clock.set(T0 + 100);
    assert_eq!(earned(&mut f, &s), 0);
    f.clock.set(T0 + 101);
    assert_eq!(earned(&mut f, &s), COIN / HOUR); // 27,777 zats for 1 second
    f.clock.set(T0 + 100 + HOUR);
    assert_eq!(earned(&mut f, &s), COIN);
}

#[test]
fn accrual_is_floored_integer_zatoshis() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(1);
    // 100,000,000 / 3,600 = 27,777.77… → 27,777 (never rounded up).
    assert_eq!(earned(&mut f, &s), 27_777);
    f.clock.advance(2);
    assert_eq!(earned(&mut f, &s), 83_333);
}

#[test]
fn end_date_caps_accrual_exactly() {
    let mut f = fx();
    let s = hourly(&mut f, None, Some(T0 + 2 * HOUR));
    f.clock.set(T0 + 2 * HOUR - 1);
    assert_eq!(earned(&mut f, &s), 2 * COIN - 27_777 - 1);
    f.clock.set(T0 + 2 * HOUR);
    assert_eq!(earned(&mut f, &s), 2 * COIN);
    f.clock.set(T0 + 100 * HOUR);
    let b = f.db.balance(&f.worker, &s.id).unwrap();
    assert_eq!(b.earned, 2 * COIN);
    assert_eq!(b.status, StreamStatus::Ended);
}

#[test]
fn rejects_retroactive_start_and_bad_terms() {
    let mut f = fx();
    let base = NewStream {
        organization_id: f.org.clone(),
        recipient_user_id: f.worker.clone(),
        rate_zats: COIN,
        rate_period_secs: HOUR,
        start_at: None,
        end_at: None,
    };
    let cases = [
        NewStream {
            start_at: Some(T0 - 1),
            ..base.clone()
        },
        NewStream {
            end_at: Some(T0),
            ..base.clone()
        },
        NewStream {
            rate_zats: 0,
            ..base.clone()
        },
        NewStream {
            rate_zats: -5,
            ..base.clone()
        },
        NewStream {
            rate_zats: 21_000_000 * COIN + 1,
            ..base.clone()
        },
        NewStream {
            rate_period_secs: 0,
            ..base.clone()
        },
    ];
    for c in cases {
        assert!(matches!(
            f.db.create_stream(&f.admin, c),
            Err(PayrollError::Validation(_))
        ));
    }
    let not_member = NewStream {
        recipient_user_id: f.other.clone(),
        ..base
    };
    assert!(matches!(
        f.db.create_stream(&f.admin, not_member),
        Err(PayrollError::Validation(_))
    ));
}

// ---- pause / resume / end -----------------------------------------------------

#[test]
fn pause_stops_accrual_and_keeps_earnings() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    f.db.pause_stream(&f.admin, &s.id).unwrap();
    assert_eq!(earned(&mut f, &s), COIN);
    f.clock.advance(10 * HOUR);
    assert_eq!(earned(&mut f, &s), COIN, "no accrual while paused");
    assert_eq!(
        f.db.balance(&f.worker, &s.id).unwrap().status,
        StreamStatus::Paused
    );
}

#[test]
fn resume_continues_from_resume_time() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    f.db.pause_stream(&f.admin, &s.id).unwrap();
    f.clock.advance(5 * HOUR);
    f.db.resume_stream(&f.admin, &s.id).unwrap();
    f.clock.advance(HOUR / 2);
    assert_eq!(earned(&mut f, &s), COIN + COIN / 2);
}

#[test]
fn many_short_pauses_lose_no_fractional_zatoshis() {
    let mut f = fx();
    // 1 zat per 7 seconds: any 1-second interval alone would floor to zero.
    let s =
        f.db.create_stream(
            &f.admin,
            NewStream {
                organization_id: f.org.clone(),
                recipient_user_id: f.worker.clone(),
                rate_zats: 1,
                rate_period_secs: 7,
                start_at: None,
                end_at: None,
            },
        )
        .unwrap();
    for _ in 0..70 {
        f.clock.advance(1);
        f.db.pause_stream(&f.admin, &s.id).unwrap();
        f.clock.advance(50);
        f.db.resume_stream(&f.admin, &s.id).unwrap();
    }
    f.db.pause_stream(&f.admin, &s.id).unwrap();
    assert_eq!(earned(&mut f, &s), 10); // 70 active seconds / 7
}

#[test]
fn end_stops_accrual_permanently_and_wages_stay_claimable() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(2 * HOUR);
    let ended = f.db.end_stream(&f.admin, &s.id).unwrap();
    assert_eq!(ended.status, StreamStatus::Ended);
    f.clock.advance(100 * HOUR);
    assert_eq!(earned(&mut f, &s), 2 * COIN);
    for r in [
        f.db.resume_stream(&f.admin, &s.id),
        f.db.pause_stream(&f.admin, &s.id),
        f.db.end_stream(&f.admin, &s.id),
    ] {
        assert!(matches!(r, Err(PayrollError::InvalidState(_))));
    }
    let c =
        f.db.reserve_claim(&f.worker, &s.id, 2 * COIN, "after-end")
            .unwrap();
    assert!(!c.duplicate);
    assert_eq!(f.db.balance(&f.worker, &s.id).unwrap().available, 0);
}

#[test]
fn ending_a_paused_stream_keeps_paused_total() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    f.db.pause_stream(&f.admin, &s.id).unwrap();
    f.clock.advance(HOUR);
    f.db.end_stream(&f.admin, &s.id).unwrap();
    assert_eq!(earned(&mut f, &s), COIN);
}

#[test]
fn pause_before_start_and_resume_waits_for_start() {
    let mut f = fx();
    let s = hourly(&mut f, Some(T0 + HOUR), None);
    f.db.pause_stream(&f.admin, &s.id).unwrap();
    f.clock.advance(HOUR / 2);
    f.db.resume_stream(&f.admin, &s.id).unwrap();
    f.clock.set(T0 + 2 * HOUR);
    assert_eq!(
        earned(&mut f, &s),
        COIN,
        "accrual counted only from start_at"
    );
}

#[test]
fn cannot_resume_after_end_date_passed_or_double_pause() {
    let mut f = fx();
    let s = hourly(&mut f, None, Some(T0 + HOUR));
    f.clock.advance(HOUR / 2);
    f.db.pause_stream(&f.admin, &s.id).unwrap();
    assert!(matches!(
        f.db.pause_stream(&f.admin, &s.id),
        Err(PayrollError::InvalidState(_))
    ));
    f.clock.set(T0 + 2 * HOUR);
    assert!(matches!(
        f.db.resume_stream(&f.admin, &s.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert_eq!(earned(&mut f, &s), COIN / 2);
}

#[test]
fn lifecycle_is_audited_in_order() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(10);
    f.db.pause_stream(&f.admin, &s.id).unwrap();
    f.clock.advance(10);
    f.db.resume_stream(&f.owner, &s.id).unwrap();
    f.clock.advance(10);
    f.db.end_stream(&f.admin, &s.id).unwrap();
    let ev = f.db.stream_events(&f.worker, &s.id).unwrap();
    let kinds: Vec<_> = ev.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["created", "paused", "resumed", "ended"]);
    assert_eq!(ev.last().unwrap().banked_active_secs_after, 20);
}

#[test]
fn clock_going_backwards_is_rejected() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(100);
    f.db.pause_stream(&f.admin, &s.id).unwrap();
    f.clock.advance(-50);
    assert!(matches!(
        f.db.resume_stream(&f.admin, &s.id),
        Err(PayrollError::ClockSkew)
    ));
}

// ---- claims -------------------------------------------------------------------

#[test]
fn claim_reserves_and_balances_add_up() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    let c =
        f.db.reserve_claim(&f.worker, &s.id, 30_000_000, "k1")
            .unwrap()
            .claim;
    assert_eq!(c.status, ClaimStatus::Reserved);
    let b = f.db.balance(&f.worker, &s.id).unwrap();
    assert_eq!(
        (b.earned, b.reserved, b.claimed, b.available),
        (COIN, 30_000_000, 0, 70_000_000)
    );

    f.db.settle_claim(&f.admin, &c.id, "phase2-payout-ref")
        .unwrap();
    let b = f.db.balance(&f.worker, &s.id).unwrap();
    assert_eq!(
        (b.reserved, b.claimed, b.available),
        (0, 30_000_000, 70_000_000)
    );
}

#[test]
fn cannot_reserve_more_than_available() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    f.db.reserve_claim(&f.worker, &s.id, 60_000_000, "a")
        .unwrap();
    let e =
        f.db.reserve_claim(&f.worker, &s.id, 40_000_001, "b")
            .unwrap_err();
    assert!(matches!(
        e,
        PayrollError::InsufficientAvailable {
            requested: 40_000_001,
            available: 40_000_000
        }
    ));
    f.db.reserve_claim(&f.worker, &s.id, 40_000_000, "c")
        .unwrap();
    assert_eq!(f.db.balance(&f.worker, &s.id).unwrap().available, 0);
}

#[test]
fn cannot_claim_unearned_future_wages() {
    let mut f = fx();
    let s = hourly(&mut f, Some(T0 + HOUR), None);
    assert!(matches!(
        f.db.reserve_claim(&f.worker, &s.id, 1, "early"),
        Err(PayrollError::InsufficientAvailable { available: 0, .. })
    ));
}

#[test]
fn release_returns_funds_and_is_one_way() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    let c =
        f.db.reserve_claim(&f.worker, &s.id, COIN, "k")
            .unwrap()
            .claim;
    assert_eq!(f.db.balance(&f.worker, &s.id).unwrap().available, 0);
    f.db.release_claim(&f.worker, &c.id).unwrap();
    assert_eq!(f.db.balance(&f.worker, &s.id).unwrap().available, COIN);
    assert!(matches!(
        f.db.release_claim(&f.worker, &c.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        f.db.settle_claim(&f.admin, &c.id, "x"),
        Err(PayrollError::InvalidState(_))
    ));
}

#[test]
fn duplicate_request_returns_original_claim() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    let a =
        f.db.reserve_claim(&f.worker, &s.id, 10_000_000, "req-123")
            .unwrap();
    let b =
        f.db.reserve_claim(&f.worker, &s.id, 10_000_000, "req-123")
            .unwrap();
    assert!(!a.duplicate && b.duplicate);
    assert_eq!(a.claim.id, b.claim.id);
    assert_eq!(f.db.list_claims(&f.worker, &s.id).unwrap().len(), 1);
    assert_eq!(f.db.balance(&f.worker, &s.id).unwrap().reserved, 10_000_000);
    // Same key, different amount: conflict, nothing created.
    assert!(matches!(
        f.db.reserve_claim(&f.worker, &s.id, 20_000_000, "req-123"),
        Err(PayrollError::IdempotencyConflict)
    ));
    // Replay after release still returns the original (released) claim.
    f.db.release_claim(&f.worker, &a.claim.id).unwrap();
    let c =
        f.db.reserve_claim(&f.worker, &s.id, 10_000_000, "req-123")
            .unwrap();
    assert!(c.duplicate && c.claim.status == ClaimStatus::Released);
}

#[test]
fn invalid_claim_inputs_rejected() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    for (amt, key) in [
        (0, "k"),
        (-1, "k"),
        (1, ""),
        (1, "has space"),
        (1, &"x".repeat(129)),
    ] {
        assert!(matches!(
            f.db.reserve_claim(&f.worker, &s.id, amt, key),
            Err(PayrollError::Validation(_))
        ));
    }
}

// ---- concurrency --------------------------------------------------------------

/// N threads, each with its own DB handle, race to reserve. Total reserved
/// must never exceed earned, and exactly floor(earned / amount) must succeed.
#[test]
fn concurrent_claims_never_over_reserve() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR); // earned = 1 ZEC
    const THREADS: usize = 16;
    const AMOUNT: i64 = 15_000_000; // 6 fit in 1 ZEC (90M), 7th would need 105M
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|i| {
            let (path, clock, worker, sid, barrier) = (
                f.path.clone(),
                f.clock.clone(),
                f.worker.clone(),
                s.id.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                let mut db = PayrollDb::open(&path, Arc::new(clock)).unwrap();
                barrier.wait();
                db.reserve_claim(&worker, &sid, AMOUNT, &format!("t{i}"))
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let ok = results.iter().filter(|r| r.is_ok()).count();
    let insufficient = results
        .iter()
        .filter(|r| matches!(r, Err(PayrollError::InsufficientAvailable { .. })))
        .count();
    assert_eq!(ok, 6, "{results:?}");
    assert_eq!(insufficient, THREADS - 6, "{results:?}");
    let b = f.db.balance(&f.worker, &s.id).unwrap();
    assert_eq!(b.reserved, 6 * AMOUNT);
    assert_eq!(b.available, COIN - 6 * AMOUNT);
}

/// The same request retried concurrently (e.g. double-click, client retry)
/// creates exactly one claim.
#[test]
fn concurrent_duplicate_requests_create_one_claim() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    const THREADS: usize = 12;
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let (path, clock, worker, sid, barrier) = (
                f.path.clone(),
                f.clock.clone(),
                f.worker.clone(),
                s.id.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                let mut db = PayrollDb::open(&path, Arc::new(clock)).unwrap();
                barrier.wait();
                db.reserve_claim(&worker, &sid, 50_000_000, "same-key")
                    .unwrap()
            })
        })
        .collect();
    let outs: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outs.iter().filter(|o| !o.duplicate).count(), 1);
    assert!(outs.iter().all(|o| o.claim.id == outs[0].claim.id));
    assert_eq!(f.db.list_claims(&f.worker, &s.id).unwrap().len(), 1);
    assert_eq!(f.db.balance(&f.worker, &s.id).unwrap().reserved, 50_000_000);
}

/// Claims racing a pause: whatever the interleaving, reserved <= earned.
#[test]
fn concurrent_claims_and_pause_keep_invariant() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    let barrier = Arc::new(Barrier::new(9));
    let mut handles = vec![];
    for i in 0..8 {
        let (path, clock, worker, sid, barrier) = (
            f.path.clone(),
            f.clock.clone(),
            f.worker.clone(),
            s.id.clone(),
            barrier.clone(),
        );
        handles.push(std::thread::spawn(move || {
            let mut db = PayrollDb::open(&path, Arc::new(clock)).unwrap();
            barrier.wait();
            let _ = db.reserve_claim(&worker, &sid, 20_000_000, &format!("p{i}"));
        }));
    }
    {
        let (path, clock, admin, sid, barrier) = (
            f.path.clone(),
            f.clock.clone(),
            f.admin.clone(),
            s.id.clone(),
            barrier.clone(),
        );
        handles.push(std::thread::spawn(move || {
            let mut db = PayrollDb::open(&path, Arc::new(clock)).unwrap();
            barrier.wait();
            db.pause_stream(&admin, &sid).unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let b = f.db.balance(&f.worker, &s.id).unwrap();
    assert_eq!(b.earned, COIN);
    assert_eq!(b.reserved, 5 * 20_000_000);
    assert_eq!(b.available, 0);
}

// ---- authorization ------------------------------------------------------------

#[test]
fn only_managers_manage_streams() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    assert!(matches!(
        f.db.pause_stream(&f.worker, &s.id),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        f.db.end_stream(&f.worker, &s.id),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        f.db.pause_stream(&f.other, &s.id),
        Err(PayrollError::NotFound(_))
    ));
    let req = NewStream {
        organization_id: f.org.clone(),
        recipient_user_id: f.worker.clone(),
        rate_zats: COIN,
        rate_period_secs: HOUR,
        start_at: None,
        end_at: None,
    };
    assert!(matches!(
        f.db.create_stream(&f.worker, req.clone()),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        f.db.create_stream(&f.other, req),
        Err(PayrollError::NotFound(_))
    ));
}

#[test]
fn only_recipient_claims_and_outsiders_see_nothing() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    assert!(matches!(
        f.db.reserve_claim(&f.admin, &s.id, 1, "x"),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        f.db.reserve_claim(&f.owner, &s.id, 1, "x"),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        f.db.reserve_claim(&f.other, &s.id, 1, "x"),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        f.db.balance(&f.other, &s.id),
        Err(PayrollError::NotFound(_))
    ));
    assert!(f.db.balance(&f.admin, &s.id).is_ok());

    let c =
        f.db.reserve_claim(&f.worker, &s.id, 1, "mine")
            .unwrap()
            .claim;
    assert!(matches!(
        f.db.settle_claim(&f.worker, &c.id, "self-pay"),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        f.db.release_claim(&f.other, &c.id),
        Err(PayrollError::NotFound(_))
    ));
}

#[test]
fn role_rules_for_membership() {
    let mut f = fx();
    let u = f.db.create_user("new@example.com", "New").unwrap();
    assert!(matches!(
        f.db.add_member(&f.admin, &f.org, &u, Role::Admin),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        f.db.add_member(&f.worker, &f.org, &u, Role::Member),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        f.db.add_member(&f.other, &f.org, &u, Role::Member),
        Err(PayrollError::NotFound(_))
    ));
    f.db.add_member(&f.owner, &f.org, &u, Role::Admin).unwrap();
    assert!(matches!(
        f.db.add_member(&f.owner, &f.org, &u, Role::Member),
        Err(PayrollError::Conflict(_))
    ));
    assert!(matches!(
        f.db.create_user("NEW@example.com", "Dup"),
        Err(PayrollError::Conflict(_))
    ));
    assert!(matches!(
        f.db.create_user("not-an-email", "x"),
        Err(PayrollError::Validation(_))
    ));
}

// ---- database constraints (defence in depth, bypassing the engine) -------------

fn raw_err(f: &Fx, sql: &str) -> String {
    f.db.connection()
        .execute_batch(sql)
        .expect_err(sql)
        .to_string()
}

#[test]
fn schema_rejects_tampering() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    let c =
        f.db.reserve_claim(&f.worker, &s.id, 1_000, "k")
            .unwrap()
            .claim;
    let sid = &s.id;
    let cid = &c.id;
    assert!(
        raw_err(
            &f,
            &format!("UPDATE payroll_streams SET rate_zats = 999999999999 WHERE id = '{sid}'")
        )
        .contains("immutable")
    );
    assert!(
        raw_err(
            &f,
            &format!("UPDATE payroll_streams SET banked_active_secs = -1 WHERE id = '{sid}'")
        )
        .contains("forward")
    );
    assert!(
        raw_err(
            &f,
            &format!("DELETE FROM payroll_streams WHERE id = '{sid}'")
        )
        .contains("cannot be deleted")
    );
    assert!(raw_err(&f, "DELETE FROM stream_events").contains("append-only"));
    assert!(raw_err(&f, "UPDATE stream_events SET earned_zats_after = 0").contains("append-only"));
    assert!(
        raw_err(
            &f,
            &format!("UPDATE claims SET amount_zats = 5 WHERE id = '{cid}'")
        )
        .contains("immutable")
    );
    assert!(
        raw_err(&f, &format!("DELETE FROM claims WHERE id = '{cid}'"))
            .contains("cannot be deleted")
    );
    // Floating-point money is rejected by STRICT typing.
    let w = &f.worker;
    assert!(raw_err(&f, &format!(
        "INSERT INTO claims (id, stream_id, requested_by, idempotency_key, amount_zats, status, earned_at_reservation_zats, created_at)
         VALUES ('00000000-0000-0000-0000-000000000001', '{sid}', '{w}', 'f', 1.5, 'reserved', 10, 0)"
    )).to_lowercase().contains("cannot store real"));
    // Overdraw is rejected even if the application computed a wrong balance.
    assert!(raw_err(&f, &format!(
        "INSERT INTO claims (id, stream_id, requested_by, idempotency_key, amount_zats, status, earned_at_reservation_zats, created_at)
         VALUES ('00000000-0000-0000-0000-000000000002', '{sid}', '{w}', 'g', 100, 'reserved', 1000, 0)"
    )).contains("exceed earned"));
    // Claims cannot be born settled, and settled is final.
    assert!(raw_err(&f, &format!(
        "INSERT INTO claims (id, stream_id, requested_by, idempotency_key, amount_zats, status, earned_at_reservation_zats, created_at, resolved_at, resolved_by, settlement_ref)
         VALUES ('00000000-0000-0000-0000-000000000003', '{sid}', '{w}', 'h', 1, 'settled', 1000000, 0, 0, '{w}', 'x')"
    )).contains("reserved state"));
    f.db.settle_claim(&f.admin, cid, "ref").unwrap();
    assert!(raw_err(&f, &format!("UPDATE claims SET status = 'reserved', resolved_at = NULL, resolved_by = NULL, settlement_ref = NULL WHERE id = '{cid}'")).contains("invalid claim status transition"));
    // Recipient must be an org member (composite FK).
    let o = &f.other;
    let org = &f.org;
    assert!(raw_err(&f, &format!(
        "INSERT INTO payroll_streams (id, organization_id, recipient_user_id, created_by, rate_zats, rate_period_secs, start_at, status, active_since, created_at, last_event_at)
         VALUES ('00000000-0000-0000-0000-000000000004', '{org}', '{o}', '{o}', 1, 1, 0, 'active', 0, 0, 0)"
    )).contains("FOREIGN KEY"));
}

#[test]
fn ended_streams_are_final_in_the_schema() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(10);
    f.db.end_stream(&f.admin, &s.id).unwrap();
    let sid = &s.id;
    assert!(raw_err(&f, &format!(
        "UPDATE payroll_streams SET status = 'active', active_since = start_at, ended_at = NULL WHERE id = '{sid}'"
    )).contains("final"));
}

// ---- migrations & persistence -------------------------------------------------

#[test]
fn migrations_are_idempotent_and_state_persists() {
    let mut f = fx();
    let s = hourly(&mut f, None, None);
    f.clock.advance(HOUR);
    f.db.reserve_claim(&f.worker, &s.id, 1_000, "persist")
        .unwrap();
    let path = f.path.clone();
    let clock = f.clock.clone();
    drop(f.db);
    // Reopen ("restart"): migrations are a no-op and balances are intact.
    let mut db = PayrollDb::open(&path, Arc::new(clock)).unwrap();
    let v: i64 = db
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, migrations::latest_version());
    let n: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    let b = db.balance(&f.worker, &s.id).unwrap();
    assert_eq!((b.earned, b.reserved), (COIN, 1_000));
}

#[test]
fn refuses_newer_schema() {
    let f = fx();
    f.db.connection()
        .execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (999, 'future', 0)",
            [],
        )
        .unwrap();
    let path = f.path.clone();
    drop(f.db);
    assert!(matches!(
        PayrollDb::open(&path, Arc::new(ManualClock::at(T0))),
        Err(PayrollError::SchemaTooNew { found: 999, .. })
    ));
}

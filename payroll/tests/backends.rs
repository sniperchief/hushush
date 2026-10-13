//! Backend-generic scenarios, run against BOTH storage backends.
//!
//! * `sqlite::*` always runs.
//! * `postgres::*` needs a reachable PostgreSQL and is `#[ignore]`d by default:
//!   `PAYROLL_TEST_DATABASE_URL=postgres://… cargo test --test backends -- --include-ignored`
//!   Each test runs in a fresh, randomly named schema that is dropped afterwards.
//!
//! tests/engine.rs (the Phase 1 suite) still covers SQLite in full.

use std::sync::{Arc, Barrier};

use hushush_payroll::clock::ManualClock;
use hushush_payroll::money::COIN;
use hushush_payroll::store::Backend;
use hushush_payroll::*;

const T0: i64 = 1_800_000_000;
const HOUR: i64 = 3_600;

/// Opens independent handles onto one shared database.
trait Factory: Send + Sync + 'static {
    type B: Backend;
    fn open(&self, clock: &ManualClock) -> Payroll<Self::B>;
}

struct World<F: Factory> {
    f: Arc<F>,
    clock: ManualClock,
    db: Payroll<F::B>,
    owner: String,
    admin: String,
    worker: String,
    org: String,
    // A second, unrelated organization.
    rival_owner: String,
    rival_org: String,
}

fn world<F: Factory>(f: F) -> World<F> {
    let f = Arc::new(f);
    let clock = ManualClock::at(T0);
    let mut db = f.open(&clock);
    let owner = db.create_user("owner@a.test", "Owner").unwrap();
    let admin = db.create_user("admin@a.test", "Admin").unwrap();
    let worker = db.create_user("worker@a.test", "Worker").unwrap();
    let rival_owner = db.create_user("owner@b.test", "Rival").unwrap();
    let org = db.create_organization(&owner, "Org A").unwrap();
    let rival_org = db.create_organization(&rival_owner, "Org B").unwrap();
    db.add_member(&owner, &org, &admin, Role::Admin).unwrap();
    db.add_member(&admin, &org, &worker, Role::Member).unwrap();
    World {
        f,
        clock,
        db,
        owner,
        admin,
        worker,
        org,
        rival_owner,
        rival_org,
    }
}

fn hourly<F: Factory>(w: &mut World<F>, end: Option<i64>) -> Stream {
    w.db.create_stream(
        &w.admin,
        NewStream {
            organization_id: w.org.clone(),
            recipient_user_id: w.worker.clone(),
            rate_zats: COIN,
            rate_period_secs: HOUR,
            start_at: None,
            end_at: end,
        },
    )
    .unwrap()
}

fn lifecycle_preserves_earnings<F: Factory>(f: F) {
    let mut w = world(f);
    let s = hourly(&mut w, None);
    w.clock.advance(HOUR);
    w.db.pause_stream(&w.admin, &s.id).unwrap();
    w.clock.advance(5 * HOUR);
    assert_eq!(w.db.balance(&w.worker, &s.id).unwrap().earned, COIN);
    w.db.resume_stream(&w.owner, &s.id).unwrap();
    w.clock.advance(HOUR / 2);
    w.db.end_stream(&w.admin, &s.id).unwrap();
    w.clock.advance(100 * HOUR);
    let b = w.db.balance(&w.worker, &s.id).unwrap();
    assert_eq!((b.earned, b.status), (COIN + COIN / 2, StreamStatus::Ended));
    let kinds: Vec<_> =
        w.db.stream_events(&w.admin, &s.id)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
    assert_eq!(kinds, ["created", "paused", "resumed", "ended"]);
    assert!(matches!(
        w.db.resume_stream(&w.admin, &s.id),
        Err(PayrollError::InvalidState(_))
    ));
}

fn end_date_caps<F: Factory>(f: F) {
    let mut w = world(f);
    let s = hourly(&mut w, Some(T0 + 2 * HOUR));
    w.clock.set(T0 + 10 * HOUR);
    let b = w.db.balance(&w.worker, &s.id).unwrap();
    assert_eq!((b.earned, b.status), (2 * COIN, StreamStatus::Ended));
}

fn no_backdating<F: Factory>(f: F) {
    let mut w = world(f);
    let r = w.db.create_stream(
        &w.admin,
        NewStream {
            organization_id: w.org.clone(),
            recipient_user_id: w.worker.clone(),
            rate_zats: COIN,
            rate_period_secs: HOUR,
            start_at: Some(T0 - 1),
            end_at: None,
        },
    );
    assert!(matches!(r, Err(PayrollError::Validation(_))));
}

fn claims_and_duplicates<F: Factory>(f: F) {
    let mut w = world(f);
    let s = hourly(&mut w, None);
    w.clock.advance(HOUR);
    let a =
        w.db.reserve_claim(&w.worker, &s.id, 30_000_000, "k1")
            .unwrap();
    let again =
        w.db.reserve_claim(&w.worker, &s.id, 30_000_000, "k1")
            .unwrap();
    assert!(!a.duplicate && again.duplicate && a.claim.id == again.claim.id);
    assert!(matches!(
        w.db.reserve_claim(&w.worker, &s.id, 1, "k1"),
        Err(PayrollError::IdempotencyConflict)
    ));
    assert!(matches!(
        w.db.reserve_claim(&w.worker, &s.id, 70_000_001, "k2"),
        Err(PayrollError::InsufficientAvailable {
            available: 70_000_000,
            ..
        })
    ));
    let b = w.db.balance(&w.worker, &s.id).unwrap();
    assert_eq!(
        (b.earned, b.reserved, b.claimed, b.available),
        (COIN, 30_000_000, 0, 70_000_000)
    );
    w.db.release_claim(&w.worker, &a.claim.id).unwrap();
    assert_eq!(w.db.balance(&w.worker, &s.id).unwrap().available, COIN);
    assert_eq!(
        w.db.list_claims(&w.admin, &s.id).unwrap()[0].status,
        ClaimStatus::Released
    );
}

fn concurrent_claims<F: Factory>(f: F) {
    let mut w = world(f);
    let s = hourly(&mut w, None);
    w.clock.advance(HOUR);
    const THREADS: usize = 16;
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|i| {
            let (fac, clock, worker, sid, barrier) = (
                w.f.clone(),
                w.clock.clone(),
                w.worker.clone(),
                s.id.clone(),
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
                db.reserve_claim(&worker, &sid, 15_000_000, &key)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let created = results
        .iter()
        .filter(|r| matches!(r, Ok(o) if !o.duplicate))
        .count();
    let b = w.db.balance(&w.worker, &s.id).unwrap();
    // 8 distinct keys + 1 shared key = 9 candidates; only 6 fit in 1 ZEC.
    assert_eq!(created, 6, "{results:?}");
    assert_eq!(b.reserved, 6 * 15_000_000);
    assert!(b.reserved <= b.earned);
    assert!(
        w.db.list_claims(&w.worker, &s.id)
            .unwrap()
            .iter()
            .filter(|c| c.idempotency_key == "retry")
            .count()
            <= 1
    );
}

fn cross_org_isolation<F: Factory>(f: F) {
    let mut w = world(f);
    let s = hourly(&mut w, None);
    w.clock.advance(HOUR);
    let rival = w.rival_owner.clone();
    assert!(matches!(
        w.db.balance(&rival, &s.id),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        w.db.pause_stream(&rival, &s.id),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        w.db.reserve_claim(&rival, &s.id, 1, "x"),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        w.db.list_org_streams(&rival, &w.org),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        w.db.list_members(&rival, &w.org),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        w.db.add_member(&rival, &w.org, &rival, Role::Owner),
        Err(PayrollError::NotFound(_))
    ));
    // An owner of org A cannot create a stream in org B or pay org B's people.
    assert!(matches!(
        w.db.create_stream(
            &w.owner,
            NewStream {
                organization_id: w.rival_org.clone(),
                recipient_user_id: w.rival_owner.clone(),
                rate_zats: 1,
                rate_period_secs: 1,
                start_at: None,
                end_at: None,
            }
        ),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        w.db.create_stream(
            &w.owner,
            NewStream {
                organization_id: w.org.clone(),
                recipient_user_id: w.rival_owner.clone(),
                rate_zats: 1,
                rate_period_secs: 1,
                start_at: None,
                end_at: None,
            }
        ),
        Err(PayrollError::Validation(_))
    ));
    let c =
        w.db.reserve_claim(&w.worker, &s.id, 1, "mine")
            .unwrap()
            .claim;
    assert!(matches!(
        w.db.release_claim(&rival, &c.id),
        Err(PayrollError::NotFound(_))
    ));
    assert!(w.db.list_my_streams(&rival).unwrap().is_empty());
}

fn roles_and_listing<F: Factory>(f: F) {
    let mut w = world(f);
    let s = hourly(&mut w, None);
    w.clock.advance(HOUR);
    // Members can't manage, list the directory, or claim others' wages.
    assert!(matches!(
        w.db.pause_stream(&w.worker, &s.id),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        w.db.list_members(&w.worker, &w.org),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        w.db.list_org_streams(&w.worker, &w.org),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        w.db.reserve_claim(&w.admin, &s.id, 1, "a"),
        Err(PayrollError::Forbidden(_))
    ));
    // Listings carry server-computed balances.
    let mine = w.db.list_my_streams(&w.worker).unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(
        (mine[0].balance.earned, mine[0].organization_name.as_str()),
        (COIN, "Org A")
    );
    let org_streams = w.db.list_org_streams(&w.admin, &w.org).unwrap();
    assert_eq!(org_streams[0].recipient_email, "worker@a.test");
    assert_eq!(
        w.db.stream_listing(&w.worker, &s.id)
            .unwrap()
            .balance
            .available,
        COIN
    );
    let orgs = w.db.list_organizations(&w.worker).unwrap();
    assert_eq!((orgs.len(), orgs[0].role), (1, Role::Member));
    assert_eq!(w.db.list_members(&w.admin, &w.org).unwrap().len(), 3);
}

fn member_management<F: Factory>(f: F) {
    let mut w = world(f);
    let extra = w.db.create_user("extra@a.test", "Extra").unwrap();
    w.db.add_member(&w.admin, &w.org, &extra, Role::Member)
        .unwrap();
    assert!(matches!(
        w.db.add_member(&w.admin, &w.org, &extra, Role::Member),
        Err(PayrollError::Conflict(_))
    ));
    // Only owners change roles; admins can't promote.
    assert!(matches!(
        w.db.set_member_role(&w.admin, &w.org, &extra, Role::Admin),
        Err(PayrollError::Forbidden(_))
    ));
    w.db.set_member_role(&w.owner, &w.org, &extra, Role::Admin)
        .unwrap();
    // Admins can't remove admins; owners can.
    assert!(matches!(
        w.db.remove_member(&w.admin, &w.org, &extra),
        Err(PayrollError::Forbidden(_))
    ));
    w.db.remove_member(&w.owner, &w.org, &extra).unwrap();
    // The last owner can't be demoted or removed.
    assert!(matches!(
        w.db.set_member_role(&w.owner, &w.org, &w.owner, Role::Member),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        w.db.remove_member(&w.owner, &w.org, &w.owner),
        Err(PayrollError::InvalidState(_))
    ));
    // Members with payroll history are kept.
    let _s = hourly(&mut w, None);
    assert!(matches!(
        w.db.remove_member(&w.owner, &w.org, &w.worker),
        Err(PayrollError::Conflict(_))
    ));
}

macro_rules! scenarios {
    ($factory:expr $(, #[$attr:meta])?) => {
        #[test] $(#[$attr])? fn lifecycle_preserves_earnings() { super::lifecycle_preserves_earnings($factory) }
        #[test] $(#[$attr])? fn end_date_caps() { super::end_date_caps($factory) }
        #[test] $(#[$attr])? fn no_backdating() { super::no_backdating($factory) }
        #[test] $(#[$attr])? fn claims_and_duplicates() { super::claims_and_duplicates($factory) }
        #[test] $(#[$attr])? fn concurrent_claims() { super::concurrent_claims($factory) }
        #[test] $(#[$attr])? fn cross_org_isolation() { super::cross_org_isolation($factory) }
        #[test] $(#[$attr])? fn roles_and_listing() { super::roles_and_listing($factory) }
        #[test] $(#[$attr])? fn member_management() { super::member_management($factory) }
    };
}

mod sqlite {
    use super::*;
    use hushush_payroll::sqlite::SqliteBackend;

    pub struct Sqlite {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
    }
    impl Sqlite {
        pub fn new() -> Self {
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
    }

    scenarios!(Sqlite::new());
}

mod postgres {
    use super::*;
    use hushush_payroll::pg::PgBackend;

    pub struct Pg {
        url: String,
        schema: String,
    }
    impl Pg {
        pub fn new() -> Self {
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
    }
    impl Drop for Pg {
        fn drop(&mut self) {
            if let Ok(mut c) = hushush_payroll::pg::connect_client(&self.url) {
                let _ = c.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.schema));
            }
        }
    }

    scenarios!(Pg::new(), #[ignore = "requires PAYROLL_TEST_DATABASE_URL"]);

    /// PostgreSQL schema enforces the same invariants as the SQLite one.
    #[test]
    #[ignore = "requires PAYROLL_TEST_DATABASE_URL"]
    fn schema_rejects_tampering() {
        let pg = Pg::new();
        let clock = ManualClock::at(T0);
        let mut db = pg.open(&clock);
        let owner = db.create_user("o@x.test", "O").unwrap();
        let worker = db.create_user("w@x.test", "W").unwrap();
        let org = db.create_organization(&owner, "X").unwrap();
        db.add_member(&owner, &org, &worker, Role::Member).unwrap();
        let s = db
            .create_stream(
                &owner,
                NewStream {
                    organization_id: org.clone(),
                    recipient_user_id: worker.clone(),
                    rate_zats: COIN,
                    rate_period_secs: HOUR,
                    start_at: None,
                    end_at: None,
                },
            )
            .unwrap();
        clock.advance(HOUR);
        let c = db.reserve_claim(&worker, &s.id, 1_000, "k").unwrap().claim;
        let has = |sql: String, needle: &str| {
            let e = client_err(&pg, &sql);
            assert!(e.contains(needle), "{sql}\n=> {e}");
        };
        has(
            format!(
                "UPDATE payroll_streams SET rate_zats = 5 WHERE id = '{}'",
                s.id
            ),
            "immutable",
        );
        has(
            format!("DELETE FROM payroll_streams WHERE id = '{}'", s.id),
            "cannot be deleted",
        );
        has("DELETE FROM stream_events".into(), "append-only");
        has(
            format!("UPDATE claims SET amount_zats = 5 WHERE id = '{}'", c.id),
            "immutable",
        );
        has(
            format!("DELETE FROM claims WHERE id = '{}'", c.id),
            "cannot be deleted",
        );
        has(format!(
            "INSERT INTO claims (id, stream_id, requested_by, idempotency_key, amount_zats, status, earned_at_reservation_zats, created_at)
             VALUES ('00000000-0000-0000-0000-000000000002', '{}', '{worker}', 'g', 100, 'reserved', 1000, 0)", s.id),
            "exceed earned");
        has(format!(
            "INSERT INTO payroll_streams (id, organization_id, recipient_user_id, created_by, rate_zats, rate_period_secs, start_at, status, active_since, created_at, last_event_at)
             VALUES ('00000000-0000-0000-0000-000000000004', '{org}', '{owner}x', '{owner}', 1, 1, 0, 'active', 0, 0, 0)"),
            "violates");
    }

    fn client_err(pg: &Pg, sql: &str) -> String {
        let mut c = hushush_payroll::pg::connect_client(&pg.url).unwrap();
        hushush_payroll::pg::use_schema(&mut c, &pg.schema).unwrap();
        let e = c.batch_execute(sql).expect_err(sql);
        match e.as_db_error() {
            Some(d) => d.message().to_string(),
            None => e.to_string(),
        }
    }
}

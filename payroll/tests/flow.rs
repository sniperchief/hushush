//! Phase 5 flow scenarios (offers, worker links, locked payout address,
//! milestone withdrawals), run against BOTH storage backends.
//!
//! *** The wallet here is a MOCK. *** [`MockWallet`] honours the wallet
//! contract (one transaction per payment id, fence on cancel) in memory and
//! counts every "transaction" it builds, so each scenario can assert that
//! nothing was paid twice. There are no keys, proofs or network. The full
//! payout state machine is covered by tests/payouts.rs; the real wallet by
//! payout-wallet's tests and the live testnet runs.
//!
//! * `sqlite::*` always runs.
//! * `postgres::*` needs `PAYROLL_TEST_DATABASE_URL` and is `#[ignore]`d by
//!   default (`cargo test --test flow -- --include-ignored`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Barrier, Mutex};

use hushush_payroll::clock::{Clock, ManualClock};
use hushush_payroll::money::COIN;
use hushush_payroll::payout::*;
use hushush_payroll::store::Backend;
use hushush_payroll::*;
use std::result::Result;

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
const ADDR: &str = "utest1mockflowrecipientaddress";
const OTHER_ADDR: &str = "utest1someoneelseentirely";
const FEE: i64 = 10_000;
/// 10 ZEC over 10 days: one milestone (20%) = 2 ZEC = 2 days.
const TOTAL: i64 = 10 * COIN;
const DURATION: i64 = 10 * DAY;

// ---- MOCK wallet ----------------------------------------------------------------

#[derive(Default)]
struct MockState {
    records: HashMap<String, WalletPayment>,
    /// Failure reasons for the next submits (no transaction built).
    fail_next: VecDeque<&'static str>,
    tip: i64,
    mined: HashMap<String, i64>,
    txs_built: usize,
}

/// MOCK of the payout wallet service. See the module docs.
#[derive(Default)]
struct MockWallet(Mutex<MockState>);

impl MockWallet {
    fn new() -> Arc<Self> {
        let w = Self::default();
        w.0.lock().unwrap().tip = 1_000;
        Arc::new(w)
    }
    fn txs_built(&self) -> usize {
        self.0.lock().unwrap().txs_built
    }
    fn fail_next(&self, why: &'static str) {
        self.0.lock().unwrap().fail_next.push_back(why);
    }
    /// Mines `txid` and buries it under `confs` confirmations.
    fn confirm(&self, txid: &str, confs: i64) {
        let mut s = self.0.lock().unwrap();
        let h = s.tip;
        s.mined.insert(txid.into(), h);
        s.tip += confs - 1;
    }
    fn view(s: &MockState, p: &WalletPayment, address: &str) -> WalletPayment {
        let mut p = p.clone();
        if let Some(txid) = &p.txid {
            let mined = s.mined.get(txid).copied();
            p.chain = Some(ChainStatus {
                tip_height: s.tip,
                mined_height: mined,
                confirmations: mined.map_or(0, |m| s.tip - m + 1),
                in_mempool: mined.is_none(),
                wallet_mined_height: mined,
                wallet_expired_unmined: Some(false),
            });
            p.verification = Some(TxVerification {
                source: "chain".into(),
                fully_shielded: true,
                single_pool: true,
                fee_zats: p.fee_zats,
                outputs: vec![VerifiedOutput {
                    recipient_fingerprint: recipient_fingerprint(&p.payment_id, address),
                    amount_zats: p.amount_zats,
                    memo_matches: true,
                    pool: "orchard".into(),
                }],
            });
        }
        p
    }
}

/// Address per payment id, as the wallet would know it.
static ADDRESSES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn remember(id: &str, addr: &str) {
    ADDRESSES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(id.into(), addr.into());
}
fn address_of(id: &str) -> String {
    ADDRESSES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(id).cloned())
        .unwrap_or_default()
}

impl WalletApi for MockWallet {
    fn submit(&self, req: &WalletSubmit) -> Result<WalletPayment, WalletError> {
        let mut s = self.0.lock().unwrap();
        if let Some(p) = s.records.get(&req.payment_id) {
            let p = p.clone();
            return Ok(Self::view(&s, &p, &req.address));
        }
        remember(&req.payment_id, &req.address);
        let mut p = WalletPayment {
            payment_id: req.payment_id.clone(),
            state: WalletPaymentState::Failed,
            recipient_fingerprint: recipient_fingerprint(&req.payment_id, &req.address),
            amount_zats: req.amount_zats,
            failure_reason: None,
            txid: None,
            fee_zats: None,
            expiry_height: None,
            chain: None,
            verification: None,
        };
        if let Some(why) = s.fail_next.pop_front() {
            p.failure_reason = Some(why.into());
        } else {
            s.txs_built += 1;
            p.state = WalletPaymentState::Broadcast;
            p.txid = Some(format!("{:064x}", s.txs_built));
            p.fee_zats = Some(FEE);
            p.expiry_height = Some(s.tip + 40);
        }
        s.records.insert(req.payment_id.clone(), p.clone());
        Ok(Self::view(&s, &p, &req.address))
    }
    fn lookup(&self, id: &str) -> Result<Option<WalletPayment>, WalletError> {
        let s = self.0.lock().unwrap();
        Ok(s.records
            .get(id)
            .map(|p| Self::view(&s, p, &address_of(id))))
    }
    fn cancel(&self, id: &str) -> Result<WalletPayment, WalletError> {
        let mut s = self.0.lock().unwrap();
        let p = s
            .records
            .entry(id.to_string())
            .or_insert_with(|| WalletPayment {
                payment_id: id.to_string(),
                state: WalletPaymentState::Failed,
                recipient_fingerprint: String::new(),
                amount_zats: 0,
                failure_reason: Some("cancelled".into()),
                txid: None,
                fee_zats: None,
                expiry_height: None,
                chain: None,
                verification: None,
            })
            .clone();
        Ok(Self::view(&s, &p, &address_of(id)))
    }
}

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
    wallet: Arc<MockWallet>,
    policy: PayoutPolicy,
    employer: String,
    worker: String,
    stranger: String,
    org: String,
    rival: String,
}

fn world<F: Factory>(f: F) -> World<F> {
    let f = Arc::new(f);
    let clock = ManualClock::at(T0);
    let mut db = f.open(&clock);
    let employer = db
        .create_user("e@users.hushush.invalid", "Employer")
        .unwrap();
    let worker = db.create_user("w@users.hushush.invalid", "Worker").unwrap();
    let stranger = db
        .create_user("s@users.hushush.invalid", "Stranger")
        .unwrap();
    let rival = db.create_user("r@users.hushush.invalid", "Rival").unwrap();
    let org = db.create_organization(&employer, "Acme").unwrap();
    db.create_organization(&rival, "Rival Co").unwrap();
    World {
        f,
        clock,
        db,
        wallet: MockWallet::new(),
        policy: PayoutPolicy::default(),
        employer,
        worker,
        stranger,
        org,
        rival,
    }
}

fn link(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn stream_fp(stream: &str, addr: &str) -> String {
    recipient_fingerprint(stream, addr)
}

const ALL_ORGS: &dyn Fn(&str) -> bool = &|_| true;

impl<F: Factory> World<F> {
    fn offer(&mut self, link_hash: &str) -> StreamOffer {
        self.db
            .create_offer(
                &self.employer.clone(),
                NewOffer {
                    organization_id: self.org.clone(),
                    worker_name: "Wendy".into(),
                    total_zats: TOTAL,
                    start_at: None,
                    end_at: self.clock.now() + DURATION,
                },
                link_hash,
            )
            .unwrap()
    }

    /// Offer, bound by the worker, with the payout address locked.
    fn bound_stream(&mut self) -> String {
        let l = link(1);
        self.offer(&l);
        let w = self.worker.clone();
        let s = self.db.bind_offer(&w, &l).unwrap().stream.id;
        self.db
            .set_payout_address(&w, &s, &stream_fp(&s, ADDR), "sealed-address")
            .unwrap();
        s
    }

    fn withdraw_as(
        &mut self,
        actor: &str,
        stream: &str,
        key: &str,
        amount: i64,
        addr: &str,
    ) -> hushush_payroll::Result<PayoutOutcome> {
        let fp = stream_fp(stream, addr);
        let a = addr.to_string();
        self.db.withdraw(
            actor,
            &WithdrawTerms {
                stream_id: stream,
                idempotency_key: key,
                amount_zats: amount,
                fee_estimate_zats: FEE,
                max_fee_zats: self.policy.max_fee_for(FEE).unwrap(),
                address_fingerprint: &fp,
                lease_owner: "api-1",
                lease_secs: self.policy.submit_lease_secs,
            },
            &move |id| recipient_fingerprint(id, &a),
            ALL_ORGS,
        )
    }

    /// The API's withdraw: everything available, then submit to the wallet.
    fn withdraw_all(&mut self, stream: &str, key: &str) -> hushush_payroll::Result<Payout> {
        let w = self.worker.clone();
        let avail = self.flow(stream).listing.balance.available;
        let out = self.withdraw_as(&w, stream, key, avail, ADDR)?;
        if out.duplicate {
            return Ok(out.payout);
        }
        submit_payout(&mut self.db, &*self.wallet, &self.policy, &out.payout, ADDR)
    }

    fn flow(&mut self, stream: &str) -> FlowStream {
        let w = self.worker.clone();
        self.db.flow_stream(&w, stream).unwrap()
    }

    fn confirm(&mut self, p: &Payout) -> Payout {
        self.wallet.confirm(
            p.txid.as_deref().unwrap(),
            self.policy.required_confirmations,
        );
        reconcile_due(&mut self.db, &*self.wallet, &self.policy, "worker-1", 50).unwrap();
        self.db.payout_internal(&p.id).unwrap()
    }
}

fn is_validation<T: std::fmt::Debug>(r: hushush_payroll::Result<T>) -> bool {
    matches!(r, Err(PayrollError::Validation(_)))
}

// ---- scenarios -----------------------------------------------------------------

/// Only the company's managers create offers; terms are validated.
fn offers_are_created_by_employers<F: Factory>(f: F) {
    let mut w = world(f);
    let now = w.clock.now();
    let req = |org: &str, total, start, end| NewOffer {
        organization_id: org.to_string(),
        worker_name: "Wendy".into(),
        total_zats: total,
        start_at: start,
        end_at: end,
    };
    let (e, r, s) = (w.employer.clone(), w.rival.clone(), w.stranger.clone());
    let org = w.org.clone();
    assert!(matches!(
        w.db.create_offer(&r, req(&org, TOTAL, None, now + DAY), &link(9)),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        w.db.create_offer(&s, req(&org, TOTAL, None, now + DAY), &link(9)),
        Err(PayrollError::NotFound(_))
    ));
    assert!(is_validation(w.db.create_offer(
        &e,
        req(&org, 0, None, now + DAY),
        &link(9)
    )));
    assert!(is_validation(w.db.create_offer(
        &e,
        req(&org, TOTAL, Some(now - 1), now + DAY),
        &link(9)
    )));
    assert!(is_validation(w.db.create_offer(
        &e,
        req(&org, TOTAL, None, now),
        &link(9)
    )));
    assert!(is_validation(w.db.create_offer(
        &e,
        req(&org, TOTAL, None, now + DAY),
        "nothex"
    )));

    let o = w.offer(&link(1));
    assert_eq!(o.status(), OfferStatus::Open);
    assert_eq!(o.organization_name, "Acme");
    assert_eq!((o.total_zats, o.end_at - o.start_at), (TOTAL, DURATION));
    // A link hash can't be reused by a second offer.
    assert!(matches!(
        w.db.create_offer(&e, req(&org, TOTAL, None, now + DAY), &link(1)),
        Err(PayrollError::Conflict(_))
    ));
    assert_eq!(w.db.list_offers(&e, &org).unwrap().len(), 1);
    assert!(w.db.list_offers(&r, &org).is_err());
}

/// The worker link binds once, to one worker, and creates the stream.
fn worker_link_binds_once<F: Factory>(f: F) {
    let mut w = world(f);
    let o = w.offer(&link(1));
    let (e, wk, s) = (w.employer.clone(), w.worker.clone(), w.stranger.clone());

    let p = w.db.offer_preview(&link(1)).unwrap();
    assert_eq!(
        (p.id.as_str(), p.status()),
        (o.id.as_str(), OfferStatus::Open)
    );
    assert!(matches!(
        w.db.offer_preview(&link(2)),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        w.db.offer_preview("zz"),
        Err(PayrollError::NotFound(_))
    ));

    // The employer can't take their own link.
    assert!(matches!(
        w.db.bind_offer(&e, &link(1)),
        Err(PayrollError::Forbidden(_))
    ));

    let b = w.db.bind_offer(&wk, &link(1)).unwrap();
    assert!(!b.duplicate);
    assert_eq!(b.stream.recipient_user_id, wk);
    assert_eq!(b.stream.organization_id, w.org);
    assert_eq!(b.stream.created_by, e);
    assert_eq!(b.offer.status(), OfferStatus::Bound);
    assert_eq!(b.offer.stream_id.as_deref(), Some(b.stream.id.as_str()));

    // Same worker again: the same stream. Anyone else: refused.
    let again = w.db.bind_offer(&wk, &link(1)).unwrap();
    assert!(again.duplicate);
    assert_eq!(again.stream.id, b.stream.id);
    assert!(matches!(
        w.db.bind_offer(&s, &link(1)),
        Err(PayrollError::Conflict(_))
    ));

    // The worker is now a plain member; their home shows the stream.
    let orgs = w.db.list_organizations(&wk).unwrap();
    assert_eq!(orgs[0].role, Role::Member);
    assert_eq!(w.db.worker_streams(&wk).unwrap().len(), 1);
    assert!(w.db.worker_streams(&s).unwrap().is_empty());
    assert_eq!(w.db.employer_streams(&e, &w.org.clone()).unwrap().len(), 1);
    // Bound offers can't be cancelled or relinked.
    assert!(matches!(
        w.db.cancel_offer(&e, &o.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        w.db.relink_offer(&e, &o.id, &link(3)),
        Err(PayrollError::InvalidState(_))
    ));

    // A second stream for the same worker from the same company is fine.
    w.offer(&link(4));
    assert!(!w.db.bind_offer(&wk, &link(4)).unwrap().duplicate);
    assert_eq!(w.db.worker_streams(&wk).unwrap().len(), 2);
}

/// Cancel and relink apply only to open offers.
fn cancel_and_relink<F: Factory>(f: F) {
    let mut w = world(f);
    let (e, wk, r) = (w.employer.clone(), w.worker.clone(), w.rival.clone());
    let o = w.offer(&link(1));
    assert!(w.db.relink_offer(&r, &o.id, &link(2)).is_err());
    w.db.relink_offer(&e, &o.id, &link(2)).unwrap();
    assert!(matches!(
        w.db.offer_preview(&link(1)),
        Err(PayrollError::NotFound(_))
    ));
    assert!(matches!(
        w.db.bind_offer(&wk, &link(1)),
        Err(PayrollError::NotFound(_))
    ));

    assert!(w.db.cancel_offer(&r, &o.id).is_err());
    let c = w.db.cancel_offer(&e, &o.id).unwrap();
    assert_eq!(c.status(), OfferStatus::Cancelled);
    assert!(matches!(
        w.db.bind_offer(&wk, &link(2)),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        w.db.cancel_offer(&e, &o.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(w.db.worker_streams(&wk).unwrap().is_empty());
}

/// The stream pays exactly the total at the end date, for awkward totals too.
fn total_is_exactly_reached<F: Factory>(f: F) {
    let mut w = world(f);
    let e = w.employer.clone();
    let total = 1_000_003; // not divisible by the duration
    let duration = 7 * DAY + 13;
    w.db.create_offer(
        &e,
        NewOffer {
            organization_id: w.org.clone(),
            worker_name: "Wendy".into(),
            total_zats: total,
            start_at: None,
            end_at: w.clock.now() + duration,
        },
        &link(1),
    )
    .unwrap();
    let wk = w.worker.clone();
    let s = w.db.bind_offer(&wk, &link(1)).unwrap().stream.id;
    w.clock.advance(duration - 1);
    assert!(w.flow(&s).listing.balance.earned < total);
    w.clock.advance(1);
    let f = w.flow(&s);
    assert_eq!(f.listing.balance.earned, total);
    assert!(f.ended());
    w.clock.advance(DAY);
    assert_eq!(w.flow(&s).listing.balance.earned, total);
}

/// A worker who joins late gets what accrued since the agreed start.
fn late_join_accrues_from_start<F: Factory>(f: F) {
    let mut w = world(f);
    w.offer(&link(1));
    w.clock.advance(DURATION / 2);
    let wk = w.worker.clone();
    let s = w.db.bind_offer(&wk, &link(1)).unwrap().stream.id;
    assert_eq!(w.flow(&s).listing.balance.earned, TOTAL / 2);
}

/// Address: recipient only, set once, locked, private to the worker.
fn payout_address_is_locked<F: Factory>(f: F) {
    let mut w = world(f);
    w.offer(&link(1));
    let (e, wk, s) = (w.employer.clone(), w.worker.clone(), w.stranger.clone());
    let st = w.db.bind_offer(&wk, &link(1)).unwrap().stream.id;
    let fp = stream_fp(&st, ADDR);
    assert!(matches!(
        w.db.set_payout_address(&e, &st, &fp, "x"),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        w.db.set_payout_address(&s, &st, &fp, "x"),
        Err(PayrollError::NotFound(_))
    ));
    assert!(!w.flow(&st).address_locked);

    let (a, dup) = w.db.set_payout_address(&wk, &st, &fp, "sealed-1").unwrap();
    assert!(!dup);
    assert_eq!(a.address_sealed, "sealed-1");
    let (_, dup) =
        w.db.set_payout_address(&wk, &st, &fp, "sealed-again")
            .unwrap();
    assert!(dup, "same address again is a no-op");
    assert!(matches!(
        w.db.set_payout_address(&wk, &st, &stream_fp(&st, OTHER_ADDR), "sealed-2"),
        Err(PayrollError::Conflict(_))
    ));
    // The worker sees their address; the employer only that it is locked.
    let mine = w.flow(&st);
    assert!(mine.address_locked);
    assert_eq!(mine.address.unwrap().address_sealed, "sealed-1");
    let theirs = w.db.flow_stream(&e, &st).unwrap();
    assert!(theirs.address_locked && theirs.address.is_none());

    // The database itself refuses to change, delete or forge it.
    for sql in [
        format!(
            "UPDATE stream_payout_addresses SET address_sealed = 'evil' WHERE stream_id = '{st}'"
        ),
        format!("DELETE FROM stream_payout_addresses WHERE stream_id = '{st}'"),
    ] {
        let e = w.f.raw(&sql).unwrap_err();
        assert!(e.contains("locked"), "{e}");
    }
    w.offer(&link(2));
    let st2 = w.db.bind_offer(&wk, &link(2)).unwrap().stream.id;
    let e2 =
        w.f.raw(&format!(
            "INSERT INTO stream_payout_addresses VALUES ('{st2}', '{e}', '{fp}', 'x', 0)"
        ))
        .unwrap_err();
    assert!(e2.contains("recipient"), "{e2}");
}

/// Withdrawals unlock at each 20% milestone; after the end, the remainder.
fn milestone_withdrawals<F: Factory>(f: F) {
    let mut w = world(f);
    let s = w.bound_stream();
    let wk = w.worker.clone();

    // Day 1: 10% earned — locked.
    w.clock.advance(DAY);
    assert_eq!(w.flow(&s).withdrawable_now(), None);
    assert!(matches!(
        w.withdraw_all(&s, "k1"),
        Err(PayrollError::InvalidState(_))
    ));

    // Day 2: 20% — the first milestone.
    w.clock.advance(DAY);
    assert_eq!(w.flow(&s).withdrawable_now(), Some(TOTAL / 5));
    let p1 = w.withdraw_all(&s, "k1").unwrap();
    assert_eq!(
        (p1.state, p1.amount_zats),
        (PayoutState::Submitted, TOTAL / 5)
    );
    // While it is unfinished, nothing else can be withdrawn.
    w.clock.advance(3 * DAY);
    assert_eq!(w.flow(&s).withdrawable_now(), None);
    let p1 = w.confirm(&p1);
    assert_eq!(p1.state, PayoutState::Confirmed);

    // Day 5: 3 days (30%) available — one bigger withdrawal.
    let p2 = w.withdraw_all(&s, "k2").unwrap();
    assert_eq!(p2.amount_zats, 3 * TOTAL / 10);
    w.confirm(&p2);

    // Day 6.5: 15% available — below a milestone.
    w.clock.advance(DAY + DAY / 2);
    assert!(matches!(
        w.withdraw_all(&s, "k3"),
        Err(PayrollError::InvalidState(_))
    ));
    // An explicit smaller-than-milestone amount is refused too.
    assert!(matches!(
        w.withdraw_as(&wk, &s, "k3", TOTAL / 10, ADDR),
        Err(PayrollError::InvalidState(_))
    ));
    // ... and more than available is refused.
    assert!(matches!(
        w.withdraw_as(&wk, &s, "k3", TOTAL, ADDR),
        Err(PayrollError::InsufficientAvailable { .. })
    ));

    // Employer ends the stream at day 6.5: the 15% remainder unlocks.
    let e = w.employer.clone();
    w.db.end_stream(&e, &s).unwrap();
    let f = w.flow(&s);
    assert!(f.ended());
    assert_eq!(f.withdrawable_now(), Some(TOTAL * 3 / 20));
    let p3 = w.withdraw_all(&s, "k3").unwrap();
    w.confirm(&p3);

    let f = w.flow(&s);
    assert_eq!(f.listing.balance.available, 0);
    assert_eq!(f.listing.balance.claimed, TOTAL * 13 / 20);
    assert_eq!(f.withdrawable_now(), None);
    assert_eq!(w.wallet.txs_built(), 3);
}

/// Double taps, refreshes (new key) and concurrent requests pay once.
fn withdraw_is_exactly_once<F: Factory>(f: F) {
    let mut w = world(f);
    let s = w.bound_stream();
    w.clock.advance(4 * DAY);
    let first = w.withdraw_all(&s, "tap").unwrap();
    // Same key: the same payout.
    let again = w.withdraw_all(&s, "tap").unwrap();
    assert_eq!(again.id, first.id);
    // A refresh with a new key while it is unfinished: still the same payout.
    let refresh = w.withdraw_all(&s, "refresh").unwrap();
    assert_eq!(refresh.id, first.id);
    assert_eq!(w.wallet.txs_built(), 1);

    // After it is paid, the old key still answers with that payout.
    w.confirm(&first);
    w.clock.advance(2 * DAY);
    assert_eq!(w.withdraw_all(&s, "tap").unwrap().id, first.id);

    // Eight concurrent requests with different keys: one withdrawal.
    let n = 8;
    let barrier = Arc::new(Barrier::new(n));
    let avail = w.flow(&s).listing.balance.available;
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let (fac, clock, b) = (w.f.clone(), w.clock.clone(), barrier.clone());
            let (wk, st, policy) = (w.worker.clone(), s.clone(), w.policy.clone());
            std::thread::spawn(move || {
                let mut db = fac.open(&clock);
                let fp = stream_fp(&st, ADDR);
                b.wait();
                db.withdraw(
                    &wk,
                    &WithdrawTerms {
                        stream_id: &st,
                        idempotency_key: &format!("c{i}"),
                        amount_zats: avail,
                        fee_estimate_zats: FEE,
                        max_fee_zats: policy.max_fee_for(FEE).unwrap(),
                        address_fingerprint: &fp,
                        lease_owner: "api-1",
                        lease_secs: policy.submit_lease_secs,
                    },
                    &|id| recipient_fingerprint(id, ADDR),
                    ALL_ORGS,
                )
                .unwrap()
            })
        })
        .collect();
    let outs: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outs.iter().filter(|o| !o.duplicate).count(), 1);
    let id = &outs[0].payout.id;
    assert!(outs.iter().all(|o| &o.payout.id == id));
    let f = w.flow(&s);
    assert_eq!(f.payouts.len(), 2);
    assert_eq!(f.listing.balance.reserved, avail);
}

/// A definite failure releases the amount; the worker can withdraw again.
fn failed_withdrawal_can_be_retried<F: Factory>(f: F) {
    let mut w = world(f);
    let s = w.bound_stream();
    w.clock.advance(2 * DAY);
    w.wallet.fail_next("insufficient funds");
    let p = w.withdraw_all(&s, "a").unwrap();
    assert_eq!(p.state, PayoutState::Failed);
    let f = w.flow(&s);
    assert_eq!(
        (f.listing.balance.available, f.listing.balance.reserved),
        (TOTAL / 5, 0)
    );
    let p2 = w.withdraw_all(&s, "b").unwrap();
    assert_eq!(p2.state, PayoutState::Submitted);
    assert_ne!(p2.id, p.id);
    assert_eq!(w.wallet.txs_built(), 1);
}

/// Who may withdraw, and to which address.
fn withdraw_authorization<F: Factory>(f: F) {
    let mut w = world(f);
    w.offer(&link(1));
    let (e, wk, s) = (w.employer.clone(), w.worker.clone(), w.stranger.clone());
    let st = w.db.bind_offer(&wk, &link(1)).unwrap().stream.id;
    w.clock.advance(3 * DAY);
    // No address yet.
    assert!(matches!(
        w.withdraw_as(&wk, &st, "k", TOTAL / 5, ADDR),
        Err(PayrollError::InvalidState(_))
    ));
    w.db.set_payout_address(&wk, &st, &stream_fp(&st, ADDR), "sealed")
        .unwrap();
    assert!(matches!(
        w.withdraw_as(&e, &st, "k", TOTAL / 5, ADDR),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        w.withdraw_as(&s, &st, "k", TOTAL / 5, ADDR),
        Err(PayrollError::NotFound(_))
    ));
    // Paying any address but the locked one is refused.
    assert!(matches!(
        w.withdraw_as(&wk, &st, "k", TOTAL / 5, OTHER_ADDR),
        Err(PayrollError::InvalidState(_))
    ));
    // Org not enabled for payouts.
    let fp = stream_fp(&st, ADDR);
    let r = w.db.withdraw(
        &wk,
        &WithdrawTerms {
            stream_id: &st,
            idempotency_key: "k",
            amount_zats: TOTAL / 5,
            fee_estimate_zats: FEE,
            max_fee_zats: FEE,
            address_fingerprint: &fp,
            lease_owner: "api-1",
            lease_secs: 600,
        },
        &|id| recipient_fingerprint(id, ADDR),
        &|_| false,
    );
    assert!(matches!(r, Err(PayrollError::Forbidden(_))));
    assert_eq!(w.wallet.txs_built(), 0);
}

/// The Phase 2/3 claim path and pause/resume don't apply to fixed-total
/// streams (they would bypass the milestone rule or make the total
/// unreachable); ordinary streams are unaffected.
fn legacy_paths_are_closed<F: Factory>(f: F) {
    let mut w = world(f);
    let s = w.bound_stream();
    let (e, wk) = (w.employer.clone(), w.worker.clone());
    w.clock.advance(3 * DAY);
    assert!(matches!(
        w.db.reserve_claim(&wk, &s, 1, "legacy"),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        w.db.pause_stream(&e, &s),
        Err(PayrollError::InvalidState(_))
    ));
    w.db.end_stream(&e, &s).unwrap();

    // An ordinary Phase 1-4 stream still pauses and claims.
    let plain =
        w.db.create_stream(
            &e,
            NewStream {
                organization_id: w.org.clone(),
                recipient_user_id: wk.clone(),
                rate_zats: COIN,
                rate_period_secs: DAY,
                start_at: None,
                end_at: None,
            },
        )
        .unwrap()
        .id;
    w.clock.advance(DAY);
    w.db.pause_stream(&e, &plain).unwrap();
    w.db.reserve_claim(&wk, &plain, 1, "ok").unwrap();
    // ... and is not listed as a fixed-total stream.
    assert_eq!(w.db.worker_streams(&wk).unwrap().len(), 1);
}

/// Offer rows are append-only where it matters.
fn offer_schema_guards<F: Factory>(f: F) {
    let mut w = world(f);
    let o = w.offer(&link(1));
    let id = o.id.clone();
    for (sql, want) in [
        (
            format!("UPDATE stream_offers SET total_zats = 1 WHERE id = '{id}'"),
            "immutable",
        ),
        (
            format!("DELETE FROM stream_offers WHERE id = '{id}'"),
            "cannot be deleted",
        ),
    ] {
        let e = w.f.raw(&sql).unwrap_err();
        assert!(e.contains(want), "{e}");
    }
    let wk = w.worker.clone();
    w.db.bind_offer(&wk, &link(1)).unwrap();
    let e =
        w.f.raw(&format!(
            "UPDATE stream_offers SET link_hash = '{}' WHERE id = '{id}'",
            link(7)
        ))
        .unwrap_err();
    assert!(e.contains("final"), "{e}");
}

macro_rules! scenarios {
    ($factory:ty $(, #[$attr:meta])?) => {
        #[test] $(#[$attr])? fn offers_are_created_by_employers() { super::offers_are_created_by_employers(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn worker_link_binds_once() { super::worker_link_binds_once(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn cancel_and_relink() { super::cancel_and_relink(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn total_is_exactly_reached() { super::total_is_exactly_reached(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn late_join_accrues_from_start() { super::late_join_accrues_from_start(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn payout_address_is_locked() { super::payout_address_is_locked(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn milestone_withdrawals() { super::milestone_withdrawals(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn withdraw_is_exactly_once() { super::withdraw_is_exactly_once(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn failed_withdrawal_can_be_retried() { super::failed_withdrawal_can_be_retried(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn withdraw_authorization() { super::withdraw_authorization(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn legacy_paths_are_closed() { super::legacy_paths_are_closed(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn offer_schema_guards() { super::offer_schema_guards(<$factory>::fresh()) }
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

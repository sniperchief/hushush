//! Payout state machine scenarios (Phase 3), run against BOTH storage backends.
//!
//! *** The wallet here is a MOCK. *** [`MockWallet`] simulates the payout
//! wallet service's contract (idempotent per payment id, fence on cancel) and
//! a toy chain (tip height, mining, mempool, expiry) in memory. There are no
//! keys, proofs, transactions or network. It counts every "transaction" it
//! builds so each scenario can assert that nothing was ever paid twice. The
//! real wallet is exercised by payout-wallet's tests and the live testnet run.
//!
//! * `sqlite::*` always runs.
//! * `postgres::*` needs `PAYROLL_TEST_DATABASE_URL` and is `#[ignore]`d by
//!   default (`cargo test --test payouts -- --include-ignored`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Barrier, Mutex};

use hushush_payroll::clock::ManualClock;
use hushush_payroll::money::COIN;
use hushush_payroll::payout::*;
use hushush_payroll::store::Backend;
use hushush_payroll::*;
use std::result::Result;

const T0: i64 = 1_800_000_000;
const HOUR: i64 = 3_600;
const ADDR: &str = "utest1mockrecipientaddressforpayouttests";
const OTHER_ADDR: &str = "utest1someoneelseentirely";
const FEE: i64 = 10_000;

// ---- MOCK wallet ----------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum SubmitMode {
    /// Built and accepted by the network.
    Broadcast,
    /// Built and stored, but the network has not acknowledged it.
    BuiltOnly,
    /// Rejected before any transaction was built.
    FailNoTx(&'static str),
    /// Built and broadcast, but the response never reached the caller.
    LostResponse,
    /// The request never reached the wallet.
    LostRequest,
}

#[derive(Clone, Copy, Debug)]
enum Tamper {
    Amount,
    Recipient,
    PoolCrossing,
    ExtraOutput,
}

#[derive(Clone)]
struct Rec {
    pay: WalletPayment,
    address: Option<String>,
}

#[derive(Default)]
struct MockState {
    records: HashMap<String, Rec>,
    next_submit: VecDeque<SubmitMode>,
    tip: i64,
    mined: HashMap<String, i64>,
    dropped: HashSet<String>,
    txs_built: usize,
    unavailable: bool,
    tamper: Option<Tamper>,
    submits: usize,
    /// The wallet DB has no view of the chain (fresh restore / not synced).
    wallet_blind: bool,
    /// The wallet's own scan saw these txids mined although the node says
    /// otherwise (a node on a stale fork).
    wallet_saw_mined: HashSet<String>,
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
    fn then(&self, m: SubmitMode) {
        self.0.lock().unwrap().next_submit.push_back(m);
    }
    fn txs_built(&self) -> usize {
        self.0.lock().unwrap().txs_built
    }
    fn set_tip(&self, h: i64) {
        self.0.lock().unwrap().tip = h;
    }
    fn tip(&self) -> i64 {
        self.0.lock().unwrap().tip
    }
    fn mine(&self, txid: &str) {
        let mut s = self.0.lock().unwrap();
        let h = s.tip;
        s.mined.insert(txid.into(), h);
    }
    fn unmine(&self, txid: &str) {
        self.0.lock().unwrap().mined.remove(txid);
    }
    fn drop_from_mempool(&self, txid: &str) {
        self.0.lock().unwrap().dropped.insert(txid.into());
    }
    fn set_unavailable(&self, v: bool) {
        self.0.lock().unwrap().unavailable = v;
    }
    fn set_wallet_blind(&self, v: bool) {
        self.0.lock().unwrap().wallet_blind = v;
    }
    fn wallet_saw_mined(&self, txid: &str) {
        self.0.lock().unwrap().wallet_saw_mined.insert(txid.into());
    }
    fn tamper(&self, t: Tamper) {
        self.0.lock().unwrap().tamper = Some(t);
    }
    /// The wallet's own background rebroadcast succeeding.
    fn acknowledge_broadcasts(&self) {
        for r in self.0.lock().unwrap().records.values_mut() {
            if r.pay.state == WalletPaymentState::Built {
                r.pay.state = WalletPaymentState::Broadcast;
            }
        }
    }
    fn view(s: &MockState, r: &Rec) -> WalletPayment {
        let mut p = r.pay.clone();
        if let Some(txid) = &p.txid {
            let mined = s.mined.get(txid).copied();
            let expiry = p.expiry_height.unwrap_or(i64::MAX);
            p.chain = Some(ChainStatus {
                tip_height: s.tip,
                mined_height: mined,
                confirmations: mined.map_or(0, |m| s.tip - m + 1),
                in_mempool: mined.is_none()
                    && p.state == WalletPaymentState::Broadcast
                    && !s.dropped.contains(txid)
                    && s.tip <= expiry,
                wallet_mined_height: if s.wallet_saw_mined.contains(txid) {
                    Some(s.tip)
                } else {
                    mined
                },
                wallet_expired_unmined: (!s.wallet_blind).then(|| {
                    mined.is_none() && !s.wallet_saw_mined.contains(txid) && s.tip > expiry
                }),
            });
            let mut out = VerifiedOutput {
                recipient_fingerprint: recipient_fingerprint(
                    &p.payment_id,
                    r.address.as_deref().unwrap_or(""),
                ),
                amount_zats: p.amount_zats,
                memo_matches: true,
                pool: "ironwood".into(),
            };
            let mut v = TxVerification {
                source: if mined.is_some() { "chain" } else { "local" }.into(),
                fully_shielded: true,
                single_pool: true,
                fee_zats: p.fee_zats,
                outputs: vec![],
            };
            match s.tamper {
                Some(Tamper::Amount) => out.amount_zats -= 1,
                Some(Tamper::Recipient) => {
                    out.recipient_fingerprint = recipient_fingerprint(&p.payment_id, OTHER_ADDR)
                }
                Some(Tamper::PoolCrossing) => v.single_pool = false,
                Some(Tamper::ExtraOutput) => v.outputs.push(out.clone()),
                None => {}
            }
            v.outputs.push(out);
            p.verification = Some(v);
        }
        p
    }
}

impl WalletApi for MockWallet {
    fn submit(&self, req: &WalletSubmit) -> Result<WalletPayment, WalletError> {
        let mut s = self.0.lock().unwrap();
        s.submits += 1;
        if s.unavailable {
            return Err(WalletError::Unavailable("connection refused".into()));
        }
        if let Some(r) = s.records.get(&req.payment_id) {
            // Idempotent: the same id never builds a second transaction.
            let same = r.address.as_deref() == Some(req.address.as_str())
                && r.pay.amount_zats == req.amount_zats;
            if r.address.is_some() && !same {
                return Err(WalletError::Refused {
                    code: "idempotency_conflict".into(),
                    message: "payment id reused with different parameters".into(),
                });
            }
            let r = r.clone();
            return Ok(Self::view(&s, &r));
        }
        let mode = s.next_submit.pop_front().unwrap_or(SubmitMode::Broadcast);
        if let SubmitMode::LostRequest = mode {
            return Err(WalletError::Unavailable("timed out".into()));
        }
        let fp = recipient_fingerprint(&req.payment_id, &req.address);
        let mut pay = WalletPayment {
            payment_id: req.payment_id.clone(),
            state: WalletPaymentState::Failed,
            recipient_fingerprint: fp,
            amount_zats: req.amount_zats,
            failure_reason: None,
            txid: None,
            fee_zats: None,
            expiry_height: None,
            chain: None,
            verification: None,
        };
        match mode {
            SubmitMode::FailNoTx(why) => pay.failure_reason = Some(why.into()),
            _ => {
                s.txs_built += 1;
                pay.state = match mode {
                    SubmitMode::BuiltOnly => WalletPaymentState::Built,
                    _ => WalletPaymentState::Broadcast,
                };
                pay.txid = Some(format!("{:064x}", s.txs_built));
                pay.fee_zats = Some(FEE);
                pay.expiry_height = Some(s.tip + 40);
            }
        }
        let rec = Rec {
            pay,
            address: Some(req.address.clone()),
        };
        s.records.insert(req.payment_id.clone(), rec.clone());
        if let SubmitMode::LostResponse = mode {
            return Err(WalletError::Unavailable("connection reset".into()));
        }
        Ok(Self::view(&s, &rec))
    }

    fn lookup(&self, id: &str) -> Result<Option<WalletPayment>, WalletError> {
        let s = self.0.lock().unwrap();
        if s.unavailable {
            return Err(WalletError::Unavailable("connection refused".into()));
        }
        Ok(s.records.get(id).map(|r| Self::view(&s, r)))
    }

    fn cancel(&self, id: &str) -> Result<WalletPayment, WalletError> {
        let mut s = self.0.lock().unwrap();
        if s.unavailable {
            return Err(WalletError::Unavailable("connection refused".into()));
        }
        let rec = s.records.entry(id.to_string()).or_insert_with(|| Rec {
            pay: WalletPayment {
                payment_id: id.to_string(),
                state: WalletPaymentState::Failed,
                recipient_fingerprint: String::new(),
                amount_zats: 0,
                failure_reason: Some("cancelled before any transaction was built".into()),
                txid: None,
                fee_zats: None,
                expiry_height: None,
                chain: None,
                verification: None,
            },
            address: None,
        });
        let rec = rec.clone();
        Ok(Self::view(&s, &rec))
    }
}

// ---- world --------------------------------------------------------------------

trait Factory: Fresh + Send + Sync + 'static {
    type B: Backend;
    fn open(&self, clock: &ManualClock) -> Payroll<Self::B>;
    /// Runs raw SQL as a direct database user would; returns the error text.
    fn raw(&self, sql: &str) -> Result<(), String>;
    /// Makes every claim status change fail (simulates a crash in the middle
    /// of settling a claim), or removes that fault.
    fn settlement_fault(&self, on: bool);
}

struct World<F: Factory> {
    f: Arc<F>,
    clock: ManualClock,
    db: Payroll<F::B>,
    wallet: Arc<MockWallet>,
    policy: PayoutPolicy,
    owner: String,
    worker: String,
    other_worker: String,
    org: String,
    rival_owner: String,
    stream: String,
}

fn world<F: Factory>(f: F) -> World<F> {
    let f = Arc::new(f);
    let clock = ManualClock::at(T0);
    let mut db = f.open(&clock);
    let owner = db.create_user("owner@a.test", "Owner").unwrap();
    let worker = db.create_user("worker@a.test", "Worker").unwrap();
    let other_worker = db.create_user("other@a.test", "Other").unwrap();
    let rival_owner = db.create_user("owner@b.test", "Rival").unwrap();
    let org = db.create_organization(&owner, "Org A").unwrap();
    db.create_organization(&rival_owner, "Org B").unwrap();
    db.add_member(&owner, &org, &worker, Role::Member).unwrap();
    db.add_member(&owner, &org, &other_worker, Role::Member)
        .unwrap();
    let stream = db
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
        .unwrap()
        .id;
    clock.advance(HOUR); // 1 ZEC earned
    World {
        f,
        clock,
        db,
        wallet: MockWallet::new(),
        policy: PayoutPolicy::default(),
        owner,
        worker,
        other_worker,
        org,
        rival_owner,
        stream,
    }
}

const ALL_ORGS: &dyn Fn(&str) -> bool = &|_| true;

impl<F: Factory> World<F> {
    fn claim(&mut self, amount: i64) -> Claim {
        let key = uuid::Uuid::new_v4().to_string();
        self.db
            .reserve_claim(&self.worker, &self.stream, amount, &key)
            .unwrap()
            .claim
    }

    fn request_as(
        &mut self,
        actor: &str,
        claim: &str,
        addr: &str,
    ) -> hushush_payroll::Result<PayoutOutcome> {
        let addr = addr.to_string();
        self.db.request_payout(
            actor,
            &PayoutTerms {
                claim_id: claim,
                fee_estimate_zats: FEE,
                max_fee_zats: self.policy.max_fee_for(FEE).unwrap(),
                lease_owner: "api-1",
                lease_secs: self.policy.submit_lease_secs,
            },
            &move |id| recipient_fingerprint(id, &addr),
            ALL_ORGS,
        )
    }

    /// Requests a payout as the claimant and submits it, like the API handler.
    fn pay(&mut self, claim: &str) -> Payout {
        let worker = self.worker.clone();
        let out = self.request_as(&worker, claim, ADDR).unwrap();
        assert!(!out.duplicate);
        submit_payout(&mut self.db, &*self.wallet, &self.policy, &out.payout, ADDR).unwrap()
    }

    fn reconcile(&mut self, owner: &str) -> Vec<Payout> {
        reconcile_due(&mut self.db, &*self.wallet, &self.policy, owner, 50).unwrap()
    }

    fn payout(&mut self, id: &str) -> Payout {
        self.db.payout_internal(id).unwrap()
    }

    fn claim_status(&mut self, id: &str) -> ClaimStatus {
        let w = self.worker.clone();
        let s = self.stream.clone();
        self.db
            .list_claims(&w, &s)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap()
            .status
    }

    fn balance(&mut self) -> Balance {
        let w = self.worker.clone();
        let s = self.stream.clone();
        self.db.balance(&w, &s).unwrap()
    }

    /// Mines `txid` now and advances the tip until it has `confs` confirmations.
    fn mine_to(&self, txid: &str, confs: i64) {
        self.wallet.mine(txid);
        self.wallet.set_tip(self.wallet.tip() + confs - 1);
    }
}

fn states(events: &[PayoutEvent]) -> Vec<PayoutState> {
    events.iter().map(|e| e.to_state).collect()
}

// ---- scenarios -----------------------------------------------------------------

/// RESERVED → SUBMITTED → CONFIRMED; the claim is paid only at confirmation.
fn submit_then_confirm<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(40_000_000);
    let p = w.pay(&c.id);
    assert_eq!(p.state, PayoutState::Submitted);
    let txid = p.txid.clone().unwrap();
    assert!(p.lease_owner.is_none());
    // A successful broadcast is not a payment.
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);
    let b = w.balance();
    assert_eq!((b.reserved, b.claimed), (40_000_000, 0));

    // Mined, but below the confirmation policy: still submitted.
    w.mine_to(&txid, 9);
    let r = w.reconcile("worker-1");
    assert_eq!(r.len(), 1);
    assert_eq!(
        (r[0].state, r[0].confirmations),
        (PayoutState::Submitted, 9)
    );
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);

    w.wallet.set_tip(w.wallet.tip() + 1);
    let r = w.reconcile("worker-1");
    assert_eq!(r[0].state, PayoutState::Confirmed);
    assert_eq!(r[0].confirmations, 10);
    let worker = w.worker.clone();
    let claim =
        w.db.list_claims(&worker, &w.stream.clone())
            .unwrap()
            .remove(0);
    assert_eq!(claim.status, ClaimStatus::Settled);
    assert_eq!(claim.settlement_ref.as_deref(), Some(txid.as_str()));
    let b = w.balance();
    assert_eq!((b.reserved, b.claimed), (0, 40_000_000));

    let (_, events) = w.db.payout_history(&worker, &p.id).unwrap();
    assert_eq!(
        states(&events),
        [
            PayoutState::Reserved,
            PayoutState::Submitted,
            PayoutState::Confirmed
        ]
    );
    // Final: nothing further happens.
    assert!(w.reconcile("worker-1").is_empty());
    assert_eq!(w.wallet.txs_built(), 1);
}

fn authorization<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(10_000_000);
    let (owner, rival, other) = (
        w.owner.clone(),
        w.rival_owner.clone(),
        w.other_worker.clone(),
    );
    // The employer cannot redirect a contributor's claim.
    assert!(matches!(
        w.request_as(&owner, &c.id, ADDR),
        Err(PayrollError::Forbidden(_))
    ));
    // Outsiders and colleagues cannot even see it.
    for actor in [&rival, &other] {
        assert!(matches!(
            w.request_as(actor, &c.id, ADDR),
            Err(PayrollError::NotFound("claim"))
        ));
        assert!(matches!(
            w.db.payout_eligibility(actor, &c.id, ALL_ORGS),
            Err(PayrollError::NotFound("claim"))
        ));
    }
    assert!(matches!(
        w.request_as("not-a-uuid", &c.id, ADDR),
        Err(PayrollError::NotFound(_))
    ));
    // Organization not enabled for payouts.
    let worker = w.worker.clone();
    let org = w.org.clone();
    let deny_org: &dyn Fn(&str) -> bool = &move |o| o != org;
    assert!(matches!(
        w.db.payout_eligibility(&worker, &c.id, deny_org),
        Err(PayrollError::Forbidden(_))
    ));
    // Released claims are not payable.
    let c2 = w.claim(10_000_000);
    w.db.release_claim(&worker, &c2.id).unwrap();
    assert!(matches!(
        w.request_as(&worker, &c2.id, ADDR),
        Err(PayrollError::InvalidState(_))
    ));
    // The claimant can; the amount comes from the claim, not the request.
    assert_eq!(
        w.db.payout_eligibility(&worker, &c.id, ALL_ORGS)
            .unwrap()
            .amount_zats,
        10_000_000
    );
    let p = w.request_as(&worker, &c.id, ADDR).unwrap().payout;
    assert_eq!(p.amount_zats, 10_000_000);
    // Payout details are visible to the claimant and managers only.
    assert!(w.db.payout_history(&owner, &p.id).is_ok());
    assert!(matches!(
        w.db.payout_history(&rival, &p.id),
        Err(PayrollError::NotFound("payout"))
    ));
    assert!(matches!(
        w.db.payout_history(&other, &p.id),
        Err(PayrollError::NotFound("payout"))
    ));
}

fn duplicate_requests<F: Factory>(f: F) {
    let mut w = world(f);
    let worker = w.worker.clone();
    let c = w.claim(10_000_000);
    let first = w.request_as(&worker, &c.id, ADDR).unwrap();
    let again = w.request_as(&worker, &c.id, ADDR).unwrap();
    assert!(!first.duplicate && again.duplicate);
    assert_eq!(first.payout.id, again.payout.id);
    // Same claim, different destination: refused.
    assert!(matches!(
        w.request_as(&worker, &c.id, OTHER_ADDR),
        Err(PayrollError::Conflict(_))
    ));
    let p = submit_payout(&mut w.db, &*w.wallet, &w.policy, &first.payout, ADDR).unwrap();
    // Re-submitting the same payout (a retried handler) is idempotent.
    let stale = first.payout.clone();
    let p2 = submit_payout(&mut w.db, &*w.wallet, &w.policy, &stale, ADDR).unwrap();
    assert_eq!(p2.txid, p.txid);
    w.mine_to(p.txid.as_ref().unwrap(), 10);
    w.reconcile("worker-1");
    // After confirmation a retry returns the confirmed payout, nothing new.
    let after = w.request_as(&worker, &c.id, ADDR).unwrap();
    assert!(after.duplicate);
    assert_eq!(after.payout.state, PayoutState::Confirmed);
    assert_eq!(w.wallet.txs_built(), 1);
}

fn concurrent_requests_pay_once<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(20_000_000);
    const THREADS: usize = 12;
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let (fac, clock, worker, cid, wallet, policy, barrier) = (
                w.f.clone(),
                w.clock.clone(),
                w.worker.clone(),
                c.id.clone(),
                w.wallet.clone(),
                w.policy.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                let mut db = fac.open(&clock);
                barrier.wait();
                let out = db
                    .request_payout(
                        &worker,
                        &PayoutTerms {
                            claim_id: &cid,
                            fee_estimate_zats: FEE,
                            max_fee_zats: 20_000,
                            lease_owner: "api-x",
                            lease_secs: 600,
                        },
                        &|id| recipient_fingerprint(id, ADDR),
                        ALL_ORGS,
                    )
                    .unwrap();
                // Exactly what the HTTP handler does: only a new payout is submitted.
                if !out.duplicate {
                    submit_payout(&mut db, &*wallet, &policy, &out.payout, ADDR).unwrap();
                }
                (out.payout.id, out.duplicate)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|(_, d)| !d).count(), 1, "{results:?}");
    assert_eq!(
        results
            .iter()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>()
            .len(),
        1
    );
    assert_eq!(w.wallet.txs_built(), 1);
    assert_eq!(w.wallet.0.lock().unwrap().submits, 1);
}

/// Several claims, several workers racing: each payout progresses exactly once.
fn concurrent_workers<F: Factory>(f: F) {
    let mut w = world(f);
    let mut ids = vec![];
    for _ in 0..6 {
        let c = w.claim(10_000_000);
        ids.push(w.pay(&c.id));
    }
    for p in &ids {
        w.wallet.mine(p.txid.as_ref().unwrap());
    }
    w.wallet.set_tip(w.wallet.tip() + 20);
    const WORKERS: usize = 4;
    let barrier = Arc::new(Barrier::new(WORKERS));
    let handles: Vec<_> = (0..WORKERS)
        .map(|i| {
            let (fac, clock, wallet, policy, barrier) = (
                w.f.clone(),
                w.clock.clone(),
                w.wallet.clone(),
                w.policy.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                let mut db = fac.open(&clock);
                barrier.wait();
                reconcile_due(&mut db, &*wallet, &policy, &format!("worker-{i}"), 50)
                    .unwrap()
                    .into_iter()
                    .map(|p| p.id)
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let processed: Vec<String> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    // Leases: every payout was processed by exactly one worker.
    assert_eq!(processed.len(), 6, "{processed:?}");
    assert_eq!(processed.iter().collect::<HashSet<_>>().len(), 6);
    for p in &ids {
        assert_eq!(w.payout(&p.id).state, PayoutState::Confirmed);
        assert_eq!(w.claim_status(&p.claim_id), ClaimStatus::Settled);
    }
    assert_eq!(w.balance().claimed, 60_000_000);
    assert_eq!(w.wallet.txs_built(), 6);
}

/// A claim with a payout can't be cancelled or hand-settled, even with raw SQL.
fn claim_locked_to_payout<F: Factory>(f: F) {
    let mut w = world(f);
    let (worker, owner) = (w.worker.clone(), w.owner.clone());
    let c = w.claim(10_000_000);
    let p = w.pay(&c.id);
    assert!(matches!(
        w.db.release_claim(&worker, &c.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        w.db.release_claim(&owner, &c.id),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        w.db.settle_claim(&owner, &c.id, "manual"),
        Err(PayrollError::InvalidState(_))
    ));
    let e = w
        .f
        .raw(&format!(
            "UPDATE claims SET status = 'released', resolved_at = {T0}, resolved_by = '{worker}' WHERE id = '{}'",
            c.id
        ))
        .unwrap_err();
    assert!(e.contains("only its confirmation or failure"), "{e}");
    let e = w
        .f
        .raw(&format!(
            "UPDATE claims SET status = 'settled', resolved_at = {T0}, resolved_by = '{worker}', settlement_ref = '{}' WHERE id = '{}'",
            p.txid.unwrap(),
            c.id
        ))
        .unwrap_err();
    assert!(e.contains("only its confirmation or failure"), "{e}");
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);
}

/// The wallet proves no transaction was built: failed, claim released, the
/// amount is available again and can be claimed and paid anew.
fn definite_failure_releases_claim<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(30_000_000);
    w.wallet
        .then(SubmitMode::FailNoTx("insufficient spendable balance"));
    let p = w.pay(&c.id);
    assert_eq!(p.state, PayoutState::Failed);
    assert_eq!(p.detail.as_deref(), Some("insufficient spendable balance"));
    assert!(p.txid.is_none() && p.resolved_at.is_some());
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Released);
    let b = w.balance();
    assert_eq!((b.reserved, b.claimed, b.available), (0, 0, COIN));
    // The failed payout is final; a retry is a new claim.
    let worker = w.worker.clone();
    assert!(w.request_as(&worker, &c.id, ADDR).unwrap().duplicate);
    let c2 = w.claim(30_000_000);
    assert_eq!(w.pay(&c2.id).state, PayoutState::Submitted);
    assert_eq!(w.wallet.txs_built(), 1);
}

/// The wallet built and broadcast the transaction but the response was lost
/// (timeout / dropped connection). No second payment; the reconciler finds it.
fn lost_response_is_reconciled<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(25_000_000);
    w.wallet.then(SubmitMode::LostResponse);
    let p = w.pay(&c.id);
    assert_eq!(p.state, PayoutState::RequiresReconciliation);
    assert!(p.txid.is_none());
    assert!(p.detail.as_deref().unwrap().contains("outcome unknown"));
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);

    let r = w.reconcile("worker-1");
    assert_eq!(r[0].state, PayoutState::Submitted);
    let txid = r[0].txid.clone().unwrap();
    w.mine_to(&txid, 10);
    assert_eq!(w.reconcile("worker-1")[0].state, PayoutState::Confirmed);
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Settled);
    assert_eq!(w.wallet.txs_built(), 1);
    let worker = w.worker.clone();
    let (_, ev) = w.db.payout_history(&worker, &p.id).unwrap();
    assert_eq!(
        states(&ev),
        [
            PayoutState::Reserved,
            PayoutState::RequiresReconciliation,
            PayoutState::Submitted,
            PayoutState::Confirmed
        ]
    );
}

/// The request never reached the wallet. The reconciler fences the payment id
/// before failing the payout, so a late-arriving request can't pay it.
fn lost_request_is_fenced_then_failed<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(25_000_000);
    w.wallet.then(SubmitMode::LostRequest);
    let p = w.pay(&c.id);
    assert_eq!(p.state, PayoutState::RequiresReconciliation);
    let r = w.reconcile("worker-1");
    assert_eq!(r[0].state, PayoutState::Failed);
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Released);
    // The original request finally arrives: the fence refuses to pay.
    let late = w
        .wallet
        .submit(&WalletSubmit {
            payment_id: p.id.clone(),
            address: ADDR.into(),
            amount_zats: p.amount_zats,
            max_fee_zats: p.max_fee_zats,
        })
        .unwrap();
    assert_eq!(late.state, WalletPaymentState::Failed);
    assert_eq!(w.wallet.txs_built(), 0);
}

/// The API process died after recording the payout but before (or while)
/// submitting it. After its lease expires a fresh process reconciles.
fn interrupted_submission_recovers_after_restart<F: Factory>(f: F) {
    let mut w = world(f);
    let worker = w.worker.clone();
    // (a) crashed before calling the wallet.
    let c1 = w.claim(10_000_000);
    let p1 = w.request_as(&worker, &c1.id, ADDR).unwrap().payout;
    // (b) crashed after the wallet received and broadcast it.
    let c2 = w.claim(10_000_000);
    let p2 = w.request_as(&worker, &c2.id, ADDR).unwrap().payout;
    w.wallet
        .submit(&WalletSubmit {
            payment_id: p2.id.clone(),
            address: ADDR.into(),
            amount_zats: p2.amount_zats,
            max_fee_zats: p2.max_fee_zats,
        })
        .unwrap();
    drop(w.db); // "crash"

    // Restart: a new process with a new identity. Live leases are respected.
    let mut db = w.f.open(&w.clock);
    assert!(
        reconcile_due(&mut db, &*w.wallet, &w.policy, "api-2", 50)
            .unwrap()
            .is_empty()
    );
    w.clock.advance(w.policy.submit_lease_secs + 1);
    let mut got = reconcile_due(&mut db, &*w.wallet, &w.policy, "api-2", 50).unwrap();
    got.sort_by_key(|p| (p.id != p1.id) as u8);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].state, PayoutState::Failed);
    assert_eq!(got[1].state, PayoutState::Submitted);
    w.db = db;
    assert_eq!(w.claim_status(&c1.id), ClaimStatus::Released);
    assert_eq!(w.claim_status(&c2.id), ClaimStatus::Reserved);
    w.mine_to(got[1].txid.as_ref().unwrap(), 10);
    w.reconcile("api-2");
    assert_eq!(w.claim_status(&c2.id), ClaimStatus::Settled);
    assert_eq!(w.wallet.txs_built(), 1);
    // A retried submit for the failed payout can never pay.
    let again = submit_payout(&mut w.db, &*w.wallet, &w.policy, &p1, ADDR).unwrap();
    assert_eq!(again.state, PayoutState::Failed);
    assert_eq!(w.wallet.txs_built(), 1);
}

/// Built but not acknowledged by the network: ambiguous until the wallet's
/// rebroadcast succeeds or the chain shows it.
fn unacknowledged_broadcast<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(10_000_000);
    w.wallet.then(SubmitMode::BuiltOnly);
    let p = w.pay(&c.id);
    assert_eq!(p.state, PayoutState::RequiresReconciliation);
    assert!(p.txid.is_some(), "txid is recorded as soon as it is known");
    assert_eq!(
        w.reconcile("w")[0].state,
        PayoutState::RequiresReconciliation
    );
    w.wallet.acknowledge_broadcasts();
    assert_eq!(w.reconcile("w")[0].state, PayoutState::Submitted);
    w.mine_to(p.txid.as_ref().unwrap(), 10);
    assert_eq!(w.reconcile("w")[0].state, PayoutState::Confirmed);
    assert_eq!(w.wallet.txs_built(), 1);
}

/// An unmined transaction fails only once it can no longer be mined.
fn expiry_fails_only_when_certain<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(10_000_000);
    let p = w.pay(&c.id);
    let expiry = p.expiry_height.unwrap();
    // Past expiry and gone from the mempool, but within the reorg margin: wait.
    w.wallet.set_tip(expiry + w.policy.expiry_margin_blocks - 1);
    w.wallet.drop_from_mempool(p.txid.as_ref().unwrap());
    assert_eq!(w.reconcile("w")[0].state, PayoutState::Submitted);
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);
    w.wallet.set_tip(expiry + w.policy.expiry_margin_blocks);
    let r = w.reconcile("w");
    assert_eq!(r[0].state, PayoutState::Failed);
    assert!(r[0].detail.as_deref().unwrap().contains("expired"));
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Released);
}

/// Wrong amount, recipient, pool crossing or extra outputs: never confirmed.
fn verification_mismatch_requires_reconciliation<F: Factory>(f: F) {
    for t in [
        Tamper::Amount,
        Tamper::Recipient,
        Tamper::PoolCrossing,
        Tamper::ExtraOutput,
    ] {
        let mut w = world(F::fresh());
        let c = w.claim(10_000_000);
        let p = w.pay(&c.id);
        w.wallet.tamper(t);
        w.mine_to(p.txid.as_ref().unwrap(), 50);
        let r = w.reconcile("w");
        assert_eq!(r[0].state, PayoutState::RequiresReconciliation, "{t:?}");
        assert!(
            r[0].detail
                .as_deref()
                .unwrap()
                .contains("verification failed")
        );
        // Stays there on later passes; the claim is never settled.
        assert_eq!(
            w.reconcile("w")[0].state,
            PayoutState::RequiresReconciliation
        );
        assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);
    }
    drop(f);
}

/// A reorg that un-mines the transaction resets the confirmation count.
fn reorg_resets_confirmations<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(10_000_000);
    let p = w.pay(&c.id);
    let txid = p.txid.clone().unwrap();
    w.mine_to(&txid, 5);
    assert_eq!(w.reconcile("w")[0].confirmations, 5);
    w.wallet.unmine(&txid);
    let r = w.reconcile("w");
    assert_eq!(
        (r[0].state, r[0].confirmations),
        (PayoutState::Submitted, 0)
    );
    assert!(r[0].mined_height.is_none());
    w.mine_to(&txid, 10);
    assert_eq!(w.reconcile("w")[0].state, PayoutState::Confirmed);
}

/// Wallet down: nothing changes, nothing is decided.
fn wallet_unavailable_waits<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(10_000_000);
    let p = w.pay(&c.id);
    w.mine_to(p.txid.as_ref().unwrap(), 10);
    w.wallet.set_unavailable(true);
    let r = w.reconcile("w");
    assert_eq!(r[0].state, PayoutState::Submitted);
    assert!(
        r[0].lease_owner.is_none(),
        "lease released for the next pass"
    );
    // Submitting while it is down: outcome unknown → reconciliation, not failure.
    let c2 = w.claim(10_000_000);
    let p2 = w.pay(&c2.id);
    assert_eq!(p2.state, PayoutState::RequiresReconciliation);
    w.wallet.set_unavailable(false);
    let r = w.reconcile("w");
    let by_id: HashMap<_, _> = r.iter().map(|p| (p.id.clone(), p.state)).collect();
    assert_eq!(by_id[&p.id], PayoutState::Confirmed);
    // Never reached the wallet: fenced, then failed safely.
    assert_eq!(by_id[&p2.id], PayoutState::Failed);
    assert_eq!(w.wallet.txs_built(), 1);
}

/// Phase 4: an unmined transaction past expiry is failed (and its claim
/// released) only when the wallet's own scan agrees; a node that hides a mined
/// transaction (stale fork) or a wallet that can't see the chain never
/// causes a failure.
fn expiry_needs_wallet_corroboration<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(10_000_000);
    let p = w.pay(&c.id);
    let txid = p.txid.clone().unwrap();
    let expiry = p.expiry_height.unwrap();
    w.wallet.drop_from_mempool(&txid);
    w.wallet.set_wallet_blind(true);
    w.wallet
        .set_tip(expiry + 10 * w.policy.expiry_margin_blocks);
    for _ in 0..3 {
        assert_eq!(w.reconcile("w")[0].state, PayoutState::Submitted);
    }
    w.wallet.set_wallet_blind(false);
    w.wallet.wallet_saw_mined(&txid);
    assert_eq!(w.reconcile("w")[0].state, PayoutState::Submitted);
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);
    // A fresh payout whose transaction really expired: both views agree.
    let c2 = w.claim(10_000_000);
    let p2 = w.pay(&c2.id);
    w.wallet.drop_from_mempool(p2.txid.as_ref().unwrap());
    w.wallet
        .set_tip(p2.expiry_height.unwrap() + w.policy.expiry_margin_blocks);
    let r = w.reconcile("w");
    let p2r = r.iter().find(|x| x.id == p2.id).unwrap();
    assert_eq!(p2r.state, PayoutState::Failed);
    assert_eq!(w.claim_status(&c2.id), ClaimStatus::Released);
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);
}

/// Phase 4: a crash in the middle of settling the claim rolls back the whole
/// confirmation (payout stays submitted, claim stays reserved); the next pass
/// completes it. Never "confirmed but unpaid" or "paid but not confirmed".
fn settlement_is_atomic<F: Factory>(f: F) {
    let mut w = world(f);
    let c = w.claim(10_000_000);
    let p = w.pay(&c.id);
    w.mine_to(p.txid.as_ref().unwrap(), 10);
    w.f.settlement_fault(true);
    let leased = w.db.lease_due_payouts("w", 60, 10).unwrap();
    assert_eq!(leased.len(), 1);
    let d = reconcile_decision(&leased[0], &*w.wallet, &w.policy);
    assert!(matches!(d, Decision::Confirmed(_)));
    let e =
        w.db.apply_payout_decision(&p.id, "w", PayoutState::Submitted, &d)
            .unwrap_err();
    assert!(format!("{e:?}").contains("injected crash"), "{e:?}");
    let after = w.payout(&p.id);
    assert_eq!(after.state, PayoutState::Submitted);
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);
    let worker = w.worker.clone();
    let (_, ev) = w.db.payout_history(&worker, &p.id).unwrap();
    assert!(!states(&ev).contains(&PayoutState::Confirmed));
    w.f.settlement_fault(false);
    w.clock.advance(61); // the crashed attempt's lease expires
    assert_eq!(w.reconcile("w2")[0].state, PayoutState::Confirmed);
    assert_eq!(w.claim_status(&c.id), ClaimStatus::Settled);
    assert_eq!(w.wallet.txs_built(), 1);
}

/// The payout schema enforces the state machine against direct SQL.
fn schema_guards<F: Factory>(f: F) {
    let mut w = world(f);
    let worker = w.worker.clone();
    let c = w.claim(10_000_000);
    let p = w.pay(&c.id);
    let f = w.f.clone();
    let raw = |sql: String, needle: &str| {
        let e = f.raw(&sql).unwrap_err();
        assert!(e.contains(needle), "{sql}\n=> {e}");
    };
    let fp = "0".repeat(64);
    raw(
        format!(
            "INSERT INTO payouts (id, claim_id, stream_id, requested_by, amount_zats, recipient_fingerprint,
                 fee_estimate_zats, max_fee_zats, state, created_at, updated_at)
             VALUES ('{}', '{}', '{}', '{worker}', 10000000, '{fp}', 0, 0, 'reserved', {T0}, {T0})",
            uuid::Uuid::new_v4(),
            c.id,
            w.stream
        ),
        "", // second payout for the same claim: UNIQUE (claim_id)
    );
    let c2 = w.claim(10_000_000);
    raw(
        format!(
            "INSERT INTO payouts (id, claim_id, stream_id, requested_by, amount_zats, recipient_fingerprint,
                 fee_estimate_zats, max_fee_zats, state, txid, created_at, updated_at)
             VALUES ('{}', '{}', '{}', '{worker}', 10000000, '{fp}', 0, 0, 'submitted', '{}', {T0}, {T0})",
            uuid::Uuid::new_v4(),
            c2.id,
            w.stream,
            "a".repeat(64)
        ),
        "must be created reserved",
    );
    raw(
        format!(
            "INSERT INTO payouts (id, claim_id, stream_id, requested_by, amount_zats, recipient_fingerprint,
                 fee_estimate_zats, max_fee_zats, state, created_at, updated_at)
             VALUES ('{}', '{}', '{}', '{worker}', 99, '{fp}', 0, 0, 'reserved', {T0}, {T0})",
            uuid::Uuid::new_v4(),
            c2.id,
            w.stream
        ),
        "for its amount",
    );
    raw(
        format!(
            "UPDATE payouts SET txid = '{}' WHERE id = '{}'",
            "b".repeat(64),
            p.id
        ),
        "write-once",
    );
    raw(
        format!("UPDATE payouts SET amount_zats = 1 WHERE id = '{}'", p.id),
        "immutable",
    );
    raw(
        format!(
            "UPDATE payouts SET state = 'reserved' WHERE id = '{}'",
            p.id
        ),
        "invalid payout state transition",
    );
    raw(
        format!("DELETE FROM payouts WHERE id = '{}'", p.id),
        "cannot be deleted",
    );
    raw(
        format!("DELETE FROM payout_events WHERE payout_id = '{}'", p.id),
        "append-only",
    );
    w.mine_to(p.txid.as_ref().unwrap(), 10);
    w.reconcile("w");
    raw(
        format!("UPDATE payouts SET detail = 'x' WHERE id = '{}'", p.id),
        "final",
    );
}

macro_rules! scenarios {
    ($factory:ty $(, #[$attr:meta])?) => {
        #[test] $(#[$attr])? fn submit_then_confirm() { super::submit_then_confirm(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn authorization() { super::authorization(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn duplicate_requests() { super::duplicate_requests(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn concurrent_requests_pay_once() { super::concurrent_requests_pay_once(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn concurrent_workers() { super::concurrent_workers(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn claim_locked_to_payout() { super::claim_locked_to_payout(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn definite_failure_releases_claim() { super::definite_failure_releases_claim(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn lost_response_is_reconciled() { super::lost_response_is_reconciled(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn lost_request_is_fenced_then_failed() { super::lost_request_is_fenced_then_failed(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn interrupted_submission_recovers_after_restart() { super::interrupted_submission_recovers_after_restart(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn unacknowledged_broadcast() { super::unacknowledged_broadcast(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn expiry_fails_only_when_certain() { super::expiry_fails_only_when_certain(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn verification_mismatch_requires_reconciliation() { super::verification_mismatch_requires_reconciliation(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn reorg_resets_confirmations() { super::reorg_resets_confirmations(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn wallet_unavailable_waits() { super::wallet_unavailable_waits(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn schema_guards() { super::schema_guards(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn expiry_needs_wallet_corroboration() { super::expiry_needs_wallet_corroboration(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn settlement_is_atomic() { super::settlement_is_atomic(<$factory>::fresh()) }
    };
}

trait Fresh {
    fn fresh() -> Self;
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
        fn settlement_fault(&self, on: bool) {
            self.raw(if on {
                "CREATE TRIGGER fault_settlement BEFORE UPDATE OF status ON claims
                 BEGIN SELECT RAISE(ABORT, 'injected crash during settlement'); END;"
            } else {
                "DROP TRIGGER fault_settlement;"
            })
            .unwrap();
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
        fn settlement_fault(&self, on: bool) {
            self.raw(if on {
                "CREATE FUNCTION fault_settlement() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN RAISE EXCEPTION 'injected crash during settlement'; END $$;
                 CREATE TRIGGER fault_settlement BEFORE UPDATE OF status ON claims
                 FOR EACH ROW EXECUTE FUNCTION fault_settlement();"
            } else {
                "DROP TRIGGER fault_settlement ON claims; DROP FUNCTION fault_settlement();"
            })
            .unwrap();
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

    /// Hosted Postgres (e.g. Supabase) grants API roles privileges on tables in
    /// the exposed schema. Row-level security with no policies must still give
    /// such a role nothing, while the owning role (the app) is unaffected.
    #[test]
    #[ignore = "requires PAYROLL_TEST_DATABASE_URL"]
    fn api_roles_see_no_rows() {
        let mut w = world(Pg::fresh());
        let c = w.claim(10_000_000);
        w.pay(&c.id);
        let schema = w.f.schema.clone();
        let mut client = hushush_payroll::pg::connect_client(&w.f.url).unwrap();
        hushush_payroll::pg::use_schema(&mut client, &schema).unwrap();
        client
            .batch_execute(&format!(
                "DO $$ BEGIN
                     IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'hushush_api_probe') THEN
                         CREATE ROLE hushush_api_probe NOLOGIN;
                     END IF;
                 END $$;
                 GRANT USAGE ON SCHEMA {schema} TO hushush_api_probe;
                 GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA {schema} TO hushush_api_probe;"
            ))
            .unwrap();
        for table in [
            "users",
            "organizations",
            "organization_members",
            "payroll_streams",
            "stream_events",
            "claims",
            "payouts",
            "payout_events",
            "payroll_schema_migrations",
        ] {
            let owner: i64 = client
                .query_one(&format!("SELECT count(*) FROM {table}"), &[])
                .unwrap()
                .get(0);
            assert!(
                owner > 0,
                "{table}: the app (owner) must still see its rows"
            );
            client.batch_execute("SET ROLE hushush_api_probe").unwrap();
            let api: i64 = client
                .query_one(&format!("SELECT count(*) FROM {table}"), &[])
                .unwrap()
                .get(0);
            client.batch_execute("RESET ROLE").unwrap();
            assert_eq!(api, 0, "{table}: an API role must see no rows");
        }
        client.batch_execute("SET ROLE hushush_api_probe").unwrap();
        let e = client
            .batch_execute("INSERT INTO organizations (id, name, created_by, created_at) VALUES ('x', 'x', 'x', 0)")
            .unwrap_err();
        client.batch_execute("RESET ROLE").unwrap();
        assert!(format!("{e:?}").contains("row-level security"), "{e:?}");
        // Workflow through the app is unaffected.
        assert_eq!(w.claim_status(&c.id), ClaimStatus::Reserved);
    }
}

// ---- pure decision rules ---------------------------------------------------------

#[test]
fn transition_table() {
    use PayoutState::*;
    let all = [
        Reserved,
        Submitted,
        Confirmed,
        Failed,
        RequiresReconciliation,
    ];
    let allowed: HashSet<_> = [
        (Reserved, Submitted),
        (Reserved, Failed),
        (Reserved, RequiresReconciliation),
        (Submitted, Confirmed),
        (Submitted, Failed),
        (Submitted, RequiresReconciliation),
        (RequiresReconciliation, Submitted),
        (RequiresReconciliation, Confirmed),
        (RequiresReconciliation, Failed),
    ]
    .into_iter()
    .collect();
    for a in all {
        for b in all {
            assert_eq!(a.can_become(b), allowed.contains(&(a, b)), "{a:?} -> {b:?}");
        }
    }
    assert!(Confirmed.is_final() && Failed.is_final());
    assert!(!RequiresReconciliation.is_final());
}

fn sample_payout() -> Payout {
    let id = "11111111-1111-1111-1111-111111111111".to_string();
    Payout {
        recipient_fingerprint: recipient_fingerprint(&id, ADDR),
        id,
        claim_id: "c".into(),
        stream_id: "s".into(),
        requested_by: "u".into(),
        amount_zats: 5_000,
        fee_estimate_zats: FEE,
        max_fee_zats: 20_000,
        state: PayoutState::Submitted,
        txid: Some("a".repeat(64)),
        fee_zats: Some(FEE),
        expiry_height: Some(140),
        mined_height: None,
        confirmations: 0,
        detail: None,
        created_at: 0,
        updated_at: 0,
        submitted_at: Some(0),
        resolved_at: None,
        lease_owner: Some("w".into()),
        lease_until: Some(1),
    }
}

fn sample_wallet(p: &Payout) -> WalletPayment {
    WalletPayment {
        payment_id: p.id.clone(),
        state: WalletPaymentState::Broadcast,
        recipient_fingerprint: p.recipient_fingerprint.clone(),
        amount_zats: p.amount_zats,
        failure_reason: None,
        txid: p.txid.clone(),
        fee_zats: Some(FEE),
        expiry_height: Some(140),
        chain: Some(ChainStatus {
            tip_height: 120,
            mined_height: Some(111),
            confirmations: 10,
            in_mempool: false,
            wallet_mined_height: Some(111),
            wallet_expired_unmined: Some(false),
        }),
        verification: Some(TxVerification {
            source: "chain".into(),
            fully_shielded: true,
            single_pool: true,
            fee_zats: Some(FEE),
            outputs: vec![VerifiedOutput {
                recipient_fingerprint: p.recipient_fingerprint.clone(),
                amount_zats: p.amount_zats,
                memo_matches: true,
                pool: "ironwood".into(),
            }],
        }),
    }
}

#[test]
fn decide_confirms_only_with_chain_verification() {
    let (p, pol) = (sample_payout(), PayoutPolicy::default());
    let w = sample_wallet(&p);
    assert!(matches!(decide(&p, &w, &pol), Decision::Confirmed(_)));
    let mut local = w.clone();
    local.verification.as_mut().unwrap().source = "local".into();
    assert!(matches!(decide(&p, &local, &pol), Decision::Submitted(_)));
    let mut none = w.clone();
    none.verification = None;
    assert!(matches!(decide(&p, &none, &pol), Decision::Submitted(_)));
}

#[test]
fn decide_never_fails_ambiguous_or_mismatched_reports() {
    let (p, pol) = (sample_payout(), PayoutPolicy::default());
    let base = sample_wallet(&p);
    type Mutation = Box<dyn Fn(&mut WalletPayment)>;
    let cases: Vec<(&str, Mutation)> = vec![
        ("other id", Box::new(|w| w.payment_id = "x".into())),
        (
            "other recipient",
            Box::new(|w| w.recipient_fingerprint = "f".repeat(64)),
        ),
        ("other amount", Box::new(|w| w.amount_zats += 1)),
        ("other txid", Box::new(|w| w.txid = Some("b".repeat(64)))),
        ("no txid", Box::new(|w| w.txid = None)),
        ("fee too high", Box::new(|w| w.fee_zats = Some(20_001))),
        (
            "failed after a tx was recorded",
            Box::new(|w| w.state = WalletPaymentState::Failed),
        ),
    ];
    for (name, f) in cases {
        let mut w = base.clone();
        f(&mut w);
        assert!(
            matches!(decide(&p, &w, &pol), Decision::NeedsReconciliation { .. }),
            "{name}: {:?}",
            decide(&p, &w, &pol)
        );
    }
}

#[test]
fn decide_expiry_needs_margin_and_absence_from_mempool() {
    let (p, pol) = (sample_payout(), PayoutPolicy::default());
    let mut w = sample_wallet(&p);
    let c = w.chain.as_mut().unwrap();
    c.mined_height = None;
    c.confirmations = 0;
    c.tip_height = 140 + pol.expiry_margin_blocks;
    c.in_mempool = true;
    c.wallet_mined_height = None;
    c.wallet_expired_unmined = Some(true);
    assert!(matches!(decide(&p, &w, &pol), Decision::Submitted(_)));
    w.chain.as_mut().unwrap().in_mempool = false;
    assert!(matches!(decide(&p, &w, &pol), Decision::Failed { .. }));
    // Phase 4: never fail without the wallet's own scan agreeing.
    for (mined, expired) in [(None, None), (None, Some(false)), (Some(130), Some(true))] {
        let mut v = w.clone();
        let c = v.chain.as_mut().unwrap();
        c.wallet_mined_height = mined;
        c.wallet_expired_unmined = expired;
        assert!(
            matches!(decide(&p, &v, &pol), Decision::Submitted(_)),
            "{mined:?} {expired:?}"
        );
    }
    w.chain.as_mut().unwrap().tip_height -= 1;
    assert!(matches!(decide(&p, &w, &pol), Decision::Submitted(_)));
}

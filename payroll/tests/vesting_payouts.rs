//! Vesting payouts (Vesting Phase 2): vesting claims paid through the
//! existing payout state machine, run against BOTH storage backends.
//!
//! *** The wallet here is a MOCK. *** [`MockWallet`] honours the payout
//! wallet's contract (one transaction per payment id, fence on cancel) and a
//! toy chain in memory, and counts every "transaction" it builds so each
//! scenario can assert that nothing was ever paid twice. There are no keys,
//! proofs or network. The real wallet is covered by payout-wallet's tests and
//! the live testnet runs.
//!
//! * `sqlite::*` always runs.
//! * `postgres::*` needs `PAYROLL_TEST_DATABASE_URL` and is `#[ignore]`d by
//!   default (`cargo test --test vesting_payouts -- --include-ignored`).

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
const TOTAL: i64 = 4 * COIN;
const DURATION: i64 = 400 * DAY;
const ADDR: &str = "utest1mockvestingrecipientaddress";
const OTHER_ADDR: &str = "utest1someoneelseentirely";
const FEE: i64 = 10_000;

// ---- MOCK wallet ----------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum SubmitMode {
    /// Built and accepted by the network.
    Broadcast,
    /// Built and stored (signed), but the network has not acknowledged it.
    BuiltOnly,
    /// Rejected before any transaction was built.
    FailNoTx(&'static str),
    /// Built and broadcast, but the response never reached the caller.
    LostResponse,
    /// The request never reached the wallet.
    LostRequest,
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
    fn then(&self, m: SubmitMode) {
        self.0.lock().unwrap().next_submit.push_back(m);
    }
    fn txs_built(&self) -> usize {
        self.0.lock().unwrap().txs_built
    }
    /// Mines `txid` and advances the tip until it has `confs` confirmations.
    fn mine_to(&self, txid: &str, confs: i64) {
        let mut s = self.0.lock().unwrap();
        let h = s.tip;
        s.mined.insert(txid.into(), h);
        s.tip += confs - 1;
    }
    /// The wallet's own background rebroadcast succeeding.
    fn acknowledge_broadcasts(&self) {
        for r in self.0.lock().unwrap().records.values_mut() {
            if r.pay.state == WalletPaymentState::Built {
                r.pay.state = WalletPaymentState::Broadcast;
            }
        }
    }
    /// A broadcast payment the wallet already knows (from before an upgrade).
    fn adopt(&self, payment_id: &str, address: &str, amount: i64, txid: &str) {
        let mut s = self.0.lock().unwrap();
        let tip = s.tip;
        s.records.insert(
            payment_id.into(),
            Rec {
                pay: WalletPayment {
                    payment_id: payment_id.into(),
                    state: WalletPaymentState::Broadcast,
                    recipient_fingerprint: recipient_fingerprint(payment_id, address),
                    amount_zats: amount,
                    failure_reason: None,
                    txid: Some(txid.into()),
                    fee_zats: Some(FEE),
                    expiry_height: Some(tip + 40),
                    chain: None,
                    verification: None,
                },
                address: Some(address.into()),
            },
        );
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
                    && s.tip <= expiry,
                wallet_mined_height: mined,
                wallet_expired_unmined: Some(mined.is_none() && s.tip > expiry),
            });
            p.verification = Some(TxVerification {
                source: if mined.is_some() { "chain" } else { "local" }.into(),
                fully_shielded: true,
                single_pool: true,
                fee_zats: p.fee_zats,
                outputs: vec![VerifiedOutput {
                    recipient_fingerprint: recipient_fingerprint(
                        &p.payment_id,
                        r.address.as_deref().unwrap_or(""),
                    ),
                    amount_zats: p.amount_zats,
                    memo_matches: true,
                    pool: "orchard".into(),
                }],
            });
        }
        p
    }
}

impl WalletApi for MockWallet {
    fn submit(&self, req: &WalletSubmit) -> Result<WalletPayment, WalletError> {
        let mut s = self.0.lock().unwrap();
        if let Some(r) = s.records.get(&req.payment_id) {
            // Idempotent: the same id never builds a second transaction.
            if r.address.is_some()
                && (r.address.as_deref() != Some(req.address.as_str())
                    || r.pay.amount_zats != req.amount_zats)
            {
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
        let mut pay = WalletPayment {
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
        Ok(s.records.get(id).map(|r| Self::view(&s, r)))
    }

    fn cancel(&self, id: &str) -> Result<WalletPayment, WalletError> {
        let mut s = self.0.lock().unwrap();
        let rec = s
            .records
            .entry(id.to_string())
            .or_insert_with(|| Rec {
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
            })
            .clone();
        Ok(Self::view(&s, &rec))
    }
}

// ---- world --------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Fault {
    /// Every vesting-claim status change fails (a crash while settling).
    Settlement,
    /// Every payout insert fails (a crash before the reservation commits).
    PayoutInsert,
}

trait Factory: Fresh + Send + Sync + 'static {
    type B: Backend;
    fn open(&self, clock: &ManualClock) -> Payroll<Self::B>;
    /// Runs raw SQL as a direct database user would; returns the error text.
    fn raw(&self, sql: &str) -> Result<(), String>;
    fn fault(&self, f: Fault, on: bool);
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
    employee: String,
    colleague: String,
    rival: String,
    org: String,
    /// A payroll stream of `employee` (for cross-type checks).
    stream: String,
}

fn world<F: Factory>(f: F) -> World<F> {
    let f = Arc::new(f);
    let clock = ManualClock::at(T0);
    let mut db = f.open(&clock);
    let mut user = |n: &str| db.create_user(&format!("{n}@a.test"), n).unwrap();
    let (employer, employee, colleague, rival) = (
        user("employer"),
        user("employee"),
        user("colleague"),
        user("rival"),
    );
    let org = db.create_organization(&employer, "Acme").unwrap();
    db.create_organization(&rival, "Rival Co").unwrap();
    db.add_member(&employer, &org, &employee, Role::Member)
        .unwrap();
    db.add_member(&employer, &org, &colleague, Role::Member)
        .unwrap();
    let stream = db
        .create_stream(
            &employer,
            NewStream {
                organization_id: org.clone(),
                recipient_user_id: employee.clone(),
                rate_zats: COIN,
                rate_period_secs: DAY,
                start_at: None,
                end_at: None,
            },
        )
        .unwrap()
        .id;
    World {
        f,
        clock,
        db,
        wallet: MockWallet::new(),
        policy: PayoutPolicy::default(),
        employer,
        employee,
        colleague,
        rival,
        org,
        stream,
    }
}

const ALL_ORGS: &dyn Fn(&str) -> bool = &|_| true;

fn agreement_fp(agreement: &str, addr: &str) -> String {
    recipient_fingerprint(agreement, addr)
}

impl<F: Factory> World<F> {
    /// An active agreement (TOTAL over DURATION, no cliff, starting now) with
    /// the employee's payout address locked.
    fn ready(&mut self) -> String {
        self.ready_with(0, 0)
    }

    fn ready_with(&mut self, start_in: i64, cliff: i64) -> String {
        let e = self.employer.clone();
        let a = self
            .db
            .create_vesting(
                &e,
                NewVesting {
                    organization_id: self.org.clone(),
                    employee_user_id: self.employee.clone(),
                    total_zats: TOTAL,
                    start_at: Some(self.clock.now() + start_in),
                    duration_secs: DURATION,
                    cliff_secs: Some(cliff),
                    method: VestingMethod::Linear,
                },
            )
            .unwrap();
        self.db.activate_vesting(&e, &a.id).unwrap();
        let emp = self.employee.clone();
        self.db
            .set_vesting_payout_address(&emp, &a.id, &agreement_fp(&a.id, ADDR), "sealed")
            .unwrap();
        a.id
    }

    fn withdraw_as(
        &mut self,
        actor: &str,
        agreement: &str,
        amount: i64,
        key: &str,
        addr: &str,
    ) -> hushush_payroll::Result<PayoutOutcome> {
        let fp = agreement_fp(agreement, addr);
        let a = addr.to_string();
        self.db.withdraw_vesting(
            actor,
            &VestingWithdrawTerms {
                agreement_id: agreement,
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

    fn withdraw(
        &mut self,
        agreement: &str,
        amount: i64,
        key: &str,
    ) -> hushush_payroll::Result<PayoutOutcome> {
        let e = self.employee.clone();
        self.withdraw_as(&e, agreement, amount, key, ADDR)
    }

    /// The API's withdraw: reserve, then submit to the wallet.
    fn pay(&mut self, agreement: &str, amount: i64, key: &str) -> Payout {
        let out = self.withdraw(agreement, amount, key).unwrap();
        if out.duplicate {
            return out.payout;
        }
        submit_payout(&mut self.db, &*self.wallet, &self.policy, &out.payout, ADDR).unwrap()
    }

    fn reconcile(&mut self, owner: &str) -> Vec<Payout> {
        reconcile_due(&mut self.db, &*self.wallet, &self.policy, owner, 50).unwrap()
    }

    fn confirm(&mut self, p: &Payout) -> Payout {
        self.wallet.mine_to(
            p.txid.as_deref().unwrap(),
            self.policy.required_confirmations,
        );
        self.reconcile("worker-1");
        self.payout(&p.id)
    }

    fn payout(&mut self, id: &str) -> Payout {
        self.db.payout_internal(id).unwrap()
    }

    fn bal(&mut self, a: &str) -> VestingBalance {
        let e = self.employee.clone();
        self.db.vesting_balance(&e, a).unwrap()
    }

    fn claim(&mut self, a: &str, id: &str) -> VestingClaim {
        let e = self.employee.clone();
        self.db
            .list_vesting_claims(&e, a)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap()
    }
}

fn states(events: &[PayoutEvent]) -> Vec<PayoutState> {
    events.iter().map(|e| e.to_state).collect()
}

fn insufficient<T: std::fmt::Debug>(r: hushush_payroll::Result<T>) -> i64 {
    match r {
        Err(PayrollError::InsufficientAvailable { available, .. }) => available,
        other => panic!("expected InsufficientAvailable, got {other:?}"),
    }
}

// ---- happy path ------------------------------------------------------------------

/// reserve → submit → confirm: the vesting claim is paid only at confirmation.
fn withdraw_submit_confirm<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2); // 2 ZEC vested
    let p = w.pay(&a, 50_000_000, "k1");
    assert_eq!(p.compensation_type, CompensationType::Vesting);
    assert_eq!(p.source_id, a);
    assert_eq!(p.state, PayoutState::Submitted);
    assert_eq!(p.recipient_fingerprint, recipient_fingerprint(&p.id, ADDR));
    // A broadcast is not a payment.
    let c = w.claim(&a, &p.claim_id);
    assert_eq!(
        (c.status, c.amount_zats),
        (ClaimStatus::Reserved, 50_000_000)
    );
    let b = w.bal(&a);
    assert_eq!(
        (b.reserved, b.paid, b.available),
        (50_000_000, 0, 150_000_000)
    );

    // Below the confirmation policy it stays submitted.
    w.wallet.mine_to(p.txid.as_deref().unwrap(), 9);
    w.reconcile("worker-1");
    assert_eq!(w.payout(&p.id).state, PayoutState::Submitted);
    assert_eq!(w.claim(&a, &p.claim_id).status, ClaimStatus::Reserved);

    let p = w.confirm(&p);
    assert_eq!(p.state, PayoutState::Confirmed);
    let c = w.claim(&a, &p.claim_id);
    assert_eq!(c.status, ClaimStatus::Settled);
    assert_eq!(c.settlement_ref, p.txid);
    let b = w.bal(&a);
    assert_eq!(
        (b.reserved, b.paid, b.available),
        (0, 50_000_000, 150_000_000)
    );

    let emp = w.employee.clone();
    let (_, events) = w.db.payout_history(&emp, &p.id).unwrap();
    assert_eq!(
        states(&events),
        [
            PayoutState::Reserved,
            PayoutState::Submitted,
            PayoutState::Confirmed
        ]
    );
    let d = w.db.vesting_detail(&emp, &a).unwrap();
    assert_eq!(d.payouts, vec![p.clone()]);
    assert!(d.open_payout().is_none() && d.address.is_some());
    // Managers see the payouts, not the address; others see nothing.
    let e = w.employer.clone();
    let d = w.db.vesting_detail(&e, &a).unwrap();
    assert!(d.address.is_none() && d.address_locked && d.payouts.len() == 1);
    let (col, rival) = (w.colleague.clone(), w.rival.clone());
    for who in [&col, &rival] {
        assert!(matches!(
            w.db.payout_history(who, &p.id),
            Err(PayrollError::NotFound("payout"))
        ));
        assert!(matches!(
            w.db.vesting_detail(who, &a),
            Err(PayrollError::NotFound(_))
        ));
    }
    // Stream payouts of the same employee are unaffected and listed apart.
    let s = w.stream.clone();
    assert!(w.db.list_payouts(&emp, &s).unwrap().is_empty());
    assert!(
        w.reconcile("worker-1").is_empty(),
        "final: nothing more to do"
    );
    assert_eq!(w.wallet.txs_built(), 1);
}

// ---- payout integrity ---------------------------------------------------------------

/// The schema accepts a payout only with exactly one, consistent claim reference.
fn payout_reference_integrity<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    let other = w.ready();
    w.clock.advance(DURATION / 2);
    let emp = w.employee.clone();
    let vc =
        w.db.reserve_vesting_claim(&emp, &a, 10_000_000, "raw")
            .unwrap()
            .claim;
    let s = w.stream.clone();
    let sc =
        w.db.reserve_claim(&emp, &s, 10_000_000, "raw")
            .unwrap()
            .claim;
    let (col, now) = (w.colleague.clone(), w.clock.now());
    let fp = "f".repeat(64);
    let ins = |kind: &str,
               claim: &str,
               stream: &str,
               vclaim: &str,
               agreement: &str,
               by: &str,
               amount: i64| {
        let q = |v: &str| {
            if v.is_empty() {
                "NULL".to_string()
            } else {
                format!("'{v}'")
            }
        };
        format!(
            "INSERT INTO payouts (id, compensation_type, claim_id, stream_id, vesting_claim_id,
                 vesting_agreement_id, requested_by, amount_zats, recipient_fingerprint,
                 fee_estimate_zats, max_fee_zats, state, confirmations, created_at, updated_at)
             VALUES ('{}', '{kind}', {}, {}, {}, {}, '{by}', {amount}, '{fp}', 0, 0, 'reserved', 0, {now}, {now})",
            uuid::Uuid::new_v4(),
            q(claim),
            q(stream),
            q(vclaim),
            q(agreement)
        )
    };
    let (vcid, scid) = (vc.id.clone(), sc.id.clone());
    for (sql, why) in [
        (
            ins("vesting", &scid, &s, &vcid, &a, &emp, 10_000_000),
            "both references",
        ),
        (
            ins("stream", &scid, &s, &vcid, &a, &emp, 10_000_000),
            "both references (stream)",
        ),
        (
            ins("vesting", "", "", "", "", &emp, 10_000_000),
            "no reference",
        ),
        (
            ins("stream", "", "", "", "", &emp, 10_000_000),
            "no reference (stream)",
        ),
        (
            ins("vesting", "", "", &vcid, &other, &emp, 10_000_000),
            "another agreement",
        ),
        (
            ins("vesting", "", "", &vcid, &a, &col, 10_000_000),
            "another employee",
        ),
        (
            ins("vesting", "", "", &vcid, &a, &emp, 9_999_999),
            "another amount",
        ),
        (
            ins("vesting", "", "", &scid, &a, &emp, 10_000_000),
            "a stream claim as vesting",
        ),
        (
            ins("stream", &vcid, &s, "", "", &emp, 10_000_000),
            "a vesting claim as stream",
        ),
        (
            ins("crypto", "", "", &vcid, &a, &emp, 10_000_000),
            "unknown type",
        ),
    ] {
        assert!(w.f.raw(&sql).is_err(), "accepted {why}");
    }
    // The one valid shape is accepted once; a second payout for the claim is not.
    w.f.raw(&ins("vesting", "", "", &vcid, &a, &emp, 10_000_000))
        .unwrap();
    let e =
        w.f.raw(&ins("vesting", "", "", &vcid, &a, &emp, 10_000_000))
            .unwrap_err();
    assert!(e.to_lowercase().contains("unique"), "{e}");
    // Once paid out (here: attached by hand), the claim is resolved only by its payout.
    for sql in [
        format!(
            "UPDATE vesting_claims SET status = 'released', resolved_at = {now}, resolved_by = '{emp}' WHERE id = '{vcid}'"
        ),
        format!(
            "UPDATE vesting_claims SET status = 'settled', resolved_at = {now}, resolved_by = '{emp}',
                 settlement_ref = 'x' WHERE id = '{vcid}'"
        ),
    ] {
        let e = w.f.raw(&sql).unwrap_err();
        assert!(e.contains("only its confirmation or failure"), "{e}");
    }
    // A payout's type and references are immutable.
    let e =
        w.f.raw(&format!(
            "UPDATE payouts SET vesting_agreement_id = '{other}' WHERE vesting_claim_id = '{vcid}'"
        ))
        .unwrap_err();
    assert!(e.contains("immutable"), "{e}");
}

/// A vesting claim with a payout can't be released or settled by hand, and
/// stream-claim operations don't know vesting claims at all.
fn attached_claim_paths<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2);
    let p = w.pay(&a, 10_000_000, "k1");
    let (emp, e) = (w.employee.clone(), w.employer.clone());
    for who in [&emp, &e] {
        assert!(matches!(
            w.db.release_vesting_claim(who, &p.claim_id),
            Err(PayrollError::InvalidState(_))
        ));
    }
    assert!(matches!(
        w.db.settle_vesting_claim(&e, &p.claim_id, "by-hand"),
        Err(PayrollError::InvalidState(_))
    ));
    assert!(matches!(
        w.db.release_claim(&emp, &p.claim_id),
        Err(PayrollError::NotFound("claim"))
    ));
    assert!(matches!(
        w.db.payout_eligibility(&emp, &p.claim_id, ALL_ORGS),
        Err(PayrollError::NotFound("claim"))
    ));
    assert!(matches!(
        w.db.claim_payout(&emp, &p.claim_id),
        Err(PayrollError::NotFound("claim"))
    ));
    let req = w.db.request_payout(
        &emp,
        &PayoutTerms {
            claim_id: &p.claim_id,
            fee_estimate_zats: FEE,
            max_fee_zats: 2 * FEE,
            lease_owner: "api-1",
            lease_secs: 600,
        },
        &|id| recipient_fingerprint(id, ADDR),
        ALL_ORGS,
    );
    assert!(matches!(req, Err(PayrollError::NotFound("claim"))));
    // Still exactly one payout and one transaction; the claim settles normally.
    let p = w.confirm(&p);
    assert_eq!(w.claim(&a, &p.claim_id).status, ClaimStatus::Settled);
    assert_eq!(w.wallet.txs_built(), 1);
    // A claim without a payout (reserved outside the withdraw flow) can still
    // be released by hand.
    let c =
        w.db.reserve_vesting_claim(&emp, &a, 1, "manual")
            .unwrap()
            .claim;
    w.db.release_vesting_claim(&emp, &c.id).unwrap();
}

/// Repeats and concurrent requests never create a second payout or transaction.
fn withdraw_idempotent_and_concurrent<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2);
    let first = w.withdraw(&a, 20_000_000, "k1").unwrap();
    assert!(!first.duplicate);
    let again = w.withdraw(&a, 20_000_000, "k1").unwrap();
    assert!(again.duplicate && again.payout.id == first.payout.id);
    assert!(matches!(
        w.withdraw(&a, 20_000_001, "k1"),
        Err(PayrollError::IdempotencyConflict)
    ));
    // Any request while a withdrawal is unfinished returns that withdrawal.
    let other_key = w.withdraw(&a, 5_000_000, "k2").unwrap();
    assert!(other_key.duplicate && other_key.payout.id == first.payout.id);
    // The process that reserved it submits; repeats after submission too.
    let p = submit_payout(&mut w.db, &*w.wallet, &w.policy, &first.payout, ADDR).unwrap();
    assert_eq!(p.state, PayoutState::Submitted);
    let again = w.withdraw(&a, 20_000_000, "k1").unwrap();
    assert!(again.duplicate && again.payout.state == PayoutState::Submitted);
    // A late duplicate submit of the same payout builds nothing new.
    submit_payout(&mut w.db, &*w.wallet, &w.policy, &first.payout, ADDR).unwrap();
    assert_eq!(w.wallet.txs_built(), 1);
    let p = w.confirm(&p);
    assert_eq!(p.state, PayoutState::Confirmed);
    // After confirmation a retry with the original key still answers with it.
    let again = w.withdraw(&a, 20_000_000, "k1").unwrap();
    assert!(again.duplicate && again.payout.state == PayoutState::Confirmed);

    // Concurrent withdrawals (distinct keys, as from two devices): one payout.
    const THREADS: usize = 8;
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|i| {
            let (fac, clock, emp, id, barrier, policy) = (
                w.f.clone(),
                w.clock.clone(),
                w.employee.clone(),
                a.clone(),
                barrier.clone(),
                w.policy.clone(),
            );
            std::thread::spawn(move || {
                let mut db = fac.open(&clock);
                barrier.wait();
                let fp = agreement_fp(&id, ADDR);
                db.withdraw_vesting(
                    &emp,
                    &VestingWithdrawTerms {
                        agreement_id: &id,
                        idempotency_key: &format!("t{i}"),
                        amount_zats: 30_000_000,
                        fee_estimate_zats: FEE,
                        max_fee_zats: policy.max_fee_for(FEE).unwrap(),
                        address_fingerprint: &fp,
                        lease_owner: "api-2",
                        lease_secs: 600,
                    },
                    &|pid| recipient_fingerprint(pid, ADDR),
                    ALL_ORGS,
                )
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().unwrap().unwrap())
        .collect();
    let created: Vec<_> = results.iter().filter(|o| !o.duplicate).collect();
    assert_eq!(created.len(), 1, "{results:?}");
    assert!(results.iter().all(|o| o.payout.id == created[0].payout.id));
    let emp = w.employee.clone();
    assert_eq!(w.db.list_vesting_claims(&emp, &a).unwrap().len(), 2);
    assert_eq!(w.bal(&a).reserved, 30_000_000);
}

/// Concurrent withdrawals mixing one shared key, distinct keys and amounts
/// above the available balance: exactly one payout is created, every other
/// request gets that payout back or is refused as insufficient, and nothing
/// beyond the created payout is reserved.
fn withdraw_concurrent_mixed_and_overdraw<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2);
    let available = w.bal(&a).available;
    const THREADS: usize = 12;
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|i| {
            let (fac, clock, emp, id, barrier, policy) = (
                w.f.clone(),
                w.clock.clone(),
                w.employee.clone(),
                a.clone(),
                barrier.clone(),
                w.policy.clone(),
            );
            // 0-3: one shared key (double taps); 4-7: distinct keys that fit;
            // 8-11: distinct keys asking for more than is available.
            let (key, amount) = match i {
                0..=3 => ("shared".to_string(), 10_000_000),
                4..=7 => (format!("fit{i}"), 15_000_000),
                _ => (format!("over{i}"), available + 1),
            };
            std::thread::spawn(move || {
                let mut db = fac.open(&clock);
                barrier.wait();
                let fp = agreement_fp(&id, ADDR);
                db.withdraw_vesting(
                    &emp,
                    &VestingWithdrawTerms {
                        agreement_id: &id,
                        idempotency_key: &key,
                        amount_zats: amount,
                        fee_estimate_zats: FEE,
                        max_fee_zats: policy.max_fee_for(FEE).unwrap(),
                        address_fingerprint: &fp,
                        lease_owner: "api-2",
                        lease_secs: 600,
                    },
                    &|pid| recipient_fingerprint(pid, ADDR),
                    ALL_ORGS,
                )
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let created: Vec<_> = results
        .iter()
        .filter_map(|r| r.as_ref().ok().filter(|o| !o.duplicate))
        .collect();
    assert_eq!(created.len(), 1, "{results:?}");
    let p = created[0].payout.clone();
    assert!(p.amount_zats == 10_000_000 || p.amount_zats == 15_000_000);
    for r in &results {
        match r {
            Ok(o) => assert_eq!(o.payout.id, p.id, "{results:?}"),
            Err(PayrollError::InsufficientAvailable { .. }) => {}
            Err(e) => panic!("unexpected {e:?}"),
        }
    }
    let emp = w.employee.clone();
    assert_eq!(w.db.list_vesting_claims(&emp, &a).unwrap().len(), 1);
    let b = w.bal(&a);
    assert_eq!((b.reserved, b.paid), (p.amount_zats, 0));
    assert_eq!(b.available, available - p.amount_zats);
    assert_eq!(b.vested, b.paid + b.reserved + b.available);
    // It is paid once; afterwards nothing else is reserved.
    let s = submit_payout(&mut w.db, &*w.wallet, &w.policy, &p, ADDR).unwrap();
    let s = w.confirm(&s);
    assert_eq!(s.state, PayoutState::Confirmed);
    let b = w.bal(&a);
    assert_eq!((b.reserved, b.paid), (0, p.amount_zats));
    assert_eq!(w.wallet.txs_built(), 1);
}

/// Who may set the address and withdraw, and against what.
fn withdraw_authorization<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready_with(0, 100 * DAY);
    let (emp, e, col, rival) = (
        w.employee.clone(),
        w.employer.clone(),
        w.colleague.clone(),
        w.rival.clone(),
    );
    // Unvested compensation can't be withdrawn (in the cliff: nothing vested).
    w.clock.advance(100 * DAY - 1);
    assert_eq!(insufficient(w.withdraw(&a, 1, "early")), 0);
    w.clock.advance(1); // 1 ZEC vested at the cliff
    assert_eq!(insufficient(w.withdraw(&a, COIN + 1, "too-much")), COIN);
    for who in [&col, &rival] {
        assert!(matches!(
            w.withdraw_as(who, &a, 1, "x", ADDR),
            Err(PayrollError::NotFound(_))
        ));
        assert!(matches!(
            w.db.set_vesting_payout_address(who, &a, &agreement_fp(&a, OTHER_ADDR), "s"),
            Err(PayrollError::NotFound(_))
        ));
    }
    assert!(matches!(
        w.withdraw_as(&e, &a, 1, "x", ADDR),
        Err(PayrollError::Forbidden(_))
    ));
    assert!(matches!(
        w.db.set_vesting_payout_address(&e, &a, &agreement_fp(&a, OTHER_ADDR), "s"),
        Err(PayrollError::Forbidden(_))
    ));
    // The address is locked: another one is refused, the same one is a repeat.
    assert!(matches!(
        w.db.set_vesting_payout_address(&emp, &a, &agreement_fp(&a, OTHER_ADDR), "s"),
        Err(PayrollError::Conflict(_))
    ));
    assert!(
        w.db.set_vesting_payout_address(&emp, &a, &agreement_fp(&a, ADDR), "s")
            .unwrap()
            .1
    );
    // A different destination than the locked one is refused.
    assert!(matches!(
        w.withdraw_as(&emp, &a, 1, "x", OTHER_ADDR),
        Err(PayrollError::InvalidState(_))
    ));
    // No address yet: refused.
    let e2 = w.employer.clone();
    let b =
        w.db.create_vesting(
            &e2,
            NewVesting {
                organization_id: w.org.clone(),
                employee_user_id: emp.clone(),
                total_zats: TOTAL,
                start_at: None,
                duration_secs: DAY,
                cliff_secs: None,
                method: VestingMethod::Linear,
            },
        )
        .unwrap();
    // A draft's address can't be set (the employee can't even see it).
    assert!(matches!(
        w.db.set_vesting_payout_address(&emp, &b.id, &agreement_fp(&b.id, ADDR), "s"),
        Err(PayrollError::NotFound(_))
    ));
    w.db.activate_vesting(&e2, &b.id).unwrap();
    w.clock.advance(DAY);
    assert!(matches!(
        w.withdraw(&b.id, 1, "x"),
        Err(PayrollError::InvalidState(_))
    ));
    // Payouts disabled for the organization.
    let fp = agreement_fp(&a, ADDR);
    let r = w.db.withdraw_vesting(
        &emp,
        &VestingWithdrawTerms {
            agreement_id: &a,
            idempotency_key: "off",
            amount_zats: 1,
            fee_estimate_zats: FEE,
            max_fee_zats: 2 * FEE,
            address_fingerprint: &fp,
            lease_owner: "api-1",
            lease_secs: 600,
        },
        &|id| recipient_fingerprint(id, ADDR),
        &|_| false,
    );
    assert!(matches!(r, Err(PayrollError::Forbidden(_))));
    // Bad terms.
    assert!(matches!(
        w.withdraw(&a, 0, "zero"),
        Err(PayrollError::Validation(_))
    ));
    assert!(matches!(
        w.withdraw(&a, 1, ""),
        Err(PayrollError::Validation(_))
    ));
    // Nothing above reserved anything.
    assert_eq!(w.bal(&a).reserved, 0);
    assert_eq!(w.wallet.txs_built(), 0);
}

// ---- recovery ----------------------------------------------------------------------

/// A crash before the reservation commits leaves nothing behind.
fn crash_before_reservation_commits<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2);
    w.f.fault(Fault::PayoutInsert, true);
    let e = w.withdraw(&a, 10_000_000, "k1").unwrap_err();
    assert!(format!("{e:?}").contains("injected crash"), "{e:?}");
    w.f.fault(Fault::PayoutInsert, false);
    let emp = w.employee.clone();
    assert!(
        w.db.list_vesting_claims(&emp, &a).unwrap().is_empty(),
        "the claim rolled back"
    );
    assert_eq!(w.bal(&a).reserved, 0);
    // The retry (same key) is a fresh request, paid once.
    let p = w.pay(&a, 10_000_000, "k1");
    assert_eq!(p.state, PayoutState::Submitted);
    assert_eq!(w.wallet.txs_built(), 1);
}

/// Committed reservation, but the process died before asking the wallet:
/// the worker fences the payment id, fails the payout and releases the claim.
/// A late submit of that payout can never pay.
fn crash_after_reservation_before_build<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2);
    let out = w.withdraw(&a, 10_000_000, "k1").unwrap();
    // Repeats while the lease is held return it.
    assert!(w.withdraw(&a, 10_000_000, "k1").unwrap().duplicate);
    assert!(w.reconcile("worker-1").is_empty(), "lease still held");
    w.clock.advance(w.policy.submit_lease_secs + 1);
    let r = w.reconcile("worker-1");
    assert_eq!(r[0].state, PayoutState::Failed);
    assert_eq!(
        w.claim(&a, &out.payout.claim_id).status,
        ClaimStatus::Released
    );
    assert_eq!(w.bal(&a).reserved, 0);
    // The crashed process comes back and submits: the wallet's fence stops it.
    let late = submit_payout(&mut w.db, &*w.wallet, &w.policy, &out.payout, ADDR).unwrap();
    assert_eq!(late.state, PayoutState::Failed);
    assert_eq!(w.wallet.txs_built(), 0);
    // The old key answers with the failed payout; a new attempt pays once.
    let old = w.withdraw(&a, 10_000_000, "k1").unwrap();
    assert!(old.duplicate && old.payout.state == PayoutState::Failed);
    let p = w.pay(&a, 10_000_000, "k2");
    let p = w.confirm(&p);
    assert_eq!(p.state, PayoutState::Confirmed);
    assert_eq!(w.wallet.txs_built(), 1);
    assert_eq!(w.bal(&a).paid, 10_000_000);
}

/// The signed transaction was stored but never acknowledged by the network.
fn crash_after_signed_tx_stored<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2);
    w.wallet.then(SubmitMode::BuiltOnly);
    let p = w.pay(&a, 10_000_000, "k1");
    assert_eq!(p.state, PayoutState::RequiresReconciliation);
    assert!(p.txid.is_some(), "the stored transaction is recorded");
    assert_eq!(w.claim(&a, &p.claim_id).status, ClaimStatus::Reserved);
    // Repeats return it; reconciling before the rebroadcast changes nothing.
    assert_eq!(w.withdraw(&a, 10_000_000, "k1").unwrap().payout.id, p.id);
    assert_eq!(w.withdraw(&a, 1, "k9").unwrap().payout.id, p.id);
    w.reconcile("worker-1");
    assert_eq!(w.payout(&p.id).state, PayoutState::RequiresReconciliation);
    // The wallet rebroadcasts it; then it confirms and settles once.
    w.wallet.acknowledge_broadcasts();
    w.reconcile("worker-1");
    let p = w.payout(&p.id);
    assert_eq!(p.state, PayoutState::Submitted);
    let p = w.confirm(&p);
    assert_eq!(p.state, PayoutState::Confirmed);
    assert_eq!(w.claim(&a, &p.claim_id).status, ClaimStatus::Settled);
    assert_eq!(w.wallet.txs_built(), 1);
}

/// Broadcast happened but the response was lost; or the request was lost.
fn crash_after_broadcast<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2);
    w.wallet.then(SubmitMode::LostResponse);
    let p = w.pay(&a, 10_000_000, "k1");
    assert_eq!(p.state, PayoutState::RequiresReconciliation);
    assert!(p.txid.is_none());
    // Retries after the broadcast don't build again.
    assert!(w.withdraw(&a, 10_000_000, "k1").unwrap().duplicate);
    w.reconcile("worker-1");
    let p = w.payout(&p.id);
    assert_eq!(
        p.state,
        PayoutState::Submitted,
        "adopted from the wallet's record"
    );
    let p = w.confirm(&p);
    assert_eq!(p.state, PayoutState::Confirmed);
    assert_eq!(w.wallet.txs_built(), 1);

    // The request itself never reached the wallet: fenced, failed, released.
    w.wallet.then(SubmitMode::LostRequest);
    let q = w.pay(&a, 20_000_000, "k2");
    assert_eq!(q.state, PayoutState::RequiresReconciliation);
    w.reconcile("worker-1");
    assert_eq!(w.payout(&q.id).state, PayoutState::Failed);
    assert_eq!(w.claim(&a, &q.claim_id).status, ClaimStatus::Released);
    assert_eq!(w.wallet.txs_built(), 1);
    // And a wallet-side refusal (no transaction) fails and releases at once.
    w.wallet.then(SubmitMode::FailNoTx("insufficient funds"));
    let r = w.pay(&a, 20_000_000, "k3");
    assert_eq!(r.state, PayoutState::Failed);
    assert_eq!(w.claim(&a, &r.claim_id).status, ClaimStatus::Released);
    let b = w.bal(&a);
    assert_eq!((b.paid, b.reserved), (10_000_000, 0));
    assert_eq!(w.wallet.txs_built(), 1);
}

/// Confirmed on chain, but the process died while settling; a restarted
/// worker settles it exactly once.
fn crash_after_confirmation_before_settlement<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2);
    let p = w.pay(&a, 10_000_000, "k1");
    w.wallet
        .mine_to(p.txid.as_deref().unwrap(), w.policy.required_confirmations);
    w.f.fault(Fault::Settlement, true);
    let leased = w.db.lease_due_payouts("w1", 60, 10).unwrap();
    let d = reconcile_decision(&leased[0], &*w.wallet, &w.policy);
    assert!(matches!(d, Decision::Confirmed(_)));
    let e =
        w.db.apply_payout_decision(&p.id, "w1", PayoutState::Submitted, &d)
            .unwrap_err();
    assert!(format!("{e:?}").contains("injected crash"), "{e:?}");
    assert_eq!(w.payout(&p.id).state, PayoutState::Submitted, "rolled back");
    assert_eq!(w.claim(&a, &p.claim_id).status, ClaimStatus::Reserved);
    // A repeat request meanwhile returns the same payout.
    assert_eq!(w.withdraw(&a, 10_000_000, "k1").unwrap().payout.id, p.id);
    w.f.fault(Fault::Settlement, false);
    // Restart: a new process (new connection, new lease owner) after the
    // crashed lease expires.
    w.clock.advance(61);
    let mut db2 = w.f.open(&w.clock);
    let r = reconcile_due(&mut db2, &*w.wallet, &w.policy, "w2", 50).unwrap();
    assert_eq!(r[0].state, PayoutState::Confirmed);
    assert_eq!(w.claim(&a, &p.claim_id).status, ClaimStatus::Settled);
    assert!(
        reconcile_due(&mut db2, &*w.wallet, &w.policy, "w2", 50)
            .unwrap()
            .is_empty()
    );
    assert_eq!(w.bal(&a).paid, 10_000_000);
    assert_eq!(w.wallet.txs_built(), 1);
}

// ---- lifecycle with payouts -----------------------------------------------------------

/// Cancellation never disturbs a payout: in-flight ones settle (or release)
/// normally, the vested remainder stays withdrawable, nothing beyond it.
fn cancellation_with_payouts<F: Factory>(f: F) {
    let mut w = world(f);
    let a = w.ready();
    w.clock.advance(DURATION / 2); // 2 ZEC vested
    let p = w.pay(&a, 50_000_000, "k1");
    assert_eq!(p.state, PayoutState::Submitted);
    let e = w.employer.clone();
    let c = w.db.cancel_vesting(&e, &a).unwrap();
    assert_eq!(c.vested_at_cancel_zats, Some(2 * COIN));
    // Repeats still return the in-flight payout; it then confirms and settles.
    assert_eq!(w.withdraw(&a, 50_000_000, "k1").unwrap().payout.id, p.id);
    let p = w.confirm(&p);
    assert_eq!(p.state, PayoutState::Confirmed);
    w.clock.advance(DURATION);
    let b = w.bal(&a);
    assert_eq!(
        (b.vested, b.paid, b.available),
        (2 * COIN, 50_000_000, 150_000_000)
    );
    // A failed withdrawal after cancellation releases back into what is owed.
    w.wallet.then(SubmitMode::FailNoTx("insufficient funds"));
    let q = w.pay(&a, 150_000_000, "k2");
    assert_eq!(q.state, PayoutState::Failed);
    assert_eq!(w.bal(&a).available, 150_000_000);
    // Nothing beyond the frozen vested amount; exactly the rest is paid.
    assert_eq!(insufficient(w.withdraw(&a, 150_000_001, "k3")), 150_000_000);
    let r = w.pay(&a, 150_000_000, "k4");
    w.confirm(&r);
    let b = w.bal(&a);
    assert_eq!((b.paid, b.available, b.unvested), (2 * COIN, 0, 2 * COIN));
    assert_eq!(w.wallet.txs_built(), 2);
}

/// Activation happens at or before the start; later it is refused.
fn activation_before_start_only<F: Factory>(f: F) {
    let mut w = world(f);
    let e = w.employer.clone();
    let mk = |w: &mut World<F>, start_in: i64| {
        let req = NewVesting {
            organization_id: w.org.clone(),
            employee_user_id: w.employee.clone(),
            total_zats: TOTAL,
            start_at: Some(w.clock.now() + start_in),
            duration_secs: DURATION,
            cliff_secs: None,
            method: VestingMethod::Linear,
        };
        w.db.create_vesting(&e, req).unwrap()
    };
    let early = mk(&mut w, DAY);
    let exact = mk(&mut w, DAY);
    let late = mk(&mut w, DAY);
    w.db.activate_vesting(&e, &early.id).unwrap();
    w.clock.advance(DAY);
    let x = w.db.activate_vesting(&e, &exact.id).unwrap(); // at the start: nothing vested yet
    assert_eq!(x.activated_at, Some(x.terms.start_at));
    w.clock.advance(1);
    let r = w.db.activate_vesting(&e, &late.id);
    assert!(
        matches!(&r, Err(PayrollError::InvalidState(m)) if m.contains("start date has passed")),
        "{r:?}"
    );
    assert_eq!(
        w.db.get_vesting(&e, &late.id).unwrap().status,
        VestingStatus::Draft
    );
    // The schema refuses it too.
    let now = w.clock.now();
    let err =
        w.f.raw(&format!(
            "UPDATE vesting_agreements SET status = 'active', activated_at = {now},
                 activated_by = '{e}', updated_at = {now} WHERE id = '{}'",
            late.id
        ))
        .unwrap_err();
    assert!(err.contains("activated before its start"), "{err}");
    // An expired draft can only be voided.
    let v = w.db.cancel_vesting(&e, &late.id).unwrap();
    assert_eq!(
        (v.status, v.vested_at_cancel_zats),
        (VestingStatus::Cancelled, Some(0))
    );
}

// ---- upgrade -----------------------------------------------------------------------

const U_OWNER: &str = "00000000-0000-0000-0000-0000000000a1";
const U_WORKER: &str = "00000000-0000-0000-0000-0000000000a2";
const U_ORG: &str = "00000000-0000-0000-0000-0000000000b1";
const U_STREAM: &str = "00000000-0000-0000-0000-0000000000c1";
const U_C1: &str = "00000000-0000-0000-0000-0000000000d1";
const U_C2: &str = "00000000-0000-0000-0000-0000000000d2";
const U_P1: &str = "00000000-0000-0000-0000-0000000000e1";
const U_P2: &str = "00000000-0000-0000-0000-0000000000e2";

fn u_tx(n: u8) -> String {
    format!("{n:064x}")
}

/// Stream claims and payouts as an existing (pre-vesting-payouts) database
/// holds them: P1 submitted (in flight), P2 confirmed with its claim settled.
fn legacy_stream_payouts_sql() -> String {
    let (fp1, fp2) = (
        recipient_fingerprint(U_P1, ADDR),
        recipient_fingerprint(U_P2, ADDR),
    );
    let (tx1, tx2) = (u_tx(0xa1), u_tx(0xa2));
    format!(
        "INSERT INTO users (id, email, display_name, created_at) VALUES
             ('{U_OWNER}', 'o@legacy.test', 'Owner', 1), ('{U_WORKER}', 'w@legacy.test', 'Worker', 1);
         INSERT INTO organizations (id, name, created_by, created_at) VALUES ('{U_ORG}', 'Legacy', '{U_OWNER}', 1);
         INSERT INTO organization_members (organization_id, user_id, role, created_at) VALUES
             ('{U_ORG}', '{U_OWNER}', 'owner', 1), ('{U_ORG}', '{U_WORKER}', 'member', 1);
         INSERT INTO payroll_streams (id, organization_id, recipient_user_id, created_by, rate_zats,
             rate_period_secs, start_at, end_at, status, banked_active_secs, active_since, ended_at,
             created_at, last_event_at)
             VALUES ('{U_STREAM}', '{U_ORG}', '{U_WORKER}', '{U_OWNER}', 100000000, 3600, 1000, NULL,
                     'active', 0, 1000, NULL, 1000, 1000);
         INSERT INTO claims (id, stream_id, requested_by, idempotency_key, amount_zats, status,
             earned_at_reservation_zats, created_at) VALUES
             ('{U_C1}', '{U_STREAM}', '{U_WORKER}', 'k1', 1000, 'reserved', 100000, 2000),
             ('{U_C2}', '{U_STREAM}', '{U_WORKER}', 'k2', 2000, 'reserved', 100000, 2000);
         INSERT INTO payouts (id, claim_id, stream_id, requested_by, amount_zats, recipient_fingerprint,
             fee_estimate_zats, max_fee_zats, state, confirmations, created_at, updated_at) VALUES
             ('{U_P1}', '{U_C1}', '{U_STREAM}', '{U_WORKER}', 1000, '{fp1}', 10000, 20000, 'reserved', 0, 2000, 2000),
             ('{U_P2}', '{U_C2}', '{U_STREAM}', '{U_WORKER}', 2000, '{fp2}', 10000, 20000, 'reserved', 0, 2000, 2000);
         UPDATE payouts SET state = 'submitted', txid = '{tx1}', submitted_at = 2001, updated_at = 2001 WHERE id = '{U_P1}';
         UPDATE payouts SET state = 'submitted', txid = '{tx2}', submitted_at = 2001, updated_at = 2001 WHERE id = '{U_P2}';
         UPDATE payouts SET state = 'confirmed', mined_height = 500, confirmations = 10, resolved_at = 2002,
             updated_at = 2002 WHERE id = '{U_P2}';
         UPDATE claims SET status = 'settled', resolved_at = 2002, resolved_by = '{U_WORKER}',
             settlement_ref = '{tx2}' WHERE id = '{U_C2}';
         INSERT INTO payout_events (payout_id, from_state, to_state, occurred_at) VALUES
             ('{U_P1}', NULL, 'reserved', 2000), ('{U_P2}', NULL, 'reserved', 2000),
             ('{U_P1}', 'reserved', 'submitted', 2001), ('{U_P2}', 'reserved', 'submitted', 2001),
             ('{U_P2}', 'submitted', 'confirmed', 2002);"
    )
}

/// After the upgrade, existing stream payouts are intact and keep working:
/// listed, audited, and the in-flight one confirms and settles once.
fn check_upgraded<B: Backend>(mut db: Payroll<B>, clock: &ManualClock) {
    let payouts = db.list_payouts(U_WORKER, U_STREAM).unwrap();
    let got: Vec<_> = payouts
        .iter()
        .map(|p| {
            (
                p.id.as_str(),
                p.compensation_type,
                p.claim_id.as_str(),
                p.source_id.as_str(),
                p.state,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (
                U_P1,
                CompensationType::Stream,
                U_C1,
                U_STREAM,
                PayoutState::Submitted
            ),
            (
                U_P2,
                CompensationType::Stream,
                U_C2,
                U_STREAM,
                PayoutState::Confirmed
            ),
        ]
    );
    let (_, ev) = db.payout_history(U_WORKER, U_P2).unwrap();
    assert_eq!(
        states(&ev),
        [
            PayoutState::Reserved,
            PayoutState::Submitted,
            PayoutState::Confirmed
        ]
    );
    let b = db.balance(U_WORKER, U_STREAM).unwrap();
    assert_eq!((b.reserved, b.claimed), (1000, 2000));
    assert_eq!(db.claim_payout(U_WORKER, U_C1).unwrap().unwrap().id, U_P1);

    // The in-flight payout is reconciled to confirmed by the new code.
    let wallet = MockWallet::new();
    wallet.adopt(U_P1, ADDR, 1000, &u_tx(0xa1));
    let policy = PayoutPolicy::default();
    wallet.mine_to(&u_tx(0xa1), policy.required_confirmations);
    let r = reconcile_due(&mut db, &*wallet, &policy, "w", 10).unwrap();
    assert_eq!((r.len(), r[0].state), (1, PayoutState::Confirmed));
    let b = db.balance(U_WORKER, U_STREAM).unwrap();
    assert_eq!((b.reserved, b.claimed), (0, 3000));
    let (_, ev) = db.payout_history(U_WORKER, U_P1).unwrap();
    assert_eq!(
        states(&ev),
        [
            PayoutState::Reserved,
            PayoutState::Submitted,
            PayoutState::Confirmed
        ]
    );

    // Vesting payouts work on the upgraded database too.
    let a = db
        .create_vesting(
            U_OWNER,
            NewVesting {
                organization_id: U_ORG.into(),
                employee_user_id: U_WORKER.into(),
                total_zats: TOTAL,
                start_at: None,
                duration_secs: DAY,
                cliff_secs: None,
                method: VestingMethod::Linear,
            },
        )
        .unwrap();
    db.activate_vesting(U_OWNER, &a.id).unwrap();
    db.set_vesting_payout_address(U_WORKER, &a.id, &agreement_fp(&a.id, ADDR), "sealed")
        .unwrap();
    clock.advance(DAY);
    let fp = agreement_fp(&a.id, ADDR);
    let out = db
        .withdraw_vesting(
            U_WORKER,
            &VestingWithdrawTerms {
                agreement_id: &a.id,
                idempotency_key: "v1",
                amount_zats: TOTAL,
                fee_estimate_zats: FEE,
                max_fee_zats: 2 * FEE,
                address_fingerprint: &fp,
                lease_owner: "api",
                lease_secs: 600,
            },
            &|id| recipient_fingerprint(id, ADDR),
            ALL_ORGS,
        )
        .unwrap();
    let p = submit_payout(&mut db, &*wallet, &policy, &out.payout, ADDR).unwrap();
    wallet.mine_to(p.txid.as_deref().unwrap(), policy.required_confirmations);
    reconcile_due(&mut db, &*wallet, &policy, "w", 10).unwrap();
    assert_eq!(db.vesting_balance(U_WORKER, &a.id).unwrap().paid, TOTAL);
    assert_eq!(
        db.list_payouts(U_WORKER, U_STREAM).unwrap().len(),
        2,
        "stream payouts unchanged"
    );
}

macro_rules! scenarios {
    ($factory:ty $(, #[$attr:meta])?) => {
        #[test] $(#[$attr])? fn withdraw_submit_confirm() { super::withdraw_submit_confirm(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn payout_reference_integrity() { super::payout_reference_integrity(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn attached_claim_paths() { super::attached_claim_paths(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn withdraw_idempotent_and_concurrent() { super::withdraw_idempotent_and_concurrent(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn withdraw_concurrent_mixed_and_overdraw() { super::withdraw_concurrent_mixed_and_overdraw(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn withdraw_authorization() { super::withdraw_authorization(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn crash_before_reservation_commits() { super::crash_before_reservation_commits(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn crash_after_reservation_before_build() { super::crash_after_reservation_before_build(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn crash_after_signed_tx_stored() { super::crash_after_signed_tx_stored(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn crash_after_broadcast() { super::crash_after_broadcast(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn crash_after_confirmation_before_settlement() { super::crash_after_confirmation_before_settlement(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn cancellation_with_payouts() { super::cancellation_with_payouts(<$factory>::fresh()) }
        #[test] $(#[$attr])? fn activation_before_start_only() { super::activation_before_start_only(<$factory>::fresh()) }
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
        fn fault(&self, f: Fault, on: bool) {
            let (name, ddl) = match f {
                Fault::Settlement => (
                    "fault_settlement",
                    "BEFORE UPDATE OF status ON vesting_claims
                     BEGIN SELECT RAISE(ABORT, 'injected crash during settlement'); END;",
                ),
                Fault::PayoutInsert => (
                    "fault_payout_insert",
                    "BEFORE INSERT ON payouts
                     BEGIN SELECT RAISE(ABORT, 'injected crash before commit'); END;",
                ),
            };
            self.raw(&if on {
                format!("CREATE TRIGGER {name} {ddl}")
            } else {
                format!("DROP TRIGGER {name};")
            })
            .unwrap();
        }
    }

    scenarios!(Sqlite);

    /// A v4 database (vesting, but stream-only payouts) with live stream
    /// payouts upgrades to v5 with every row intact.
    #[test]
    fn upgrade_keeps_stream_payouts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        {
            let c = rusqlite::Connection::open(&path).unwrap();
            c.execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL,
                     applied_at INTEGER NOT NULL) STRICT;",
            )
            .unwrap();
            for m in hushush_payroll::migrations::MIGRATIONS
                .iter()
                .filter(|m| m.version <= 4)
            {
                c.execute_batch(m.sql).unwrap();
                c.execute(
                    "INSERT INTO schema_migrations VALUES (?1, ?2, 0)",
                    rusqlite::params![m.version, m.name],
                )
                .unwrap();
            }
            c.pragma_update(None, "user_version", 4).unwrap();
            c.execute_batch(&legacy_stream_payouts_sql()).unwrap();
        }
        let clock = ManualClock::at(T0);
        let db = PayrollDb::open(&path, Arc::new(clock.clone())).unwrap();
        let v: i64 = db
            .connection()
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(v, 5);
        let fk: Vec<String> = db
            .connection()
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(fk.is_empty(), "{fk:?}");
        // The rebuilt tables keep their guards.
        let e = db
            .connection()
            .execute(&format!("DELETE FROM payouts WHERE id = '{U_P2}'"), [])
            .unwrap_err();
        assert!(e.to_string().contains("cannot be deleted"), "{e}");
        let e = db
            .connection()
            .execute("UPDATE payout_events SET detail = 'x'", [])
            .unwrap_err();
        assert!(e.to_string().contains("append-only"), "{e}");
        check_upgraded(db, &clock);
    }
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
    impl Pg {
        fn client(&self) -> ::postgres::Client {
            let mut c = hushush_payroll::pg::connect_client(&self.url).unwrap();
            hushush_payroll::pg::use_schema(&mut c, &self.schema).unwrap();
            c
        }
    }
    impl Factory for Pg {
        type B = PgBackend;
        fn open(&self, clock: &ManualClock) -> PgPayrollDb {
            PgPayrollDb::connect(&self.url, Some(&self.schema), Arc::new(clock.clone())).unwrap()
        }
        fn raw(&self, sql: &str) -> Result<(), String> {
            self.client().batch_execute(sql).map_err(|e| {
                e.as_db_error()
                    .map_or_else(|| e.to_string(), |d| d.message().to_string())
            })
        }
        fn fault(&self, f: Fault, on: bool) {
            let (name, table, when, msg) = match f {
                Fault::Settlement => (
                    "fault_settlement",
                    "vesting_claims",
                    "BEFORE UPDATE",
                    "injected crash during settlement",
                ),
                Fault::PayoutInsert => (
                    "fault_payout_insert",
                    "payouts",
                    "BEFORE INSERT",
                    "injected crash before commit",
                ),
            };
            self.raw(&if on {
                format!(
                    "CREATE FUNCTION {name}() RETURNS trigger LANGUAGE plpgsql AS $$
                     BEGIN RAISE EXCEPTION '{msg}'; END $$;
                     CREATE TRIGGER {name} {when} ON {table} FOR EACH ROW EXECUTE FUNCTION {name}();"
                )
            } else {
                format!("DROP TRIGGER {name} ON {table}; DROP FUNCTION {name}();")
            })
            .unwrap();
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

    /// A v5 database (vesting, but stream-only payouts) with live stream
    /// payouts upgrades to v6 with every row intact.
    #[test]
    #[ignore = "requires PAYROLL_TEST_DATABASE_URL"]
    fn upgrade_keeps_stream_payouts() {
        let pg = Pg::fresh();
        {
            let mut c = pg.client();
            c.batch_execute(
                "CREATE TABLE payroll_schema_migrations (version BIGINT PRIMARY KEY, name TEXT NOT NULL,
                     applied_at BIGINT NOT NULL)",
            )
            .unwrap();
            for m in hushush_payroll::migrations::PG_MIGRATIONS
                .iter()
                .filter(|m| m.version <= 5)
            {
                c.batch_execute(m.sql).unwrap();
                c.execute(
                    "INSERT INTO payroll_schema_migrations VALUES ($1, $2, 0)",
                    &[&m.version, &m.name],
                )
                .unwrap();
            }
            c.batch_execute(&legacy_stream_payouts_sql()).unwrap();
        }
        let clock = ManualClock::at(T0);
        let mut db = pg.open(&clock);
        let v: i64 = db
            .client()
            .query_one("SELECT MAX(version) FROM payroll_schema_migrations", &[])
            .unwrap()
            .get(0);
        assert_eq!(v, 6);
        check_upgraded(db, &clock);
    }
}

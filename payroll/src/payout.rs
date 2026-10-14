//! Shielded payouts of reserved claims (Phase 3).
//!
//! The payroll engine stays the authority for *what* is paid (claim amount,
//! ownership, reservation). The payout wallet service decides nothing about
//! payroll; it builds at most one shielded transaction per payment id. This
//! module joins the two:
//!
//! * [`Payroll::request_payout`] records a `reserved` payout for a reserved
//!   claim, persistently and before any wallet call. `UNIQUE (claim_id)` makes
//!   it impossible to create a second payout for the same claim.
//! * [`submit_payout`] asks the wallet to pay and records the result.
//! * [`reconcile_due`] is the worker step: it leases unfinished payouts and
//!   moves them forward from what the wallet and chain report — including
//!   after a crash or restart.
//! * [`decide`] is the single pure function mapping (payout, wallet report,
//!   policy) to the next state. Everything ambiguous maps to
//!   `requires_reconciliation`, never to `failed` and never to a resubmission.
//!
//! The destination address only passes through memory on its way to the
//! wallet; the payroll database stores a salted fingerprint of it
//! ([`recipient_fingerprint`]), which is enough to verify what was paid.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::{Payroll, load_stream, new_id, require_can_view, validate_id};
use crate::error::{PayrollError, Result};
use crate::model::*;
use crate::money::MAX_MONEY_ZATS;
use crate::store::{Backend, StoreTx};

/// Salted SHA-256 of a destination address, bound to one payment id, so the
/// payroll database cannot link payouts to the same address and the address
/// is not recoverable without already knowing it. The wallet service computes
/// the same value; both sides compare fingerprints, never addresses.
pub fn recipient_fingerprint(payment_id: &str, address: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"hushush-payout-recipient-v1\0");
    h.update(payment_id.as_bytes());
    h.update(b"\0");
    h.update(address.trim().as_bytes());
    hex_lower(&h.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Operational policy for payouts. Defaults are conservative.
#[derive(Clone, Debug)]
pub struct PayoutPolicy {
    /// A payout is confirmed only at this depth (1 = mined in the tip block).
    pub required_confirmations: i64,
    /// An unmined transaction is declared failed only once the chain tip is
    /// this many blocks past its expiry height (reorg safety margin).
    pub expiry_margin_blocks: i64,
    /// Absolute cap on the fee of any payout transaction.
    pub max_fee_zats: i64,
    /// Allowed increase of the actual fee over the quoted estimate (notes can
    /// change between quote and build).
    pub fee_headroom_zats: i64,
    /// Lease held by the request that creates a payout while it submits.
    /// Must exceed the wallet submit timeout.
    pub submit_lease_secs: i64,
    /// Lease held by the background worker per payout it examines.
    pub worker_lease_secs: i64,
}

impl Default for PayoutPolicy {
    fn default() -> Self {
        Self {
            required_confirmations: 10,
            expiry_margin_blocks: 20,
            max_fee_zats: 100_000,
            fee_headroom_zats: 10_000,
            submit_lease_secs: 600,
            worker_lease_secs: 120,
        }
    }
}

impl PayoutPolicy {
    pub fn validate(&self) -> Result<()> {
        let ok = self.required_confirmations >= 1
            && self.expiry_margin_blocks >= 1
            && (0..=MAX_MONEY_ZATS).contains(&self.max_fee_zats)
            && self.fee_headroom_zats >= 0
            && self.submit_lease_secs >= 30
            && self.worker_lease_secs >= 10;
        if ok {
            Ok(())
        } else {
            Err(PayrollError::Validation("invalid payout policy".into()))
        }
    }

    /// The most a payout quoted at `estimate` may pay in fees, or `None` if
    /// the estimate itself exceeds the cap.
    pub fn max_fee_for(&self, estimate: i64) -> Option<i64> {
        (0..=self.max_fee_zats)
            .contains(&estimate)
            .then(|| (estimate + self.fee_headroom_zats).min(self.max_fee_zats))
    }
}

// ---- wallet contract -------------------------------------------------------------

/// A payment request sent to the wallet service. The wallet derives the memo
/// from `payment_id` itself.
#[derive(Clone, Debug, Serialize)]
pub struct WalletSubmit {
    pub payment_id: String,
    pub address: String,
    pub amount_zats: i64,
    pub max_fee_zats: i64,
}

/// The wallet's state for one payment id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalletPaymentState {
    /// Accepted; the transaction is being built.
    Pending,
    /// No transaction was built and none ever will be for this id.
    Failed,
    /// A transaction was built and stored; the network has not acknowledged it.
    Built,
    /// The network accepted the transaction (it may still be unmined).
    Broadcast,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainStatus {
    pub tip_height: i64,
    pub mined_height: Option<i64>,
    pub confirmations: i64,
    pub in_mempool: bool,
    /// The wallet's own scanned view (compact-block scanning, a second code
    /// path next to `GetTransaction`). `None` = the wallet has no record.
    #[serde(default)]
    pub wallet_mined_height: Option<i64>,
    #[serde(default)]
    pub wallet_expired_unmined: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedOutput {
    pub recipient_fingerprint: String,
    pub amount_zats: i64,
    /// The memo carries this payment's reference.
    pub memo_matches: bool,
    pub pool: String,
}

/// What the wallet verified about the transaction, by decrypting its
/// outgoing outputs with the wallet's viewing key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxVerification {
    /// `"chain"` when checked against the transaction as served by the node,
    /// `"local"` when checked against the wallet's own copy.
    pub source: String,
    pub fully_shielded: bool,
    /// All value moved inside one shielded pool (no amount-revealing crossing).
    pub single_pool: bool,
    pub fee_zats: Option<i64>,
    /// Outputs paid to someone other than the wallet itself (change excluded).
    pub outputs: Vec<VerifiedOutput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletPayment {
    pub payment_id: String,
    pub state: WalletPaymentState,
    pub recipient_fingerprint: String,
    pub amount_zats: i64,
    #[serde(default)]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub txid: Option<String>,
    #[serde(default)]
    pub fee_zats: Option<i64>,
    #[serde(default)]
    pub expiry_height: Option<i64>,
    #[serde(default)]
    pub chain: Option<ChainStatus>,
    #[serde(default)]
    pub verification: Option<TxVerification>,
}

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    /// Transport failure, timeout or server error: the outcome is unknown.
    #[error("wallet unavailable: {0}")]
    Unavailable(String),
    /// The wallet refused the request without creating a payment record.
    #[error("wallet refused request ({code}): {message}")]
    Refused { code: String, message: String },
}

/// The payout wallet service, as seen by the payroll side.
///
/// Contract the wallet must honour: `submit` and `cancel` are idempotent per
/// `payment_id`; one payment id yields at most one transaction, ever.
pub trait WalletApi {
    /// Pays (or returns the existing record for) `req.payment_id`.
    fn submit(&self, req: &WalletSubmit) -> std::result::Result<WalletPayment, WalletError>;
    /// Current record for `payment_id`, with chain status and verification.
    fn lookup(&self, payment_id: &str) -> std::result::Result<Option<WalletPayment>, WalletError>;
    /// Fences `payment_id`: if the wallet has no record yet, it records the id
    /// as failed so a late or repeated submit can never pay it. Returns the
    /// (possibly pre-existing) record.
    fn cancel(&self, payment_id: &str) -> std::result::Result<WalletPayment, WalletError>;
}

// ---- decisions -------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxObservation {
    pub txid: String,
    pub fee_zats: Option<i64>,
    pub expiry_height: Option<i64>,
    pub mined_height: Option<i64>,
    pub confirmations: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to change now; look again later.
    Wait,
    /// A transaction exists and the network has it; record or refresh it.
    Submitted(TxObservation),
    /// Deep enough under the policy and verified: settle the claim.
    Confirmed(TxObservation),
    /// Certain that nothing was or can be mined: release the claim.
    Failed { reason: String },
    /// Ambiguous or inconsistent: keep the claim locked and investigate.
    NeedsReconciliation {
        reason: String,
        tx: Option<TxObservation>,
    },
}

fn reconcile(reason: impl Into<String>, tx: Option<TxObservation>) -> Decision {
    Decision::NeedsReconciliation {
        reason: reason.into(),
        tx,
    }
}

/// Checks a verification report against the payout.
pub fn verify(p: &Payout, v: &TxVerification) -> std::result::Result<(), String> {
    if !v.fully_shielded {
        return Err("transaction is not fully shielded".into());
    }
    if !v.single_pool {
        return Err("transaction moves value between shielded pools".into());
    }
    if let Some(fee) = v.fee_zats
        && fee > p.max_fee_zats
    {
        return Err(format!("fee {fee} exceeds the allowed {}", p.max_fee_zats));
    }
    match v.outputs.as_slice() {
        [o] if o.recipient_fingerprint != p.recipient_fingerprint => {
            Err("paid to a different recipient".into())
        }
        [o] if o.amount_zats != p.amount_zats => Err(format!(
            "paid {} zats instead of {}",
            o.amount_zats, p.amount_zats
        )),
        [o] if !o.memo_matches => Err("memo does not reference this payout".into()),
        [_] => Ok(()),
        outs => Err(format!(
            "expected exactly one external output, found {}",
            outs.len()
        )),
    }
}

/// Maps the wallet's report for a payout to the next payout state.
pub fn decide(p: &Payout, w: &WalletPayment, policy: &PayoutPolicy) -> Decision {
    if w.payment_id != p.id {
        return reconcile("wallet returned a different payment", None);
    }
    // A failed record (including a fence placed by `cancel`, which never saw
    // the request) proves nothing was paid, so its details need not match.
    if w.state != WalletPaymentState::Failed
        && (w.recipient_fingerprint != p.recipient_fingerprint || w.amount_zats != p.amount_zats)
    {
        return reconcile("wallet payment details do not match the payout", None);
    }
    match w.state {
        WalletPaymentState::Failed => match &p.txid {
            Some(_) => reconcile("wallet reports no transaction, but one was recorded", None),
            None => Decision::Failed {
                reason: w
                    .failure_reason
                    .clone()
                    .unwrap_or_else(|| "the wallet did not create a transaction".into()),
            },
        },
        WalletPaymentState::Pending => match p.state {
            PayoutState::Reserved => {
                reconcile("the wallet has not finished building the transaction", None)
            }
            _ => Decision::Wait,
        },
        WalletPaymentState::Built | WalletPaymentState::Broadcast => {
            let Some(txid) = w.txid.clone() else {
                return reconcile("wallet reports a transaction without an id", None);
            };
            if p.txid.as_ref().is_some_and(|t| *t != txid) {
                return reconcile("wallet reports a different transaction id", None);
            }
            let chain = w.chain.as_ref();
            let tx = TxObservation {
                txid,
                fee_zats: w.fee_zats,
                expiry_height: w.expiry_height,
                mined_height: chain.and_then(|c| c.mined_height),
                confirmations: chain.map_or(0, |c| c.confirmations.max(0)),
            };
            if w.fee_zats.is_some_and(|f| f > p.max_fee_zats) {
                return reconcile("transaction fee exceeds the allowed maximum", Some(tx));
            }
            if let Some(v) = &w.verification
                && let Err(why) = verify(p, v)
            {
                return reconcile(format!("verification failed: {why}"), Some(tx));
            }
            let broadcast = w.state == WalletPaymentState::Broadcast;
            match chain {
                Some(c) if c.mined_height.is_some() => {
                    let chain_verified =
                        w.verification.as_ref().is_some_and(|v| v.source == "chain");
                    if c.confirmations >= policy.required_confirmations && chain_verified {
                        Decision::Confirmed(tx)
                    } else {
                        Decision::Submitted(tx)
                    }
                }
                // Failing releases the claim, so it needs two agreeing views:
                // the node no longer has the transaction anywhere, AND the
                // wallet's own scan marks it expired unmined. Anything less
                // (including a server on a stale fork) waits.
                Some(c)
                    if !c.in_mempool
                        && c.wallet_expired_unmined == Some(true)
                        && c.wallet_mined_height.is_none()
                        && w.expiry_height.is_some_and(|e| {
                            e > 0 && c.tip_height >= e + policy.expiry_margin_blocks
                        }) =>
                {
                    Decision::Failed {
                        reason: format!(
                            "transaction expired at height {} without being mined",
                            w.expiry_height.unwrap_or_default()
                        ),
                    }
                }
                _ if broadcast => Decision::Submitted(tx),
                _ => reconcile(
                    "the network has not acknowledged the transaction yet; the wallet will rebroadcast it",
                    Some(tx),
                ),
            }
        }
    }
}

// ---- engine operations -----------------------------------------------------------

/// Terms of a payout request, computed by the caller from the wallet's quote.
#[derive(Clone, Debug)]
pub struct PayoutTerms<'a> {
    pub claim_id: &'a str,
    pub fee_estimate_zats: i64,
    pub max_fee_zats: i64,
    /// Lease taken by the requesting process while it submits to the wallet.
    pub lease_owner: &'a str,
    pub lease_secs: i64,
}

fn truncate(s: &str) -> String {
    s.chars().take(500).collect()
}

impl<B: Backend> Payroll<B> {
    /// Checks that `actor` may pay out `claim_id` now, and returns the claim
    /// (whose amount is authoritative). Used to quote before requesting.
    pub fn payout_eligibility(
        &mut self,
        actor: &str,
        claim_id: &str,
        org_allowed: &dyn Fn(&str) -> bool,
    ) -> Result<Claim> {
        validate_id("claim", claim_id)?;
        let mut tx = self.backend.begin(false)?;
        let (c, s) = load_claim_for_payout(&mut tx, actor, claim_id, false)?;
        if tx.payout_by_claim(claim_id, false)?.is_some() {
            return Err(PayrollError::InvalidState(
                "a payout was already requested for this claim".into(),
            ));
        }
        check_payable(&c, &s, org_allowed)?;
        Ok(c)
    }

    /// Records a `reserved` payout for a reserved claim. Only the claimant may
    /// do this. Repeating the request returns the existing payout
    /// (`duplicate = true`) as long as it is for the same address; a request
    /// for a different address is a conflict. Nothing is sent here.
    pub fn request_payout(
        &mut self,
        actor: &str,
        terms: &PayoutTerms<'_>,
        fingerprint: &dyn Fn(&str) -> String,
        org_allowed: &dyn Fn(&str) -> bool,
    ) -> Result<PayoutOutcome> {
        validate_id("claim", terms.claim_id)?;
        if !(0..=MAX_MONEY_ZATS).contains(&terms.fee_estimate_zats)
            || !(terms.fee_estimate_zats..=MAX_MONEY_ZATS).contains(&terms.max_fee_zats)
            || terms.lease_secs <= 0
            || terms.lease_owner.is_empty()
        {
            return Err(PayrollError::Validation("invalid payout terms".into()));
        }
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        // Locking the claim row serializes concurrent requests for it.
        let (c, s) = load_claim_for_payout(&mut tx, actor, terms.claim_id, true)?;
        if let Some(existing) = tx.payout_by_claim(terms.claim_id, true)? {
            return if fingerprint(&existing.id) == existing.recipient_fingerprint {
                Ok(PayoutOutcome {
                    payout: existing,
                    duplicate: true,
                })
            } else {
                Err(PayrollError::Conflict(
                    "a payout to a different address was already requested for this claim".into(),
                ))
            };
        }
        check_payable(&c, &s, org_allowed)?;
        let id = new_id();
        let p = Payout {
            recipient_fingerprint: fingerprint(&id),
            id,
            claim_id: c.id.clone(),
            stream_id: c.stream_id.clone(),
            requested_by: actor.to_string(),
            amount_zats: c.amount_zats,
            fee_estimate_zats: terms.fee_estimate_zats,
            max_fee_zats: terms.max_fee_zats,
            state: PayoutState::Reserved,
            txid: None,
            fee_zats: None,
            expiry_height: None,
            mined_height: None,
            confirmations: 0,
            detail: None,
            created_at: now,
            updated_at: now,
            submitted_at: None,
            resolved_at: None,
            lease_owner: Some(terms.lease_owner.to_string()),
            lease_until: Some(now + terms.lease_secs),
        };
        tx.insert_payout(&p)?;
        tx.insert_payout_event(&p.id, None, PayoutState::Reserved, now, None)?;
        tx.commit()?;
        Ok(PayoutOutcome {
            payout: p,
            duplicate: false,
        })
    }

    /// The payout of a claim, if any, for anyone who may view the claim's stream.
    pub fn claim_payout(&mut self, actor: &str, claim_id: &str) -> Result<Option<Payout>> {
        validate_id("claim", claim_id)?;
        let mut tx = self.backend.begin(false)?;
        let c = tx
            .load_claim(claim_id, false)?
            .ok_or(PayrollError::NotFound("claim"))?;
        let s = load_stream(&mut tx, &c.stream_id, false)?;
        require_can_view(&mut tx, actor, &s).map_err(|_| PayrollError::NotFound("claim"))?;
        tx.payout_by_claim(claim_id, false)
    }

    /// Succeeds only for an owner/admin of `org` (e.g. to show the payout
    /// wallet's balance and funding address).
    pub fn require_payroll_manager(&mut self, actor: &str, org: &str) -> Result<()> {
        validate_id("organization", org)?;
        let mut tx = self.backend.begin(false)?;
        crate::engine::require_manager(&mut tx, actor, org).map(|_| ())
    }

    /// Payouts of a stream, for anyone who may view the stream.
    pub fn list_payouts(&mut self, actor: &str, stream_id: &str) -> Result<Vec<Payout>> {
        let mut tx = self.backend.begin(false)?;
        let s = load_stream(&mut tx, stream_id, false)?;
        require_can_view(&mut tx, actor, &s)?;
        tx.payouts_of_stream(stream_id)
    }

    /// One payout and its state history, for anyone who may view its stream.
    pub fn payout_history(
        &mut self,
        actor: &str,
        payout_id: &str,
    ) -> Result<(Payout, Vec<PayoutEvent>)> {
        validate_id("payout", payout_id)?;
        let mut tx = self.backend.begin(false)?;
        let p = tx
            .load_payout(payout_id, false)?
            .ok_or(PayrollError::NotFound("payout"))?;
        let s = load_stream(&mut tx, &p.stream_id, false)?;
        require_can_view(&mut tx, actor, &s).map_err(|_| PayrollError::NotFound("payout"))?;
        let events = tx.payout_events_of(payout_id)?;
        Ok((p, events))
    }

    // ---- service-internal (payout worker); never exposed to users ----------

    /// Loads a payout without an authorization check. Worker use only.
    pub fn payout_internal(&mut self, payout_id: &str) -> Result<Payout> {
        let mut tx = self.backend.begin(false)?;
        tx.load_payout(payout_id, false)?
            .ok_or(PayrollError::NotFound("payout"))
    }

    /// Leases up to `limit` unfinished payouts whose lease is free or expired.
    pub fn lease_due_payouts(
        &mut self,
        owner: &str,
        lease_secs: i64,
        limit: i64,
    ) -> Result<Vec<Payout>> {
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        let v = tx.lease_due_payouts(owner, now, now + lease_secs, limit)?;
        tx.commit()?;
        Ok(v)
    }

    /// Applies `d` to a payout this `owner` has leased in `expected` state,
    /// releasing the lease. Confirmation settles the claim and failure releases
    /// it, in the same transaction. Returns `Conflict` if the payout moved on
    /// or the lease was lost in the meantime.
    pub fn apply_payout_decision(
        &mut self,
        payout_id: &str,
        owner: &str,
        expected: PayoutState,
        d: &Decision,
    ) -> Result<Payout> {
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        let p = tx
            .load_payout(payout_id, true)?
            .ok_or(PayrollError::NotFound("payout"))?;
        if p.state != expected || p.lease_owner.as_deref() != Some(owner) {
            return Err(PayrollError::Conflict("payout changed concurrently".into()));
        }
        let mut n = p.clone();
        n.lease_owner = None;
        n.lease_until = None;
        n.updated_at = now.max(p.updated_at);
        let record = |obs: &TxObservation, n: &mut Payout| -> Result<()> {
            if n.txid.as_ref().is_some_and(|t| *t != obs.txid) {
                return Err(PayrollError::InvalidState(
                    "a payout's transaction id can never change".into(),
                ));
            }
            if obs.txid.len() != 64 || !obs.txid.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(PayrollError::Validation("malformed txid".into()));
            }
            n.txid = Some(obs.txid.clone());
            n.fee_zats = obs.fee_zats.or(n.fee_zats);
            n.expiry_height = obs.expiry_height.or(n.expiry_height);
            n.mined_height = obs.mined_height;
            n.confirmations = obs.confirmations.max(0);
            Ok(())
        };
        let to = match d {
            Decision::Wait => p.state,
            Decision::Submitted(obs) => {
                record(obs, &mut n)?;
                n.detail = None;
                PayoutState::Submitted
            }
            Decision::Confirmed(obs) => {
                record(obs, &mut n)?;
                if n.mined_height.is_none() || n.confirmations < 1 {
                    return Err(PayrollError::InvalidState(
                        "cannot confirm an unmined transaction".into(),
                    ));
                }
                n.detail = None;
                PayoutState::Confirmed
            }
            Decision::Failed { reason } => {
                n.detail = Some(truncate(reason));
                PayoutState::Failed
            }
            Decision::NeedsReconciliation { reason, tx } => {
                if let Some(obs) = tx {
                    record(obs, &mut n)?;
                }
                n.detail = Some(truncate(reason));
                PayoutState::RequiresReconciliation
            }
        };
        if to != p.state && !p.state.can_become(to) {
            return Err(PayrollError::InvalidState(format!(
                "payout cannot move from {} to {}",
                p.state.as_str(),
                to.as_str()
            )));
        }
        n.state = to;
        if to == PayoutState::Submitted && n.submitted_at.is_none() {
            n.submitted_at = Some(now);
        }
        if to.is_final() {
            n.resolved_at = Some(now);
        }
        if tx.update_payout_cas(&n, p.state, Some(owner))? != 1 {
            return Err(PayrollError::Conflict("payout changed concurrently".into()));
        }
        if to != p.state {
            tx.insert_payout_event(&n.id, Some(p.state), to, now, n.detail.as_deref())?;
        }
        let claim_update = match to {
            PayoutState::Confirmed if p.state != to => {
                Some((ClaimStatus::Settled, n.txid.as_deref()))
            }
            PayoutState::Failed if p.state != to => Some((ClaimStatus::Released, None)),
            _ => None,
        };
        if let Some((status, settlement_ref)) = claim_update
            && tx.update_claim_cas(&n.claim_id, status, now, &n.requested_by, settlement_ref)? != 1
        {
            return Err(PayrollError::InvalidState(
                "claim was not reserved while its payout resolved".into(),
            ));
        }
        tx.commit()?;
        Ok(n)
    }
}

fn load_claim_for_payout<T: StoreTx>(
    tx: &mut T,
    actor: &str,
    claim_id: &str,
    lock: bool,
) -> Result<(Claim, Stream)> {
    let c = tx
        .load_claim(claim_id, lock)?
        .ok_or(PayrollError::NotFound("claim"))?;
    let s = load_stream(tx, &c.stream_id, false)?;
    require_can_view(tx, actor, &s).map_err(|_| PayrollError::NotFound("claim"))?;
    if actor != c.requested_by || actor != s.recipient_user_id {
        return Err(PayrollError::Forbidden(
            "only the claimant can request a payout",
        ));
    }
    Ok((c, s))
}

fn check_payable(c: &Claim, s: &Stream, org_allowed: &dyn Fn(&str) -> bool) -> Result<()> {
    if !org_allowed(&s.organization_id) {
        return Err(PayrollError::Forbidden(
            "payouts are not enabled for this organization",
        ));
    }
    if c.status != ClaimStatus::Reserved {
        return Err(PayrollError::InvalidState(
            "only a reserved claim can be paid out".into(),
        ));
    }
    Ok(())
}

// ---- worker steps ------------------------------------------------------------------

/// Submits a freshly reserved payout (leased by the caller) to the wallet and
/// records the outcome. Any doubt about the outcome leaves the payout in
/// `requires_reconciliation`; it is never resubmitted under a new id.
pub fn submit_payout<B: Backend>(
    db: &mut Payroll<B>,
    wallet: &dyn WalletApi,
    policy: &PayoutPolicy,
    p: &Payout,
    address: &str,
) -> Result<Payout> {
    let owner = p
        .lease_owner
        .clone()
        .ok_or_else(|| PayrollError::InvalidState("payout is not leased".into()))?;
    if p.state != PayoutState::Reserved {
        return Ok(p.clone());
    }
    let req = WalletSubmit {
        payment_id: p.id.clone(),
        address: address.trim().to_string(),
        amount_zats: p.amount_zats,
        max_fee_zats: p.max_fee_zats,
    };
    let d = match wallet.submit(&req) {
        Ok(w) => decide(p, &w, policy),
        Err(e) => reconcile(
            format!(
                "payout submission outcome unknown ({e}); the reconciler will check the wallet"
            ),
            None,
        ),
    };
    match db.apply_payout_decision(&p.id, &owner, PayoutState::Reserved, &d) {
        // Someone else (the reconciler) already moved it on: report that.
        Err(PayrollError::Conflict(_)) => db.payout_internal(&p.id),
        other => other,
    }
}

/// What the worker should do with a payout it leased.
pub fn reconcile_decision(p: &Payout, wallet: &dyn WalletApi, policy: &PayoutPolicy) -> Decision {
    match p.state {
        // A reserved payout whose lease expired: its submission was interrupted
        // (crash, restart, timeout). Fence the id at the wallet first so a
        // late submit can't pay it, then follow whatever the wallet reports.
        PayoutState::Reserved => match wallet.cancel(&p.id) {
            Ok(w) => decide(p, &w, policy),
            Err(e) => reconcile(
                format!("submission was interrupted and the wallet could not be reached ({e})"),
                None,
            ),
        },
        PayoutState::Submitted | PayoutState::RequiresReconciliation => {
            match wallet.lookup(&p.id) {
                Err(_) => Decision::Wait,
                Ok(Some(w)) => decide(p, &w, policy),
                Ok(None) if p.txid.is_some() => reconcile(
                    "the wallet has no record of this payment's transaction",
                    None,
                ),
                // The request never reached the wallet: fence it, then decide.
                Ok(None) => match wallet.cancel(&p.id) {
                    Ok(w) => decide(p, &w, policy),
                    Err(_) => Decision::Wait,
                },
            }
        }
        PayoutState::Confirmed | PayoutState::Failed => Decision::Wait,
    }
}

/// One worker pass: lease due payouts and move each forward. Returns the
/// payouts it updated.
pub fn reconcile_due<B: Backend>(
    db: &mut Payroll<B>,
    wallet: &dyn WalletApi,
    policy: &PayoutPolicy,
    owner: &str,
    limit: i64,
) -> Result<Vec<Payout>> {
    let leased = db.lease_due_payouts(owner, policy.worker_lease_secs, limit)?;
    let mut out = Vec::with_capacity(leased.len());
    for p in leased {
        let d = reconcile_decision(&p, wallet, policy);
        match db.apply_payout_decision(&p.id, owner, p.state, &d) {
            Ok(n) => out.push(n),
            Err(PayrollError::Conflict(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_salted_per_payment_and_stable() {
        let a = recipient_fingerprint("id-1", "utest1abc");
        assert_eq!(a.len(), 64);
        assert_eq!(a, recipient_fingerprint("id-1", " utest1abc "));
        assert_ne!(a, recipient_fingerprint("id-2", "utest1abc"));
        assert_ne!(a, recipient_fingerprint("id-1", "utest1abd"));
        // Shared test vector with the wallet service (payout-wallet/src/intents.rs).
        assert_eq!(
            recipient_fingerprint("00000000-0000-0000-0000-000000000000", "utest1example"),
            "9d5ce522bd2347c13521b7a002f9b114cd4ee5bdeb9321608dd9f038109e950f"
        );
    }

    #[test]
    fn max_fee_respects_cap() {
        let p = PayoutPolicy::default();
        assert_eq!(p.max_fee_for(10_000), Some(20_000));
        assert_eq!(p.max_fee_for(95_000), Some(100_000));
        assert_eq!(p.max_fee_for(100_001), None);
        assert_eq!(p.max_fee_for(-1), None);
    }
}

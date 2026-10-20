//! The Phase 5 product flow on top of the engine.
//!
//! * An employer creates a [`StreamOffer`]: a worker's display name and the
//!   stream's terms (total, start, end). The worker link carries a random
//!   token; only its SHA-256 (`link_hash`) reaches the engine.
//! * [`Payroll::bind_offer`]: a signed-in worker opens the link. In one
//!   transaction they join the organization, the payroll stream is created
//!   with the offer's terms, and the offer is marked bound (single use).
//!   The stream pays exactly `total_zats` over `end_at - start_at`:
//!   `rate_zats = total_zats` and `rate_period_secs = end_at - start_at`, so
//!   the existing accrual (`floor(total × active / duration)`) reaches the
//!   total exactly at the end date. Earnings count from the offer's start
//!   date, even if the worker binds later (the employer agreed to those terms).
//! * [`Payroll::set_payout_address`]: the worker sets the stream's payout
//!   address once; it is then locked for the life of the stream.
//! * [`Payroll::withdraw`]: one action that reserves a claim for the whole
//!   withdrawal and records its `reserved` payout, in one transaction, under
//!   the stream's row lock. It is exactly-once by construction: a repeat with
//!   the same key, or any request while an earlier withdrawal is unfinished,
//!   returns that earlier payout. Submission and confirmation are the
//!   unchanged Phase 3 machinery ([`crate::payout`]).
//!
//! Withdrawal rule: the amount must be at least 20% of the stream's total
//! ([`withdrawal_unlocked`]); once the stream has ended, any remainder may be
//! withdrawn.

use crate::engine::{
    MAX_RATE_PERIOD_SECS, Payroll, balance_in, listings, load_stream, new_id, record_event,
    require_can_view, require_manager, validate_id, validate_text,
};
use crate::error::{PayrollError, Result};
use crate::model::*;
use crate::money::MAX_MONEY_ZATS;
use crate::store::{Backend, StoreTx};

/// Withdrawals unlock at each `1 / MILESTONES` of the total (20%).
pub const MILESTONES: i64 = 5;

/// Whether `amount` may be withdrawn from a stream paying `total`: at least
/// one milestone (20% of the total), or anything once the stream has ended.
pub fn withdrawal_unlocked(total: i64, amount: i64, ended: bool) -> bool {
    amount > 0 && (ended || (amount as i128) * (MILESTONES as i128) >= total as i128)
}

/// Request to create a stream offer. Times are unix seconds.
#[derive(Clone, Debug)]
pub struct NewOffer {
    pub organization_id: String,
    pub worker_name: String,
    pub total_zats: i64,
    /// Defaults to now. May not be in the past.
    pub start_at: Option<i64>,
    pub end_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindOutcome {
    pub offer: StreamOffer,
    pub stream: Stream,
    /// True when the actor had already bound this offer.
    pub duplicate: bool,
}

/// A fixed-total stream with everything its home screens show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowStream {
    pub listing: StreamListing,
    pub offer: StreamOffer,
    /// Present for the stream's recipient only; managers see `address_locked`.
    pub address: Option<PayoutAddress>,
    pub address_locked: bool,
    pub payouts: Vec<Payout>,
}

impl FlowStream {
    /// The unfinished withdrawal, if any (at most one exists at a time).
    pub fn open_payout(&self) -> Option<&Payout> {
        self.payouts.iter().find(|p| !p.state.is_final())
    }
    pub fn ended(&self) -> bool {
        self.listing.balance.status == StreamStatus::Ended
    }
    /// What a withdrawal right now would pay, if one is allowed.
    pub fn withdrawable_now(&self) -> Option<i64> {
        let a = self.listing.balance.available;
        (self.address_locked
            && self.open_payout().is_none()
            && withdrawal_unlocked(self.offer.total_zats, a, self.ended()))
        .then_some(a)
    }
}

/// Terms of one withdrawal, prepared by the caller (payroll-api) from a
/// wallet quote for exactly `amount_zats`.
pub struct WithdrawTerms<'a> {
    pub stream_id: &'a str,
    pub idempotency_key: &'a str,
    pub amount_zats: i64,
    pub fee_estimate_zats: i64,
    pub max_fee_zats: i64,
    /// `recipient_fingerprint(stream_id, address)` of the address the caller
    /// will pay; must equal the stream's locked fingerprint.
    pub address_fingerprint: &'a str,
    /// Lease taken by the requesting process while it submits to the wallet.
    pub lease_owner: &'a str,
    pub lease_secs: i64,
}

fn validate_link_hash(h: &str) -> Result<()> {
    if h.len() == 64
        && h.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        // Malformed links can't exist; report them like any unknown link.
        Err(PayrollError::NotFound("worker link"))
    }
}

fn load_offer<T: StoreTx>(tx: &mut T, id: &str, lock: bool) -> Result<StreamOffer> {
    validate_id("stream offer", id)?;
    tx.load_offer(id, lock)?
        .ok_or(PayrollError::NotFound("stream offer"))
}

fn flow_stream<T: StoreTx>(
    tx: &mut T,
    actor: &str,
    listing: StreamListing,
) -> Result<Option<FlowStream>> {
    let Some(offer) = tx.offer_by_stream(&listing.stream.id)? else {
        return Ok(None);
    };
    let addr = tx.payout_address(&listing.stream.id)?;
    let is_recipient = actor == listing.stream.recipient_user_id;
    let payouts = tx.payouts_of_stream(&listing.stream.id)?;
    Ok(Some(FlowStream {
        listing,
        offer,
        address_locked: addr.is_some(),
        address: addr.filter(|_| is_recipient),
        payouts,
    }))
}

impl<B: Backend> Payroll<B> {
    // ---- employer -------------------------------------------------------------

    /// Creates an offer for a worker who doesn't need to exist yet. Owners
    /// and admins only.
    pub fn create_offer(
        &mut self,
        actor: &str,
        req: NewOffer,
        link_hash: &str,
    ) -> Result<StreamOffer> {
        validate_id("organization", &req.organization_id)?;
        validate_text("worker_name", &req.worker_name, 200)?;
        validate_link_hash(link_hash)
            .map_err(|_| PayrollError::Validation("invalid link hash".into()))?;
        let now = self.now();
        let start_at = req.start_at.unwrap_or(now);
        if !(1..=MAX_MONEY_ZATS).contains(&req.total_zats) {
            return Err(PayrollError::Validation(
                "total must be positive and at most 21,000,000 ZEC".into(),
            ));
        }
        if start_at < now {
            return Err(PayrollError::Validation(
                "start may not be in the past".into(),
            ));
        }
        if req.end_at <= start_at {
            return Err(PayrollError::Validation(
                "end date must be after the start".into(),
            ));
        }
        if req.end_at - start_at > MAX_RATE_PERIOD_SECS {
            return Err(PayrollError::Validation(
                "a stream may last at most 100 years".into(),
            ));
        }
        let mut tx = self.backend.begin(true)?;
        require_manager(&mut tx, actor, &req.organization_id)?;
        let id = new_id();
        let offer = StreamOffer {
            id: id.clone(),
            organization_id: req.organization_id,
            organization_name: String::new(),
            created_by: actor.to_string(),
            worker_name: req.worker_name,
            total_zats: req.total_zats,
            start_at,
            end_at: req.end_at,
            created_at: now,
            stream_id: None,
            bound_user_id: None,
            bound_at: None,
            cancelled_at: None,
        };
        tx.insert_offer(&offer, link_hash)?;
        let offer = load_offer(&mut tx, &id, false)?;
        tx.commit()?;
        Ok(offer)
    }

    /// Cancels an offer nobody has bound yet. Nothing is owed on it.
    pub fn cancel_offer(&mut self, actor: &str, offer_id: &str) -> Result<StreamOffer> {
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        let o = load_offer(&mut tx, offer_id, true)?;
        require_manager(&mut tx, actor, &o.organization_id)?;
        if tx.cancel_offer_cas(offer_id, now)? != 1 {
            return Err(PayrollError::InvalidState(match o.status() {
                OfferStatus::Bound => {
                    "the worker has already joined; end the stream instead".into()
                }
                _ => "this stream was already cancelled".into(),
            }));
        }
        let o = load_offer(&mut tx, offer_id, false)?;
        tx.commit()?;
        Ok(o)
    }

    /// Replaces the worker link of an open offer (the old link stops working).
    pub fn relink_offer(
        &mut self,
        actor: &str,
        offer_id: &str,
        link_hash: &str,
    ) -> Result<StreamOffer> {
        validate_link_hash(link_hash)
            .map_err(|_| PayrollError::Validation("invalid link hash".into()))?;
        let mut tx = self.backend.begin(true)?;
        let o = load_offer(&mut tx, offer_id, true)?;
        require_manager(&mut tx, actor, &o.organization_id)?;
        if tx.relink_offer_cas(offer_id, link_hash)? != 1 {
            return Err(PayrollError::InvalidState(
                "the worker link can only be replaced before the worker joins".into(),
            ));
        }
        tx.commit()?;
        Ok(o)
    }

    /// Every offer of an organization, newest first. Owners/admins only.
    pub fn list_offers(&mut self, actor: &str, org: &str) -> Result<Vec<StreamOffer>> {
        validate_id("organization", org)?;
        let mut tx = self.backend.begin(false)?;
        require_manager(&mut tx, actor, org)?;
        tx.offers_of_org(org)
    }

    /// The organization's fixed-total streams. Owners/admins only.
    pub fn employer_streams(&mut self, actor: &str, org: &str) -> Result<Vec<FlowStream>> {
        validate_id("organization", org)?;
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        require_manager(&mut tx, actor, org)?;
        let rows = listings(tx.list_streams(StreamFilter::Organization(org))?, now)?;
        let mut out = vec![];
        for l in rows {
            out.extend(flow_stream(&mut tx, actor, l)?);
        }
        Ok(out)
    }

    // ---- worker link ----------------------------------------------------------

    /// What a worker link offers, for the preview shown before sign-in. The
    /// caller must show only the terms (company, total, dates), never who
    /// bound it.
    pub fn offer_preview(&mut self, link_hash: &str) -> Result<StreamOffer> {
        validate_link_hash(link_hash)?;
        let mut tx = self.backend.begin(false)?;
        tx.offer_by_link(link_hash, false)?
            .ok_or(PayrollError::NotFound("worker link"))
    }

    /// Binds the offer behind `link_hash` to `actor`, creating the stream.
    /// Repeating it as the same worker returns the stream (`duplicate`).
    pub fn bind_offer(&mut self, actor: &str, link_hash: &str) -> Result<BindOutcome> {
        validate_link_hash(link_hash)?;
        validate_id("user", actor)?;
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        // Locking the offer row serializes concurrent binds of one link.
        let o = tx
            .offer_by_link(link_hash, true)?
            .ok_or(PayrollError::NotFound("worker link"))?;
        match (
            o.status(),
            o.bound_user_id.as_deref(),
            o.stream_id.as_deref(),
        ) {
            (OfferStatus::Bound, Some(u), Some(sid)) if u == actor => {
                let stream = load_stream(&mut tx, sid, false)?;
                return Ok(BindOutcome {
                    offer: o,
                    stream,
                    duplicate: true,
                });
            }
            (OfferStatus::Bound, ..) => {
                return Err(PayrollError::Conflict(
                    "this worker link has already been used".into(),
                ));
            }
            (OfferStatus::Cancelled, ..) => {
                return Err(PayrollError::InvalidState(
                    "this stream was cancelled by the employer".into(),
                ));
            }
            (OfferStatus::Open, ..) => {}
        }
        if !tx.user_exists(actor)? {
            return Err(PayrollError::NotFound("user"));
        }
        match tx.role_of(&o.organization_id, actor)? {
            Some(r) if r.manages_payroll() => {
                return Err(PayrollError::Forbidden(
                    "an employer cannot accept their own worker link",
                ));
            }
            Some(_) => {}
            None => tx.insert_member(&o.organization_id, actor, Role::Member, now)?,
        }
        let req = NewStream {
            organization_id: o.organization_id.clone(),
            recipient_user_id: actor.to_string(),
            rate_zats: o.total_zats,
            rate_period_secs: o.end_at - o.start_at,
            start_at: Some(o.start_at),
            end_at: Some(o.end_at),
        };
        let stream_id = new_id();
        tx.insert_stream(&stream_id, &o.created_by, &req, o.start_at, now)?;
        if tx.bind_offer_cas(&o.id, &stream_id, actor, now)? != 1 {
            return Err(PayrollError::Conflict(
                "this worker link has already been used".into(),
            ));
        }
        let stream = load_stream(&mut tx, &stream_id, false)?;
        record_event(&mut tx, &stream, "created", actor, now)?;
        let offer = load_offer(&mut tx, &o.id, false)?;
        tx.commit()?;
        Ok(BindOutcome {
            offer,
            stream,
            duplicate: false,
        })
    }

    // ---- worker -----------------------------------------------------------------

    /// The actor's fixed-total streams, newest first.
    pub fn worker_streams(&mut self, actor: &str) -> Result<Vec<FlowStream>> {
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        let rows = listings(tx.list_streams(StreamFilter::Recipient(actor))?, now)?;
        let mut out = vec![];
        for l in rows {
            out.extend(flow_stream(&mut tx, actor, l)?);
        }
        Ok(out)
    }

    /// One fixed-total stream, for its recipient or the paying organization's
    /// owners/admins.
    pub fn flow_stream(&mut self, actor: &str, stream_id: &str) -> Result<FlowStream> {
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        let s = load_stream(&mut tx, stream_id, false)?;
        require_can_view(&mut tx, actor, &s)?;
        let row = tx
            .list_streams(StreamFilter::Organization(&s.organization_id))?
            .into_iter()
            .find(|r| r.stream.id == stream_id)
            .ok_or(PayrollError::NotFound("stream"))?;
        let listing = listings(vec![row], now)?.remove(0);
        flow_stream(&mut tx, actor, listing)?.ok_or(PayrollError::NotFound("stream"))
    }

    /// Sets the stream's payout address. Recipient only, once: afterwards it
    /// is locked for the life of the stream. Repeating it with the same
    /// address (same fingerprint) returns the lock (`true` = duplicate).
    pub fn set_payout_address(
        &mut self,
        actor: &str,
        stream_id: &str,
        address_fingerprint: &str,
        address_sealed: &str,
    ) -> Result<(PayoutAddress, bool)> {
        if address_fingerprint.len() != 64
            || address_sealed.is_empty()
            || address_sealed.len() > 4000
        {
            return Err(PayrollError::Validation("invalid payout address".into()));
        }
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        let s = load_stream(&mut tx, stream_id, true)?;
        require_can_view(&mut tx, actor, &s)?;
        if actor != s.recipient_user_id {
            return Err(PayrollError::Forbidden(
                "only the worker can set the payout address",
            ));
        }
        if tx.offer_by_stream(stream_id)?.is_none() {
            return Err(PayrollError::InvalidState(
                "this stream has no payout address".into(),
            ));
        }
        if let Some(a) = tx.payout_address(stream_id)? {
            return if a.address_fingerprint == address_fingerprint {
                Ok((a, true))
            } else {
                Err(PayrollError::Conflict(
                    "the payout address for this stream is locked".into(),
                ))
            };
        }
        let a = PayoutAddress {
            stream_id: stream_id.to_string(),
            set_by: actor.to_string(),
            address_fingerprint: address_fingerprint.to_string(),
            address_sealed: address_sealed.to_string(),
            set_at: now,
        };
        tx.insert_payout_address(&a)?;
        tx.commit()?;
        Ok((a, false))
    }

    /// Withdraws `terms.amount_zats` to the stream's locked address: reserves
    /// a claim and records its `reserved` payout (leased to the caller, who
    /// then submits it with [`crate::payout::submit_payout`]), atomically.
    ///
    /// Exactly-once: the same `idempotency_key`, or any request while a
    /// withdrawal of this stream is unfinished, returns that payout
    /// (`duplicate = true`) instead of creating another.
    pub fn withdraw(
        &mut self,
        actor: &str,
        terms: &WithdrawTerms<'_>,
        payout_fingerprint: &dyn Fn(&str) -> String,
        org_allowed: &dyn Fn(&str) -> bool,
    ) -> Result<PayoutOutcome> {
        let key = terms.idempotency_key;
        if !(1..=128).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(PayrollError::Validation(
                "idempotency_key must be 1..=128 printable ASCII characters".into(),
            ));
        }
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        // The stream row lock serializes every withdrawal of this stream.
        let s = load_stream(&mut tx, terms.stream_id, true)?;
        require_can_view(&mut tx, actor, &s)?;
        if actor != s.recipient_user_id {
            return Err(PayrollError::Forbidden("only the worker can withdraw"));
        }

        if let Some(c) = tx.claim_by_key(&s.id, key)? {
            return match tx.payout_by_claim(&c.id, true)? {
                Some(p) if p.requested_by == actor => Ok(PayoutOutcome {
                    payout: p,
                    duplicate: true,
                }),
                _ => Err(PayrollError::IdempotencyConflict),
            };
        }
        if let Some(p) = tx
            .payouts_of_stream(&s.id)?
            .into_iter()
            .find(|p| !p.state.is_final())
        {
            return Ok(PayoutOutcome {
                payout: p,
                duplicate: true,
            });
        }

        // Checked only now: a repeat must get its payout back even when its
        // terms (e.g. "everything available" = 0 while reserved) are void.
        if terms.amount_zats == 0 {
            return Err(PayrollError::InvalidState(
                "there is nothing to withdraw yet".into(),
            ));
        }
        if !(1..=MAX_MONEY_ZATS).contains(&terms.amount_zats)
            || !(0..=MAX_MONEY_ZATS).contains(&terms.fee_estimate_zats)
            || !(terms.fee_estimate_zats..=MAX_MONEY_ZATS).contains(&terms.max_fee_zats)
            || terms.lease_secs <= 0
            || terms.lease_owner.is_empty()
        {
            return Err(PayrollError::Validation("invalid withdrawal terms".into()));
        }
        let offer = tx
            .offer_by_stream(&s.id)?
            .ok_or_else(|| PayrollError::InvalidState("this stream has no withdrawals".into()))?;
        let addr = tx.payout_address(&s.id)?.ok_or_else(|| {
            PayrollError::InvalidState("set your payout address before withdrawing".into())
        })?;
        if addr.address_fingerprint != terms.address_fingerprint {
            return Err(PayrollError::InvalidState(
                "the payout address does not match this stream's locked address".into(),
            ));
        }
        if !org_allowed(&s.organization_id) {
            return Err(PayrollError::Forbidden(
                "payouts are not enabled for this organization",
            ));
        }
        let bal = balance_in(&mut tx, &s, now)?;
        if terms.amount_zats > bal.available {
            return Err(PayrollError::InsufficientAvailable {
                requested: terms.amount_zats,
                available: bal.available,
            });
        }
        let ended = s.effective_status_at(now) == StreamStatus::Ended;
        if !withdrawal_unlocked(offer.total_zats, terms.amount_zats, ended) {
            return Err(PayrollError::InvalidState(
                "withdrawals unlock at each 20% of the stream's total (or after it ends)".into(),
            ));
        }

        let claim_id = new_id();
        tx.insert_claim(
            &claim_id,
            &s.id,
            actor,
            key,
            terms.amount_zats,
            bal.earned,
            now,
        )?;
        let id = new_id();
        let p = Payout {
            recipient_fingerprint: payout_fingerprint(&id),
            id,
            compensation_type: CompensationType::Stream,
            claim_id,
            source_id: s.id.clone(),
            requested_by: actor.to_string(),
            amount_zats: terms.amount_zats,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestones_are_twenty_percent_of_the_total() {
        // total 1_000: one milestone = 200.
        assert!(!withdrawal_unlocked(1_000, 0, false));
        assert!(!withdrawal_unlocked(1_000, 199, false));
        assert!(withdrawal_unlocked(1_000, 200, false));
        assert!(withdrawal_unlocked(1_000, 450, false));
        // A total not divisible by 5: 20% of 1_001 is 200.2, so 201 is needed.
        assert!(!withdrawal_unlocked(1_001, 200, false));
        assert!(withdrawal_unlocked(1_001, 201, false));
    }

    #[test]
    fn after_the_end_any_remainder_unlocks() {
        assert!(withdrawal_unlocked(1_000, 1, true));
        assert!(!withdrawal_unlocked(1_000, 0, true));
    }

    #[test]
    fn no_overflow_at_the_money_supply() {
        assert!(withdrawal_unlocked(MAX_MONEY_ZATS, MAX_MONEY_ZATS, false));
        assert!(!withdrawal_unlocked(
            MAX_MONEY_ZATS,
            MAX_MONEY_ZATS / 5 - 1,
            false
        ));
    }
}

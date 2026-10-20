//! Vesting compensation: a fixed ZEC allocation that vests to one employee
//! over time. A second compensation type next to payroll streams; it shares
//! organizations, membership, the claim-reservation design and the shielded
//! payout state machine ([`crate::payout`]), but has its own agreement and
//! claim tables and never touches streams or stream claims.
//!
//! # Vesting rule (method `linear`)
//!
//! Integer zatoshis only, evaluated at the trusted server clock `t`:
//!
//! ```text
//! elapsed    = t - start_at
//! vested(t)  = 0                                       if elapsed <= 0 or elapsed < cliff_secs
//!            = total                                   if elapsed >= duration_secs
//!            = floor(total * elapsed / duration_secs)  otherwise   (i128, no overflow)
//! ```
//!
//! * **Cliff:** nothing vests before `start_at + cliff_secs`. Exactly at the
//!   cliff, everything the linear schedule would have vested by then vests at
//!   once (`floor(total * cliff / duration)`); after it, vesting is linear.
//!   `cliff_secs = 0` means no cliff; `cliff_secs = duration_secs` vests
//!   everything at the end.
//! * **Rounding:** the floor is taken once over the whole elapsed time, so the
//!   amount is monotonically non-decreasing, never exceeds `total`, and is at
//!   most 1 zatoshi short of the exact rational value until the end.
//! * **End:** at and after `start_at + duration_secs` the vested amount is
//!   exactly `total` (an explicit branch, not a consequence of rounding).
//!
//! # Lifecycle
//!
//! * [`Payroll::create_vesting`] (owner/admin) creates a `draft`. A draft vests
//!   nothing and is invisible to the employee.
//! * [`Payroll::activate_vesting`] (owner/admin) makes it `active`, at or
//!   before its `start_at`. Activation after the start is refused (engine and
//!   schema): it would grant everything due since the start at once. A draft
//!   whose start has passed can only be cancelled and created again.
//! * [`Payroll::cancel_vesting`] (owner/admin): see the cancellation policy.
//!
//! # Cancellation policy
//!
//! * Cancelling an `active` agreement freezes the vested amount at the
//!   cancellation time (`vested_at_cancel_zats`, stored). Everything vested
//!   until then stays owed and claimable; the rest is forfeited and can never
//!   become claimable. Cancelling during the cliff (or before the start)
//!   therefore freezes it at 0.
//! * Cancelling a `draft` voids it: nothing was ever owed (frozen at 0).
//! * A fully vested agreement cannot be cancelled: there is nothing left to stop.
//! * Claims are never touched by cancellation. A reserved claim was reserved
//!   against vested compensation, which stays vested, so it can still be
//!   settled (paid) or released; a settled claim stays settled. The schema
//!   refuses a cancellation that would freeze less than is already reserved
//!   or paid.
//! * `cancelled` is final.
//!
//! # Claims
//!
//! `available = vested - reserved - paid`. A claim reserves part of
//! `available` under the agreement's row lock; an idempotency key per
//! agreement makes retries return the original claim. `reserved → released`
//! (claimant or owner/admin) returns the amount to `available`;
//! `reserved → settled` records payment and is final.
//!
//! # Payouts
//!
//! [`Payroll::withdraw_vesting`] reserves a vesting claim and records its
//! `reserved` payout (`compensation_type = vesting`) in one transaction,
//! under the agreement's row lock, to the agreement's locked payout address.
//! Submission, confirmation, failure and reconciliation are the unchanged
//! payout state machine: only a verified, confirmed payout settles its claim,
//! and only a proven failure releases it. A claim with a payout can't be
//! released or settled any other way (engine and schema). The owner/admin
//! accounting settlement ([`Payroll::settle_vesting_claim`]) remains for
//! claims paid outside hushush that have no payout.

use crate::engine::{
    MAX_RATE_PERIOD_SECS, Payroll, new_id, require_manager, validate_id, validate_text,
};
use crate::error::{PayrollError, Result};
use crate::model::*;
use crate::money::MAX_MONEY_ZATS;
use crate::store::{Backend, StoreTx};

/// Vested zatoshis of `terms` at time `t` (see the module docs).
pub fn vested_amount(terms: &VestingTerms, t: i64) -> i64 {
    let VestingTerms {
        total_zats,
        start_at,
        duration_secs,
        cliff_secs,
        method: VestingMethod::Linear,
    } = *terms;
    let elapsed = t.saturating_sub(start_at);
    if elapsed <= 0 || elapsed < cliff_secs {
        return 0;
    }
    if elapsed >= duration_secs {
        return total_zats;
    }
    ((total_zats as i128 * elapsed as i128) / duration_secs as i128) as i64
}

impl VestingAgreement {
    pub fn end_at(&self) -> i64 {
        self.terms.start_at.saturating_add(self.terms.duration_secs)
    }

    pub fn cliff_at(&self) -> i64 {
        self.terms.start_at.saturating_add(self.terms.cliff_secs)
    }

    /// Vested zatoshis at `t`: 0 for a draft, the schedule while active, the
    /// frozen amount once cancelled.
    pub fn vested_at(&self, t: i64) -> i64 {
        match self.status {
            VestingStatus::Draft => 0,
            VestingStatus::Active => vested_amount(&self.terms, t),
            VestingStatus::Cancelled => self.vested_at_cancel_zats.unwrap_or(0),
        }
    }

    pub fn phase_at(&self, t: i64) -> VestingPhase {
        match self.status {
            VestingStatus::Draft => VestingPhase::Draft,
            VestingStatus::Cancelled => VestingPhase::Cancelled,
            VestingStatus::Active if t < self.terms.start_at => VestingPhase::NotStarted,
            VestingStatus::Active if t >= self.end_at() => VestingPhase::FullyVested,
            VestingStatus::Active if t < self.cliff_at() => VestingPhase::InCliff,
            VestingStatus::Active => VestingPhase::Vesting,
        }
    }

    /// Whether the employee may see it: only once it was activated.
    pub fn visible_to_employee(&self) -> bool {
        self.activated_at.is_some()
    }
}

/// Request to create a vesting agreement. Times are unix seconds.
#[derive(Clone, Debug)]
pub struct NewVesting {
    pub organization_id: String,
    pub employee_user_id: String,
    pub total_zats: i64,
    /// Defaults to now. May not be in the past.
    pub start_at: Option<i64>,
    pub duration_secs: i64,
    /// `None` (or 0) = no cliff.
    pub cliff_secs: Option<i64>,
    pub method: VestingMethod,
}

fn load_vesting<T: StoreTx>(tx: &mut T, id: &str, lock: bool) -> Result<VestingAgreement> {
    validate_id("vesting agreement", id)?;
    tx.load_vesting(id, lock)?
        .ok_or(PayrollError::NotFound("vesting agreement"))
}

/// The employee (once activated), or an owner/admin of the organization.
/// Anyone else is told the agreement does not exist.
fn require_can_view<T: StoreTx>(tx: &mut T, actor: &str, a: &VestingAgreement) -> Result<()> {
    if actor == a.employee_user_id && a.visible_to_employee() {
        return Ok(());
    }
    match tx.role_of(&a.organization_id, actor)? {
        Some(r) if r.manages_payroll() => Ok(()),
        _ => Err(PayrollError::NotFound("vesting agreement")),
    }
}

/// [`require_can_view`] for an agreement id (used by the payout module).
pub(crate) fn require_can_view_id<T: StoreTx>(tx: &mut T, actor: &str, id: &str) -> Result<()> {
    let a = load_vesting(tx, id, false)?;
    require_can_view(tx, actor, &a)
}

/// Terms of one vesting withdrawal, prepared by the caller (payroll-api) from
/// a wallet quote for exactly `amount_zats`.
pub struct VestingWithdrawTerms<'a> {
    pub agreement_id: &'a str,
    pub idempotency_key: &'a str,
    pub amount_zats: i64,
    pub fee_estimate_zats: i64,
    pub max_fee_zats: i64,
    /// `recipient_fingerprint(agreement_id, address)` of the address the
    /// caller will pay; must equal the agreement's locked fingerprint.
    pub address_fingerprint: &'a str,
    /// Lease taken by the requesting process while it submits to the wallet.
    pub lease_owner: &'a str,
    pub lease_secs: i64,
}

/// An agreement with everything its detail screen shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VestingDetail {
    pub listing: VestingListing,
    pub events: Vec<VestingEvent>,
    pub claims: Vec<VestingClaim>,
    /// Oldest first.
    pub payouts: Vec<Payout>,
    /// Present for the employee only; managers see `address_locked`.
    pub address: Option<VestingPayoutAddress>,
    pub address_locked: bool,
}

impl VestingDetail {
    /// The unfinished withdrawal, if any (at most one exists at a time).
    pub fn open_payout(&self) -> Option<&Payout> {
        self.payouts.iter().find(|p| !p.state.is_final())
    }
}

fn balance_from(
    a: &VestingAgreement,
    now: i64,
    reserved: i64,
    paid: i64,
) -> Result<VestingBalance> {
    let vested = a.vested_at(now);
    let available = vested - reserved - paid;
    if available < 0 {
        // Unreachable if the invariants hold; never paper over it.
        return Err(PayrollError::InvalidState(format!(
            "ledger invariant violated for vesting agreement {}: vested {vested} < reserved {reserved} + paid {paid}",
            a.id
        )));
    }
    Ok(VestingBalance {
        agreement_id: a.id.clone(),
        as_of: now,
        phase: a.phase_at(now),
        total: a.terms.total_zats,
        vested,
        unvested: a.terms.total_zats - vested,
        reserved,
        paid,
        available,
    })
}

fn balance_in<T: StoreTx>(tx: &mut T, a: &VestingAgreement, now: i64) -> Result<VestingBalance> {
    let (reserved, paid) = tx.vesting_claim_sums(&a.id)?;
    balance_from(a, now, reserved, paid)
}

fn listings(rows: Vec<VestingListedRow>, now: i64) -> Result<Vec<VestingListing>> {
    rows.into_iter()
        .map(|r| {
            let balance = balance_from(&r.agreement, now, r.reserved, r.paid)?;
            Ok(VestingListing {
                agreement: r.agreement,
                organization_name: r.organization_name,
                employee_display_name: r.employee_display_name,
                balance,
            })
        })
        .collect()
}

fn validate_key(key: &str) -> Result<()> {
    if !(1..=128).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(PayrollError::Validation(
            "idempotency_key must be 1..=128 printable ASCII characters".into(),
        ));
    }
    Ok(())
}

impl<B: Backend> Payroll<B> {
    // ---- employer -------------------------------------------------------------

    /// Creates a `draft` agreement for a member of the organization. Owners
    /// and admins only, and never for themselves.
    pub fn create_vesting(&mut self, actor: &str, req: NewVesting) -> Result<VestingAgreement> {
        validate_id("organization", &req.organization_id)?;
        let now = self.now();
        let start_at = req.start_at.unwrap_or(now);
        let cliff_secs = req.cliff_secs.unwrap_or(0);
        if !(1..=MAX_MONEY_ZATS).contains(&req.total_zats) {
            return Err(PayrollError::Validation(
                "total must be positive and at most 21,000,000 ZEC".into(),
            ));
        }
        if !(1..=MAX_RATE_PERIOD_SECS).contains(&req.duration_secs) {
            return Err(PayrollError::Validation(
                "vesting duration must be 1 second..=100 years".into(),
            ));
        }
        if !(0..=req.duration_secs).contains(&cliff_secs) {
            return Err(PayrollError::Validation(
                "cliff must be between 0 and the vesting duration".into(),
            ));
        }
        if start_at < now {
            return Err(PayrollError::Validation(
                "start may not be in the past".into(),
            ));
        }
        let mut tx = self.backend.begin(true)?;
        require_manager(&mut tx, actor, &req.organization_id)?;
        if uuid::Uuid::parse_str(&req.employee_user_id).is_err()
            || tx
                .role_of(&req.organization_id, &req.employee_user_id)?
                .is_none()
        {
            return Err(PayrollError::Validation(
                "employee must be a member of the organization".into(),
            ));
        }
        if req.employee_user_id == actor {
            return Err(PayrollError::Forbidden(
                "you cannot create a vesting agreement for yourself",
            ));
        }
        let a = VestingAgreement {
            id: new_id(),
            organization_id: req.organization_id,
            employee_user_id: req.employee_user_id,
            created_by: actor.to_string(),
            terms: VestingTerms {
                total_zats: req.total_zats,
                start_at,
                duration_secs: req.duration_secs,
                cliff_secs,
                method: req.method,
            },
            status: VestingStatus::Draft,
            created_at: now,
            updated_at: now,
            activated_at: None,
            activated_by: None,
            cancelled_at: None,
            cancelled_by: None,
            vested_at_cancel_zats: None,
        };
        tx.insert_vesting(&a)?;
        tx.insert_vesting_event(&a.id, "created", actor, now, 0)?;
        let a = load_vesting(&mut tx, &a.id, false)?;
        tx.commit()?;
        Ok(a)
    }

    /// Activates a draft at or before its start: it then vests by its
    /// schedule. After the start it is refused (no retroactive vesting).
    pub fn activate_vesting(&mut self, actor: &str, id: &str) -> Result<VestingAgreement> {
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        let a = load_vesting(&mut tx, id, true)?;
        require_can_view(&mut tx, actor, &a)?;
        require_manager(&mut tx, actor, &a.organization_id)?;
        if now < a.updated_at {
            return Err(PayrollError::ClockSkew);
        }
        match a.status {
            VestingStatus::Draft => {}
            VestingStatus::Active => {
                return Err(PayrollError::InvalidState(
                    "agreement is already active".into(),
                ));
            }
            VestingStatus::Cancelled => {
                return Err(PayrollError::InvalidState("agreement was cancelled".into()));
            }
        }
        if now > a.terms.start_at {
            return Err(PayrollError::InvalidState(
                "the agreement's start date has passed, so it can no longer be activated; cancel it and create a new one".into(),
            ));
        }
        if tx.activate_vesting_cas(id, actor, now)? != 1 {
            return Err(PayrollError::Conflict(
                "agreement changed concurrently; retry".into(),
            ));
        }
        let a = load_vesting(&mut tx, id, false)?;
        tx.insert_vesting_event(id, "activated", actor, now, a.vested_at(now))?;
        tx.commit()?;
        Ok(a)
    }

    /// Cancels an agreement under the cancellation policy (module docs):
    /// vesting stops now, vested compensation stays owed, the rest is
    /// forfeited. Claims are left exactly as they are.
    pub fn cancel_vesting(&mut self, actor: &str, id: &str) -> Result<VestingAgreement> {
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        // The row lock serializes cancellation against claim reservations.
        let a = load_vesting(&mut tx, id, true)?;
        require_can_view(&mut tx, actor, &a)?;
        require_manager(&mut tx, actor, &a.organization_id)?;
        if now < a.updated_at {
            return Err(PayrollError::ClockSkew);
        }
        let vested = match a.phase_at(now) {
            VestingPhase::Cancelled => {
                return Err(PayrollError::InvalidState(
                    "agreement is already cancelled".into(),
                ));
            }
            VestingPhase::FullyVested => {
                return Err(PayrollError::InvalidState(
                    "agreement is fully vested; there is nothing left to cancel".into(),
                ));
            }
            _ => a.vested_at(now),
        };
        let (reserved, paid) = tx.vesting_claim_sums(id)?;
        if vested < reserved + paid {
            // Only possible if the server clock went backwards.
            return Err(PayrollError::ClockSkew);
        }
        if tx.cancel_vesting_cas(id, a.status, actor, now, vested)? != 1 {
            return Err(PayrollError::Conflict(
                "agreement changed concurrently; retry".into(),
            ));
        }
        tx.insert_vesting_event(id, "cancelled", actor, now, vested)?;
        let a = load_vesting(&mut tx, id, false)?;
        tx.commit()?;
        Ok(a)
    }

    /// Every agreement of an organization, newest first. Owners/admins only.
    pub fn list_org_vesting(&mut self, actor: &str, org: &str) -> Result<Vec<VestingListing>> {
        validate_id("organization", org)?;
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        require_manager(&mut tx, actor, org)?;
        listings(tx.list_vesting(VestingFilter::Organization(org))?, now)
    }

    // ---- employee and managers ---------------------------------------------------

    /// The actor's own agreements (activated ones only), newest first.
    pub fn list_my_vesting(&mut self, actor: &str) -> Result<Vec<VestingListing>> {
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        listings(tx.list_vesting(VestingFilter::Employee(actor))?, now)
    }

    pub fn get_vesting(&mut self, actor: &str, id: &str) -> Result<VestingAgreement> {
        let mut tx = self.backend.begin(false)?;
        let a = load_vesting(&mut tx, id, false)?;
        require_can_view(&mut tx, actor, &a)?;
        Ok(a)
    }

    pub fn vesting_balance(&mut self, actor: &str, id: &str) -> Result<VestingBalance> {
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        let a = load_vesting(&mut tx, id, false)?;
        require_can_view(&mut tx, actor, &a)?;
        balance_in(&mut tx, &a, now)
    }

    /// The agreement's append-only lifecycle history, oldest first.
    pub fn vesting_events(&mut self, actor: &str, id: &str) -> Result<Vec<VestingEvent>> {
        let mut tx = self.backend.begin(false)?;
        let a = load_vesting(&mut tx, id, false)?;
        require_can_view(&mut tx, actor, &a)?;
        tx.vesting_events_of(id)
    }

    pub fn list_vesting_claims(&mut self, actor: &str, id: &str) -> Result<Vec<VestingClaim>> {
        let mut tx = self.backend.begin(false)?;
        let a = load_vesting(&mut tx, id, false)?;
        require_can_view(&mut tx, actor, &a)?;
        tx.vesting_claims_of(id)
    }

    /// Atomically reserves `amount_zats` of the employee's available vested
    /// compensation. Employee only. A repeat with the same `idempotency_key`
    /// returns the original claim (`duplicate = true`); reusing a key with
    /// different parameters is rejected.
    pub fn reserve_vesting_claim(
        &mut self,
        actor: &str,
        agreement_id: &str,
        amount_zats: i64,
        idempotency_key: &str,
    ) -> Result<VestingClaimOutcome> {
        validate_key(idempotency_key)?;
        if !(1..=MAX_MONEY_ZATS).contains(&amount_zats) {
            return Err(PayrollError::Validation(
                "amount_zats must be positive and at most 21,000,000 ZEC".into(),
            ));
        }
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        // Locking the agreement row serializes every reservation (and
        // cancellation) of this agreement.
        let a = load_vesting(&mut tx, agreement_id, true)?;
        require_can_view(&mut tx, actor, &a)?;
        if actor != a.employee_user_id {
            return Err(PayrollError::Forbidden(
                "only the employee can claim vested compensation",
            ));
        }
        if let Some(claim) = tx.vesting_claim_by_key(agreement_id, idempotency_key)? {
            return if claim.amount_zats == amount_zats && claim.requested_by == actor {
                Ok(VestingClaimOutcome {
                    claim,
                    duplicate: true,
                })
            } else {
                Err(PayrollError::IdempotencyConflict)
            };
        }
        let bal = balance_in(&mut tx, &a, now)?;
        if amount_zats > bal.available {
            return Err(PayrollError::InsufficientAvailable {
                requested: amount_zats,
                available: bal.available,
            });
        }
        let id = new_id();
        tx.insert_vesting_claim(
            &id,
            agreement_id,
            actor,
            idempotency_key,
            amount_zats,
            bal.vested,
            now,
        )?;
        let claim = tx
            .load_vesting_claim(&id, false)?
            .ok_or(PayrollError::NotFound("vesting claim"))?;
        tx.commit()?;
        Ok(VestingClaimOutcome {
            claim,
            duplicate: false,
        })
    }

    /// Cancels a reservation (e.g. a failed payment attempt); the amount
    /// becomes available again. The claimant or an owner/admin.
    pub fn release_vesting_claim(&mut self, actor: &str, claim_id: &str) -> Result<VestingClaim> {
        self.resolve_vesting_claim(actor, claim_id, None)
    }

    /// Records that a reserved claim was paid. Owner/admin only; accounting
    /// only. Final: a settled claim's amount can never be claimed again.
    pub fn settle_vesting_claim(
        &mut self,
        actor: &str,
        claim_id: &str,
        settlement_ref: &str,
    ) -> Result<VestingClaim> {
        validate_text("settlement_ref", settlement_ref, 256)?;
        self.resolve_vesting_claim(actor, claim_id, Some(settlement_ref))
    }

    fn resolve_vesting_claim(
        &mut self,
        actor: &str,
        claim_id: &str,
        settlement_ref: Option<&str>,
    ) -> Result<VestingClaim> {
        validate_id("vesting claim", claim_id)?;
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        let c = tx
            .load_vesting_claim(claim_id, true)?
            .ok_or(PayrollError::NotFound("vesting claim"))?;
        let a = load_vesting(&mut tx, &c.agreement_id, false)?;
        require_can_view(&mut tx, actor, &a)
            .map_err(|_| PayrollError::NotFound("vesting claim"))?;
        let is_manager = tx
            .role_of(&a.organization_id, actor)?
            .is_some_and(Role::manages_payroll);
        match settlement_ref {
            Some(_) if !is_manager => {
                return Err(PayrollError::Forbidden(
                    "only owners/admins can settle claims",
                ));
            }
            None if !(is_manager || actor == c.requested_by) => {
                return Err(PayrollError::Forbidden(
                    "only the claimant or an owner/admin can release a claim",
                ));
            }
            _ => {}
        }
        if c.status != ClaimStatus::Reserved {
            return Err(PayrollError::InvalidState(
                "claim is no longer reserved".into(),
            ));
        }
        // A claim with a payout is resolved only by that payout (confirmation
        // settles it, failure releases it), never by hand.
        if tx.payout_by_vesting_claim(claim_id, true)?.is_some() {
            return Err(PayrollError::InvalidState(
                "a payout is in progress for this claim".into(),
            ));
        }
        let to = if settlement_ref.is_some() {
            ClaimStatus::Settled
        } else {
            ClaimStatus::Released
        };
        if tx.update_vesting_claim_cas(claim_id, to, now, actor, settlement_ref)? != 1 {
            return Err(PayrollError::Conflict(
                "claim changed concurrently; retry".into(),
            ));
        }
        let c = tx
            .load_vesting_claim(claim_id, false)?
            .ok_or(PayrollError::NotFound("vesting claim"))?;
        tx.commit()?;
        Ok(c)
    }
}

// ---- payouts ---------------------------------------------------------------------

impl<B: Backend> Payroll<B> {
    /// Everything an agreement's screen shows, for the employee (once
    /// activated) or the organization's owners/admins. The sealed payout
    /// address is returned to the employee only.
    pub fn vesting_detail(&mut self, actor: &str, id: &str) -> Result<VestingDetail> {
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        let a = load_vesting(&mut tx, id, false)?;
        require_can_view(&mut tx, actor, &a)?;
        let row = tx
            .list_vesting(VestingFilter::Organization(&a.organization_id))?
            .into_iter()
            .find(|r| r.agreement.id == id)
            .ok_or(PayrollError::NotFound("vesting agreement"))?;
        let listing = listings(vec![row], now)?.remove(0);
        let address = tx.vesting_address(id)?;
        Ok(VestingDetail {
            listing,
            events: tx.vesting_events_of(id)?,
            claims: tx.vesting_claims_of(id)?,
            payouts: tx.payouts_of_vesting(id)?,
            address_locked: address.is_some(),
            address: address.filter(|_| actor == a.employee_user_id),
        })
    }

    /// Sets the agreement's payout address. Employee only, once: afterwards it
    /// is locked for the life of the agreement. Repeating it with the same
    /// address (same fingerprint) returns the lock (`true` = duplicate).
    pub fn set_vesting_payout_address(
        &mut self,
        actor: &str,
        id: &str,
        address_fingerprint: &str,
        address_sealed: &str,
    ) -> Result<(VestingPayoutAddress, bool)> {
        if address_fingerprint.len() != 64
            || address_sealed.is_empty()
            || address_sealed.len() > 4000
        {
            return Err(PayrollError::Validation("invalid payout address".into()));
        }
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        let a = load_vesting(&mut tx, id, true)?;
        require_can_view(&mut tx, actor, &a)?;
        if actor != a.employee_user_id {
            return Err(PayrollError::Forbidden(
                "only the employee can set the payout address",
            ));
        }
        if let Some(x) = tx.vesting_address(id)? {
            return if x.address_fingerprint == address_fingerprint {
                Ok((x, true))
            } else {
                Err(PayrollError::Conflict(
                    "the payout address for this agreement is locked".into(),
                ))
            };
        }
        let x = VestingPayoutAddress {
            agreement_id: id.to_string(),
            set_by: actor.to_string(),
            address_fingerprint: address_fingerprint.to_string(),
            address_sealed: address_sealed.to_string(),
            set_at: now,
        };
        tx.insert_vesting_address(&x)?;
        tx.commit()?;
        Ok((x, false))
    }

    /// Withdraws `terms.amount_zats` of vested compensation to the
    /// agreement's locked address: reserves a vesting claim and records its
    /// `reserved` payout (leased to the caller, who then submits it with
    /// [`crate::payout::submit_payout`]), atomically, under the agreement's
    /// row lock.
    ///
    /// Exactly-once: the same `idempotency_key` (with the same amount), or
    /// any request while a withdrawal of this agreement is unfinished,
    /// returns that payout (`duplicate = true`) instead of creating another.
    pub fn withdraw_vesting(
        &mut self,
        actor: &str,
        terms: &VestingWithdrawTerms<'_>,
        payout_fingerprint: &dyn Fn(&str) -> String,
        org_allowed: &dyn Fn(&str) -> bool,
    ) -> Result<PayoutOutcome> {
        validate_key(terms.idempotency_key)?;
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        // The agreement row lock serializes withdrawals, reservations and
        // cancellation of this agreement.
        let a = load_vesting(&mut tx, terms.agreement_id, true)?;
        require_can_view(&mut tx, actor, &a)?;
        if actor != a.employee_user_id {
            return Err(PayrollError::Forbidden(
                "only the employee can withdraw vested compensation",
            ));
        }

        if let Some(c) = tx.vesting_claim_by_key(&a.id, terms.idempotency_key)? {
            return match tx.payout_by_vesting_claim(&c.id, true)? {
                Some(p) if p.requested_by == actor && p.amount_zats == terms.amount_zats => {
                    Ok(PayoutOutcome {
                        payout: p,
                        duplicate: true,
                    })
                }
                _ => Err(PayrollError::IdempotencyConflict),
            };
        }
        if let Some(p) = tx
            .payouts_of_vesting(&a.id)?
            .into_iter()
            .find(|p| !p.state.is_final())
        {
            return Ok(PayoutOutcome {
                payout: p,
                duplicate: true,
            });
        }

        if !(1..=MAX_MONEY_ZATS).contains(&terms.amount_zats) {
            return Err(PayrollError::Validation(
                "amount must be positive and at most 21,000,000 ZEC".into(),
            ));
        }
        if !(0..=MAX_MONEY_ZATS).contains(&terms.fee_estimate_zats)
            || !(terms.fee_estimate_zats..=MAX_MONEY_ZATS).contains(&terms.max_fee_zats)
            || terms.lease_secs <= 0
            || terms.lease_owner.is_empty()
        {
            return Err(PayrollError::Validation("invalid withdrawal terms".into()));
        }
        let addr = tx.vesting_address(&a.id)?.ok_or_else(|| {
            PayrollError::InvalidState("set your payout address before withdrawing".into())
        })?;
        if addr.address_fingerprint != terms.address_fingerprint {
            return Err(PayrollError::InvalidState(
                "the payout address does not match this agreement's locked address".into(),
            ));
        }
        if !org_allowed(&a.organization_id) {
            return Err(PayrollError::Forbidden(
                "payouts are not enabled for this organization",
            ));
        }
        let bal = balance_in(&mut tx, &a, now)?;
        if terms.amount_zats > bal.available {
            return Err(PayrollError::InsufficientAvailable {
                requested: terms.amount_zats,
                available: bal.available,
            });
        }

        let claim_id = new_id();
        tx.insert_vesting_claim(
            &claim_id,
            &a.id,
            actor,
            terms.idempotency_key,
            terms.amount_zats,
            bal.vested,
            now,
        )?;
        let id = new_id();
        let p = Payout {
            recipient_fingerprint: payout_fingerprint(&id),
            id,
            compensation_type: CompensationType::Vesting,
            claim_id,
            source_id: a.id.clone(),
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
    use crate::money::COIN;

    const START: i64 = 1_000_000;

    fn terms(total: i64, duration: i64, cliff: i64) -> VestingTerms {
        VestingTerms {
            total_zats: total,
            start_at: START,
            duration_secs: duration,
            cliff_secs: cliff,
            method: VestingMethod::Linear,
        }
    }

    fn v(t: &VestingTerms, elapsed: i64) -> i64 {
        vested_amount(t, START + elapsed)
    }

    const YEAR: i64 = 365 * 86_400;

    #[test]
    fn nothing_before_or_at_start() {
        let t = terms(1_000, 1_000, 0);
        assert_eq!(vested_amount(&t, 0), 0);
        assert_eq!(v(&t, -1), 0);
        assert_eq!(v(&t, 0), 0, "zero elapsed time vests nothing");
        assert_eq!(v(&t, 1), 1);
    }

    #[test]
    fn cliff_boundaries() {
        // 4 ZEC over 4 years, 1-year cliff.
        let t = terms(4 * COIN, 4 * YEAR, YEAR);
        assert_eq!(v(&t, YEAR - 1), 0, "just before the cliff");
        assert_eq!(
            v(&t, YEAR),
            COIN,
            "exactly at the cliff: the cliff's share at once"
        );
        assert_eq!(
            v(&t, YEAR + 1),
            (4 * COIN as i128 * (YEAR + 1) as i128 / (4 * YEAR) as i128) as i64,
            "just after the cliff: linear"
        );
        assert!(v(&t, YEAR + 1) > COIN);
    }

    #[test]
    fn midway_end_and_after() {
        let t = terms(4 * COIN, 4 * YEAR, YEAR);
        assert_eq!(v(&t, 2 * YEAR), 2 * COIN);
        // 4 ZEC / 4 years ≈ 3.17 zats/s: one second short leaves 3.17 unvested,
        // floored to 4 zats short.
        assert_eq!(v(&t, 4 * YEAR - 1), 4 * COIN - 4);
        assert_eq!(v(&t, 4 * YEAR), 4 * COIN, "exactly at the end: everything");
        assert_eq!(v(&t, 4 * YEAR + 1), 4 * COIN);
        assert_eq!(vested_amount(&t, i64::MAX), 4 * COIN);
    }

    #[test]
    fn very_short_and_long_durations() {
        let one = terms(7, 1, 0);
        assert_eq!((v(&one, 0), v(&one, 1), v(&one, 2)), (0, 7, 7));
        let one_cliff = terms(7, 1, 1);
        assert_eq!((v(&one_cliff, 0), v(&one_cliff, 1)), (0, 7));

        let max = terms(MAX_MONEY_ZATS, MAX_RATE_PERIOD_SECS, 0);
        assert_eq!(v(&max, MAX_RATE_PERIOD_SECS / 2), MAX_MONEY_ZATS / 2);
        // 21M ZEC / 100 years ≈ 665,905.6 zats/s; floored, 665,906 short.
        assert_eq!(v(&max, MAX_RATE_PERIOD_SECS - 1), MAX_MONEY_ZATS - 665_906);
        assert_eq!(v(&max, MAX_RATE_PERIOD_SECS), MAX_MONEY_ZATS);
    }

    #[test]
    fn small_allocations_floor_and_reconcile() {
        // 3 zats over 10 s: 0.3/s.
        let t = terms(3, 10, 0);
        let got: Vec<i64> = (0..=11).map(|e| v(&t, e)).collect();
        assert_eq!(got, [0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3]);
        // 1 zat over a year is 0 until the very end.
        let t = terms(1, YEAR, 0);
        assert_eq!(v(&t, YEAR - 1), 0);
        assert_eq!(v(&t, YEAR), 1);
    }

    #[test]
    fn cliff_equal_to_duration_vests_all_at_the_end() {
        let t = terms(999, 100, 100);
        assert_eq!(v(&t, 99), 0);
        assert_eq!(v(&t, 100), 999);
    }

    #[test]
    fn never_exceeds_total_monotonic_and_ends_exactly_at_total() {
        for (total, duration, cliff) in [
            (1, 1, 0),
            (1, 7, 3),
            (13, 7, 0),
            (1_000_003, 97, 13),
            (COIN + 1, 86_400, 3_600),
            (7, 1_000, 999),
        ] {
            let t = terms(total, duration, cliff);
            let mut prev = 0;
            for e in -2..=duration + 2 {
                let x = v(&t, e);
                assert!(x >= prev && x <= total, "{total}/{duration}/{cliff} at {e}");
                if e < cliff {
                    assert_eq!(x, 0);
                }
                prev = x;
            }
            assert_eq!(v(&t, duration), total, "fully vested = total");
        }
    }

    #[test]
    fn floor_is_within_one_zat_of_exact_value() {
        let t = terms(1_000_003, 97, 0);
        for e in 0..97 {
            let exact_num = 1_000_003i128 * e as i128; // exact = exact_num / 97
            let x = v(&t, e) as i128;
            assert!(x * 97 <= exact_num && exact_num < (x + 1) * 97);
        }
    }

    fn agreement(status: VestingStatus, frozen: Option<i64>) -> VestingAgreement {
        VestingAgreement {
            id: "a".into(),
            organization_id: "o".into(),
            employee_user_id: "e".into(),
            created_by: "c".into(),
            terms: terms(1_000, 1_000, 100),
            status,
            created_at: 0,
            updated_at: 0,
            activated_at: (status != VestingStatus::Draft).then_some(0),
            activated_by: None,
            cancelled_at: frozen.map(|_| 0),
            cancelled_by: None,
            vested_at_cancel_zats: frozen,
        }
    }

    #[test]
    fn status_decides_what_is_vested() {
        let draft = agreement(VestingStatus::Draft, None);
        assert_eq!(draft.vested_at(START + 2_000), 0);
        assert_eq!(draft.phase_at(START + 2_000), VestingPhase::Draft);

        let active = agreement(VestingStatus::Active, None);
        assert_eq!(active.phase_at(START - 1), VestingPhase::NotStarted);
        assert_eq!(active.phase_at(START), VestingPhase::InCliff);
        assert_eq!(active.phase_at(START + 99), VestingPhase::InCliff);
        assert_eq!(active.phase_at(START + 100), VestingPhase::Vesting);
        assert_eq!(active.phase_at(START + 1_000), VestingPhase::FullyVested);
        assert_eq!(active.vested_at(START + 500), 500);

        let cancelled = agreement(VestingStatus::Cancelled, Some(300));
        assert_eq!(cancelled.vested_at(START + 500), 300);
        assert_eq!(cancelled.vested_at(START + 5_000), 300, "vesting stopped");
        assert_eq!(cancelled.phase_at(START + 5_000), VestingPhase::Cancelled);
    }
}

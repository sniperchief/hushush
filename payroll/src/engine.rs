//! Payroll operations: validation, authorization, accrual and atomic claim
//! reservation. Written once, generic over the storage [`Backend`].

use std::sync::Arc;

use crate::accrual::AccrualState;
use crate::clock::Clock;
use crate::error::{PayrollError, Result};
use crate::model::*;
use crate::money::MAX_MONEY_ZATS;
use crate::store::{Backend, StoreTx};

/// Longest supported rate period (100 years); mirrors the schema CHECK.
pub const MAX_RATE_PERIOD_SECS: i64 = 3_153_600_000;

pub struct Payroll<B: Backend> {
    pub(crate) backend: B,
    pub(crate) clock: Arc<dyn Clock>,
}

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) fn validate_text(field: &str, v: &str, max: usize) -> Result<()> {
    let len = v.chars().count();
    if len == 0 || len > max || v.trim() != v || v.chars().any(char::is_control) {
        return Err(PayrollError::Validation(format!(
            "{field} must be 1..={max} characters without leading/trailing whitespace or control characters"
        )));
    }
    Ok(())
}

pub(crate) fn validate_id(field: &'static str, v: &str) -> Result<()> {
    if uuid::Uuid::parse_str(v).is_err() {
        // Malformed ids can't exist; report them like any unknown id.
        return Err(PayrollError::NotFound(field));
    }
    Ok(())
}

/// Recipient, or an owner/admin of the paying organization.
pub(crate) fn require_can_view<T: StoreTx>(tx: &mut T, actor: &str, s: &Stream) -> Result<()> {
    if actor == s.recipient_user_id {
        return Ok(());
    }
    match tx.role_of(&s.organization_id, actor)? {
        Some(r) if r.manages_payroll() => Ok(()),
        // Do not reveal whether the stream exists to outsiders.
        _ => Err(PayrollError::NotFound("stream")),
    }
}

pub(crate) fn require_manager<T: StoreTx>(tx: &mut T, actor: &str, org: &str) -> Result<Role> {
    match tx.role_of(org, actor)? {
        Some(r) if r.manages_payroll() => Ok(r),
        Some(_) => Err(PayrollError::Forbidden(
            "only organization owners/admins can manage payroll",
        )),
        None => Err(PayrollError::NotFound("organization")),
    }
}

fn balance_from(s: &Stream, now: i64, reserved: i64, claimed: i64) -> Result<Balance> {
    let earned = s.accrual.earned_at(now);
    let available = earned - reserved - claimed;
    if available < 0 {
        // Unreachable if the invariants hold; never paper over it.
        return Err(PayrollError::InvalidState(format!(
            "ledger invariant violated for stream {}: earned {earned} < reserved {reserved} + claimed {claimed}",
            s.id
        )));
    }
    Ok(Balance {
        stream_id: s.id.clone(),
        as_of: now,
        status: s.effective_status_at(now),
        earned,
        reserved,
        claimed,
        available,
    })
}

fn balance_in<T: StoreTx>(tx: &mut T, s: &Stream, now: i64) -> Result<Balance> {
    let (reserved, claimed) = tx.claim_sums(&s.id)?;
    balance_from(s, now, reserved, claimed)
}

fn record_event<T: StoreTx>(
    tx: &mut T,
    s: &Stream,
    kind: &str,
    actor: &str,
    now: i64,
) -> Result<()> {
    tx.insert_event(
        &s.id,
        kind,
        actor,
        now,
        s.accrual.active_secs_at(now),
        s.accrual.earned_at(now),
    )
}

pub(crate) fn load_stream<T: StoreTx>(tx: &mut T, id: &str, lock: bool) -> Result<Stream> {
    validate_id("stream", id)?;
    tx.load_stream(id, lock)?
        .ok_or(PayrollError::NotFound("stream"))
}

#[derive(Clone, Copy)]
enum Op {
    Pause,
    Resume,
    End,
}

/// Pure lifecycle rule: given the current stream and time, what changes.
fn plan(op: Op, s: &Stream, now: i64) -> Result<(StreamStatus, StreamStatus, AccrualState)> {
    use StreamStatus::*;
    match op {
        Op::Pause => match s.effective_status_at(now) {
            Active => Ok((Active, Paused, s.accrual.closed_at(now))),
            Paused => Err(PayrollError::InvalidState(
                "stream is already paused".into(),
            )),
            Ended => Err(PayrollError::InvalidState("stream has ended".into())),
        },
        Op::Resume => {
            if s.status == Paused && s.accrual.schedule_finished_at(now) {
                return Err(PayrollError::InvalidState(
                    "stream end date has passed".into(),
                ));
            }
            match s.status {
                Paused => Ok((Paused, Active, s.accrual.resumed_at(now))),
                Active => Err(PayrollError::InvalidState("stream is not paused".into())),
                Ended => Err(PayrollError::InvalidState("stream has ended".into())),
            }
        }
        Op::End => match s.status {
            Ended => Err(PayrollError::InvalidState(
                "stream has already ended".into(),
            )),
            from => Ok((from, Ended, s.accrual.closed_at(now))),
        },
    }
}

impl<B: Backend> Payroll<B> {
    pub fn from_backend(backend: B, clock: Arc<dyn Clock>) -> Self {
        Self { backend, clock }
    }

    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    // ---- users & organizations -------------------------------------------------

    /// Registers a user (sign-up). Authentication is handled by the caller
    /// (the web app); every other operation takes the authenticated user's id
    /// as `actor`.
    pub fn create_user(&mut self, email: &str, display_name: &str) -> Result<String> {
        validate_text("email", email, 320)?;
        validate_text("display_name", display_name, 200)?;
        let at = email.find('@');
        if !matches!(at, Some(i) if i > 0 && i + 1 < email.len()) || email.contains(' ') {
            return Err(PayrollError::Validation("email is not valid".into()));
        }
        let id = new_id();
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        tx.insert_user(&id, email, display_name, now)?;
        tx.commit()?;
        Ok(id)
    }

    /// Creates an organization; `actor` becomes its owner.
    pub fn create_organization(&mut self, actor: &str, name: &str) -> Result<String> {
        validate_text("name", name, 200)?;
        validate_id("user", actor)?;
        let id = new_id();
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        if !tx.user_exists(actor)? {
            return Err(PayrollError::NotFound("user"));
        }
        tx.insert_org(&id, name, actor, now)?;
        tx.insert_member(&id, actor, Role::Owner, now)?;
        tx.commit()?;
        Ok(id)
    }

    /// Adds `user` to `org`. Owners may grant any role; admins only `member`.
    pub fn add_member(&mut self, actor: &str, org: &str, user: &str, role: Role) -> Result<()> {
        validate_id("organization", org)?;
        validate_id("user", user)?;
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        match (tx.role_of(org, actor)?, role) {
            (Some(Role::Owner), _) | (Some(Role::Admin), Role::Member) => {}
            (Some(_), _) => {
                return Err(PayrollError::Forbidden(
                    "insufficient role to add this member",
                ));
            }
            (None, _) => return Err(PayrollError::NotFound("organization")),
        }
        if !tx.user_exists(user)? {
            return Err(PayrollError::NotFound("user"));
        }
        if tx.role_of(org, user)?.is_some() {
            return Err(PayrollError::Conflict("user is already a member".into()));
        }
        tx.insert_member(org, user, role, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Changes a member's role. Owner only; the last owner cannot be demoted.
    pub fn set_member_role(
        &mut self,
        actor: &str,
        org: &str,
        user: &str,
        role: Role,
    ) -> Result<()> {
        validate_id("organization", org)?;
        validate_id("user", user)?;
        let mut tx = self.backend.begin(true)?;
        match tx.role_of(org, actor)? {
            Some(Role::Owner) => {}
            Some(_) => return Err(PayrollError::Forbidden("only owners can change roles")),
            None => return Err(PayrollError::NotFound("organization")),
        }
        let current = tx
            .role_of(org, user)?
            .ok_or(PayrollError::NotFound("member"))?;
        if current == Role::Owner && role != Role::Owner && tx.count_owners(org)? <= 1 {
            return Err(PayrollError::InvalidState(
                "an organization must keep at least one owner".into(),
            ));
        }
        tx.update_member_role(org, user, role)?;
        tx.commit()?;
        Ok(())
    }

    /// Removes a member. Owners may remove anyone except the last owner;
    /// admins may remove plain members. Members with payroll streams (even
    /// ended ones) are kept so payroll history stays intact.
    pub fn remove_member(&mut self, actor: &str, org: &str, user: &str) -> Result<()> {
        validate_id("organization", org)?;
        validate_id("user", user)?;
        let mut tx = self.backend.begin(true)?;
        let actor_role = tx
            .role_of(org, actor)?
            .ok_or(PayrollError::NotFound("organization"))?;
        let target = tx
            .role_of(org, user)?
            .ok_or(PayrollError::NotFound("member"))?;
        match (actor_role, target) {
            (Role::Owner, Role::Owner) if tx.count_owners(org)? <= 1 => {
                return Err(PayrollError::InvalidState(
                    "an organization must keep at least one owner".into(),
                ));
            }
            (Role::Owner, _) | (Role::Admin, Role::Member) => {}
            _ => {
                return Err(PayrollError::Forbidden(
                    "insufficient role to remove this member",
                ));
            }
        }
        if tx.count_streams_for_member(org, user)? > 0 {
            return Err(PayrollError::Conflict(
                "member has payroll streams; end them instead of removing the member".into(),
            ));
        }
        tx.delete_member(org, user)?;
        tx.commit()?;
        Ok(())
    }

    /// Organizations the actor belongs to, with their role.
    pub fn list_organizations(&mut self, actor: &str) -> Result<Vec<Membership>> {
        let mut tx = self.backend.begin(false)?;
        tx.memberships_of(actor)
    }

    /// Member directory. Owners/admins only (colleagues' details are private).
    pub fn list_members(&mut self, actor: &str, org: &str) -> Result<Vec<Member>> {
        validate_id("organization", org)?;
        let mut tx = self.backend.begin(false)?;
        require_manager(&mut tx, actor, org)?;
        tx.members_of(org)
    }

    // ---- streams --------------------------------------------------------------

    pub fn create_stream(&mut self, actor: &str, req: NewStream) -> Result<Stream> {
        validate_id("organization", &req.organization_id)?;
        let now = self.now();
        let start_at = req.start_at.unwrap_or(now);
        if !(1..=MAX_MONEY_ZATS).contains(&req.rate_zats) {
            return Err(PayrollError::Validation(
                "rate_zats must be 1..=21,000,000 ZEC in zatoshis".into(),
            ));
        }
        if !(1..=MAX_RATE_PERIOD_SECS).contains(&req.rate_period_secs) {
            return Err(PayrollError::Validation(
                "rate_period_secs must be 1..=100 years".into(),
            ));
        }
        if start_at < now {
            return Err(PayrollError::Validation(
                "start_at may not be in the past".into(),
            ));
        }
        if req.end_at.is_some_and(|e| e <= start_at) {
            return Err(PayrollError::Validation(
                "end_at must be after start_at".into(),
            ));
        }

        let id = new_id();
        let mut tx = self.backend.begin(true)?;
        require_manager(&mut tx, actor, &req.organization_id)?;
        if uuid::Uuid::parse_str(&req.recipient_user_id).is_err()
            || tx
                .role_of(&req.organization_id, &req.recipient_user_id)?
                .is_none()
        {
            return Err(PayrollError::Validation(
                "recipient must be a member of the organization".into(),
            ));
        }
        tx.insert_stream(&id, actor, &req, start_at, now)?;
        let stream = load_stream(&mut tx, &id, false)?;
        record_event(&mut tx, &stream, "created", actor, now)?;
        tx.commit()?;
        Ok(stream)
    }

    pub fn get_stream(&mut self, actor: &str, id: &str) -> Result<Stream> {
        let mut tx = self.backend.begin(false)?;
        let s = load_stream(&mut tx, id, false)?;
        require_can_view(&mut tx, actor, &s)?;
        Ok(s)
    }

    /// Pauses accrual. Wages earned so far are kept.
    pub fn pause_stream(&mut self, actor: &str, id: &str) -> Result<Stream> {
        self.transition(actor, id, Op::Pause)
    }

    /// Resumes accrual from now (or from `start_at` if not yet started).
    pub fn resume_stream(&mut self, actor: &str, id: &str) -> Result<Stream> {
        self.transition(actor, id, Op::Resume)
    }

    /// Ends the stream permanently. Earned wages remain claimable.
    pub fn end_stream(&mut self, actor: &str, id: &str) -> Result<Stream> {
        self.transition(actor, id, Op::End)
    }

    fn transition(&mut self, actor: &str, id: &str, op: Op) -> Result<Stream> {
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        let s = load_stream(&mut tx, id, true)?;
        require_can_view(&mut tx, actor, &s)?;
        require_manager(&mut tx, actor, &s.organization_id)?;
        if now < s.last_event_at {
            return Err(PayrollError::ClockSkew);
        }
        let (from, to, next) = plan(op, &s, now)?;
        let ended_at = (to == StreamStatus::Ended).then_some(now);
        if tx.update_stream_cas(id, from, to, &next, ended_at, now)? != 1 {
            return Err(PayrollError::Conflict(
                "stream changed concurrently; retry".into(),
            ));
        }
        let updated = load_stream(&mut tx, id, false)?;
        let kind = match to {
            StreamStatus::Paused => "paused",
            StreamStatus::Active => "resumed",
            StreamStatus::Ended => "ended",
        };
        record_event(&mut tx, &updated, kind, actor, now)?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn stream_events(&mut self, actor: &str, id: &str) -> Result<Vec<StreamEvent>> {
        let mut tx = self.backend.begin(false)?;
        let s = load_stream(&mut tx, id, false)?;
        require_can_view(&mut tx, actor, &s)?;
        tx.events_of(id)
    }

    /// All streams of an organization with balances. Owners/admins only.
    pub fn list_org_streams(&mut self, actor: &str, org: &str) -> Result<Vec<StreamListing>> {
        validate_id("organization", org)?;
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        require_manager(&mut tx, actor, org)?;
        listings(tx.list_streams(StreamFilter::Organization(org))?, now)
    }

    /// The actor's own streams across all organizations, with balances.
    pub fn list_my_streams(&mut self, actor: &str) -> Result<Vec<StreamListing>> {
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        listings(tx.list_streams(StreamFilter::Recipient(actor))?, now)
    }

    /// One stream with display context and balance, for viewers of the stream.
    pub fn stream_listing(&mut self, actor: &str, id: &str) -> Result<StreamListing> {
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        let s = load_stream(&mut tx, id, false)?;
        require_can_view(&mut tx, actor, &s)?;
        let rows = tx.list_streams(StreamFilter::Organization(&s.organization_id))?;
        let row = rows
            .into_iter()
            .find(|r| r.stream.id == id)
            .ok_or(PayrollError::NotFound("stream"))?;
        listings(vec![row], now).map(|mut v| v.remove(0))
    }

    // ---- balances & claims ----------------------------------------------------

    pub fn balance(&mut self, actor: &str, stream_id: &str) -> Result<Balance> {
        let now = self.now();
        let mut tx = self.backend.begin(false)?;
        let s = load_stream(&mut tx, stream_id, false)?;
        require_can_view(&mut tx, actor, &s)?;
        balance_in(&mut tx, &s, now)
    }

    /// Atomically reserves `amount_zats` of the recipient's available wages.
    ///
    /// Only the stream's recipient may claim. A repeated request with the same
    /// `idempotency_key` returns the original claim (`duplicate = true`); reusing
    /// a key with different parameters is rejected.
    pub fn reserve_claim(
        &mut self,
        actor: &str,
        stream_id: &str,
        amount_zats: i64,
        idempotency_key: &str,
    ) -> Result<ClaimOutcome> {
        if !(1..=128).contains(&idempotency_key.len())
            || !idempotency_key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(PayrollError::Validation(
                "idempotency_key must be 1..=128 printable ASCII characters".into(),
            ));
        }
        if !(1..=MAX_MONEY_ZATS).contains(&amount_zats) {
            return Err(PayrollError::Validation(
                "amount_zats must be positive and at most 21,000,000 ZEC".into(),
            ));
        }
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        // Locking the stream row serializes all reservations for this stream.
        let s = load_stream(&mut tx, stream_id, true)?;
        require_can_view(&mut tx, actor, &s)?;
        if actor != s.recipient_user_id {
            return Err(PayrollError::Forbidden(
                "only the stream recipient can claim wages",
            ));
        }

        if let Some(claim) = tx.claim_by_key(stream_id, idempotency_key)? {
            return if claim.amount_zats == amount_zats && claim.requested_by == actor {
                Ok(ClaimOutcome {
                    claim,
                    duplicate: true,
                })
            } else {
                Err(PayrollError::IdempotencyConflict)
            };
        }

        let bal = balance_in(&mut tx, &s, now)?;
        if amount_zats > bal.available {
            return Err(PayrollError::InsufficientAvailable {
                requested: amount_zats,
                available: bal.available,
            });
        }
        let id = new_id();
        tx.insert_claim(
            &id,
            stream_id,
            actor,
            idempotency_key,
            amount_zats,
            bal.earned,
            now,
        )?;
        let claim = tx
            .load_claim(&id, false)?
            .ok_or(PayrollError::NotFound("claim"))?;
        tx.commit()?;
        Ok(ClaimOutcome {
            claim,
            duplicate: false,
        })
    }

    /// Cancels a reservation; the amount becomes available again. Allowed for
    /// the claimant or an owner/admin.
    pub fn release_claim(&mut self, actor: &str, claim_id: &str) -> Result<Claim> {
        self.resolve_claim(actor, claim_id, None)
    }

    /// Records that a reserved claim was paid outside hushush. Owner/admin
    /// only; accounting only, and the web API does not expose it. Claims paid
    /// through hushush are settled by their payout's confirmation instead
    /// (see [`crate::payout`]); this refuses claims that have a payout.
    pub fn settle_claim(
        &mut self,
        actor: &str,
        claim_id: &str,
        settlement_ref: &str,
    ) -> Result<Claim> {
        validate_text("settlement_ref", settlement_ref, 256)?;
        self.resolve_claim(actor, claim_id, Some(settlement_ref))
    }

    fn resolve_claim(
        &mut self,
        actor: &str,
        claim_id: &str,
        settlement_ref: Option<&str>,
    ) -> Result<Claim> {
        validate_id("claim", claim_id)?;
        let now = self.now();
        let mut tx = self.backend.begin(true)?;
        let c = tx
            .load_claim(claim_id, true)?
            .ok_or(PayrollError::NotFound("claim"))?;
        let s = load_stream(&mut tx, &c.stream_id, false)?;
        require_can_view(&mut tx, actor, &s).map_err(|_| PayrollError::NotFound("claim"))?;
        let is_manager = tx
            .role_of(&s.organization_id, actor)?
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
        if tx.payout_by_claim(claim_id, true)?.is_some() {
            return Err(PayrollError::InvalidState(
                "a payout is in progress for this claim".into(),
            ));
        }
        let to = if settlement_ref.is_some() {
            ClaimStatus::Settled
        } else {
            ClaimStatus::Released
        };
        if tx.update_claim_cas(claim_id, to, now, actor, settlement_ref)? != 1 {
            return Err(PayrollError::Conflict(
                "claim changed concurrently; retry".into(),
            ));
        }
        let c = tx
            .load_claim(claim_id, false)?
            .ok_or(PayrollError::NotFound("claim"))?;
        tx.commit()?;
        Ok(c)
    }

    pub fn list_claims(&mut self, actor: &str, stream_id: &str) -> Result<Vec<Claim>> {
        let mut tx = self.backend.begin(false)?;
        let s = load_stream(&mut tx, stream_id, false)?;
        require_can_view(&mut tx, actor, &s)?;
        tx.claims_of(stream_id)
    }
}

fn listings(rows: Vec<ListedRow>, now: i64) -> Result<Vec<StreamListing>> {
    rows.into_iter()
        .map(|r| {
            let balance = balance_from(&r.stream, now, r.reserved, r.claimed)?;
            Ok(StreamListing {
                stream: r.stream,
                organization_name: r.organization_name,
                recipient_display_name: r.recipient_display_name,
                recipient_email: r.recipient_email,
                balance,
            })
        })
        .collect()
}

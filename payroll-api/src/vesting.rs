//! Vesting endpoints (Vesting Phase 2): agreements, the locked payout
//! address, quotes, and withdrawals of vested compensation.
//!
//! Rules live in the engine (`hushush_payroll::vesting`). Withdrawals reserve
//! a vesting claim and its `reserved` payout atomically and are then submitted,
//! confirmed and reconciled by the same payout machinery as stream
//! withdrawals (`crate::payouts`): the same wallet, the organization's own
//! payout account, the same worker.
//!
//! Authorization: every request carries the service token and the web app's
//! authenticated user (`X-Actor-Id`); the engine checks that user against
//! the database. Managers (owners/admins) create, activate, cancel and view an
//! organization's agreements; an employee views and withdraws from their own
//! activated agreements only. Everyone else is told an agreement does not
//! exist.
//!
//! The payout address is sealed with the associated data `vesting:<id>`, so a
//! sealed address can't be moved between agreements or onto a stream. It is
//! never logged and is returned only to the agreement's employee.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;

use hushush_payroll::money::format_zec;
use hushush_payroll::payout::{recipient_fingerprint, submit_payout};
use hushush_payroll::vesting::vested_amount;
use hushush_payroll::{
    ClaimStatus, CompensationType, NewVesting, VestingClaim, VestingDetail, VestingEvent,
    VestingListing, VestingMethod, VestingPhase, VestingStatus, VestingWithdrawTerms,
};

use crate::payouts::{
    AddressReq, OrgWallet, PayoutView, QuoteView, blocking, clean_address, enabled, quote_view,
    wallet_err,
};
use crate::{ApiError, ApiResult, AppState, actor, err, parse_amount, require_actor, with_db};

/// Associated data for sealing an agreement's address (see module docs).
fn seal_id(agreement: &str) -> String {
    format!("vesting:{agreement}")
}

// ---- views ---------------------------------------------------------------------

#[derive(Serialize)]
pub struct VestingBalanceView {
    as_of: i64,
    total_zats: i64,
    total_zec: String,
    vested_zats: i64,
    vested_zec: String,
    unvested_zats: i64,
    unvested_zec: String,
    /// Held for a withdrawal in progress; not paid.
    reserved_zats: i64,
    reserved_zec: String,
    /// Paid: confirmed on chain.
    paid_zats: i64,
    paid_zec: String,
    available_zats: i64,
    available_zec: String,
}

#[derive(Serialize)]
pub struct VestingView {
    id: String,
    organization_id: String,
    organization_name: String,
    employee_user_id: String,
    employee_display_name: String,
    created_by: String,
    method: VestingMethod,
    /// draft | active | cancelled
    status: VestingStatus,
    /// draft | not_started | in_cliff | vesting | fully_vested | cancelled
    /// (at `balance.as_of`)
    phase: VestingPhase,
    total_zats: i64,
    total_zec: String,
    start_at: i64,
    end_at: i64,
    duration_secs: i64,
    cliff_secs: i64,
    cliff_at: i64,
    /// What vests at once at the cliff (the linear share of the cliff
    /// period); 0 without a cliff.
    cliff_vests_zats: i64,
    cliff_vests_zec: String,
    created_at: i64,
    activated_at: Option<i64>,
    cancelled_at: Option<i64>,
    /// A draft can be activated until its start.
    activatable: bool,
    /// Drafts and agreements that are not yet fully vested can be cancelled.
    cancellable: bool,
    balance: VestingBalanceView,
}

impl From<&VestingListing> for VestingView {
    fn from(l: &VestingListing) -> Self {
        let a = &l.agreement;
        let b = &l.balance;
        let cliff_vests = if a.terms.cliff_secs > 0 {
            vested_amount(&a.terms, a.cliff_at())
        } else {
            0
        };
        Self {
            id: a.id.clone(),
            organization_id: a.organization_id.clone(),
            organization_name: l.organization_name.clone(),
            employee_user_id: a.employee_user_id.clone(),
            employee_display_name: l.employee_display_name.clone(),
            created_by: a.created_by.clone(),
            method: a.terms.method,
            status: a.status,
            phase: b.phase,
            total_zats: a.terms.total_zats,
            total_zec: format_zec(a.terms.total_zats),
            start_at: a.terms.start_at,
            end_at: a.end_at(),
            duration_secs: a.terms.duration_secs,
            cliff_secs: a.terms.cliff_secs,
            cliff_at: a.cliff_at(),
            cliff_vests_zats: cliff_vests,
            cliff_vests_zec: format_zec(cliff_vests),
            created_at: a.created_at,
            activated_at: a.activated_at,
            cancelled_at: a.cancelled_at,
            activatable: a.status == VestingStatus::Draft && b.as_of <= a.terms.start_at,
            cancellable: !matches!(b.phase, VestingPhase::Cancelled | VestingPhase::FullyVested),
            balance: VestingBalanceView {
                as_of: b.as_of,
                total_zats: b.total,
                total_zec: format_zec(b.total),
                vested_zats: b.vested,
                vested_zec: format_zec(b.vested),
                unvested_zats: b.unvested,
                unvested_zec: format_zec(b.unvested),
                reserved_zats: b.reserved,
                reserved_zec: format_zec(b.reserved),
                paid_zats: b.paid,
                paid_zec: format_zec(b.paid),
                available_zats: b.available,
                available_zec: format_zec(b.available),
            },
        }
    }
}

#[derive(Serialize)]
pub struct VestingClaimView {
    id: String,
    compensation_type: CompensationType,
    amount_zats: i64,
    amount_zec: String,
    /// `reserved` = pending and UNPAID, even while its payout is in flight.
    /// `settled` = paid: its payout was confirmed on chain.
    status: ClaimStatus,
    created_at: i64,
    resolved_at: Option<i64>,
    payout: Option<PayoutView>,
}

#[derive(Serialize)]
pub struct VestingDetailView {
    agreement: VestingView,
    /// "employee" when the caller is the agreement's employee, else "manager".
    viewer: &'static str,
    events: Vec<VestingEvent>,
    /// Newest first.
    claims: Vec<VestingClaimView>,
    /// Newest first.
    payouts: Vec<PayoutView>,
    address_locked: bool,
    /// The locked address; only for the employee.
    address: Option<String>,
    payouts_enabled: bool,
    /// What may be withdrawn right now (up to all of `available`), if anything.
    withdrawable_zats: Option<i64>,
    withdrawable_zec: Option<String>,
}

fn detail_view(
    st: &AppState,
    actor: &str,
    d: &VestingDetail,
) -> Result<VestingDetailView, ApiError> {
    let a = &d.listing.agreement;
    let po = st.payouts.as_ref();
    let need = po.map_or(0, |p| p.policy.required_confirmations);
    let enabled = po.is_some_and(|p| p.orgs.allows(&a.organization_id));
    let address = match (&d.address, po) {
        (Some(x), Some(p)) => Some(p.sealer.open(&seal_id(&a.id), &x.address_sealed).map_err(
            |e| {
                tracing::error!(agreement_id = %a.id, "cannot open sealed payout address: {e}");
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "The stored payout address could not be read.",
                )
            },
        )?),
        _ => None,
    };
    let available = d.listing.balance.available;
    let withdrawable = (enabled && d.address_locked && d.open_payout().is_none() && available > 0)
        .then_some(available);
    let payout_of = |c: &VestingClaim| {
        d.payouts
            .iter()
            .find(|p| p.claim_id == c.id)
            .map(|p| PayoutView::new(p, need))
    };
    Ok(VestingDetailView {
        agreement: (&d.listing).into(),
        viewer: if actor == a.employee_user_id {
            "employee"
        } else {
            "manager"
        },
        events: d.events.clone(),
        claims: d
            .claims
            .iter()
            .rev()
            .map(|c| VestingClaimView {
                id: c.id.clone(),
                compensation_type: c.compensation_type,
                amount_zats: c.amount_zats,
                amount_zec: format_zec(c.amount_zats),
                status: c.status,
                created_at: c.created_at,
                resolved_at: c.resolved_at,
                payout: payout_of(c),
            })
            .collect(),
        payouts: d
            .payouts
            .iter()
            .rev()
            .map(|p| PayoutView::new(p, need))
            .collect(),
        address_locked: d.address_locked,
        address,
        payouts_enabled: enabled,
        withdrawable_zats: withdrawable,
        withdrawable_zec: withdrawable.map(format_zec),
    })
}

// ---- employer ------------------------------------------------------------------

pub async fn list_org(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(org): Path<String>,
) -> ApiResult<Vec<VestingView>> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let v = with_db(&st, move |db| db.list_org_vesting(&a, &org)).await?;
    Ok(Json(v.iter().map(VestingView::from).collect()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewVestingReq {
    employee_user_id: String,
    /// Decimal ZEC, e.g. "1.5". Parsed exactly into zatoshis.
    total_zec: String,
    start_at: Option<i64>,
    duration_secs: i64,
    cliff_secs: Option<i64>,
}

/// Creates a draft agreement (linear vesting).
pub async fn create(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(org): Path<String>,
    Json(r): Json<NewVestingReq>,
) -> Result<(StatusCode, Json<VestingView>), ApiError> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let total = parse_amount("total_zec", &r.total_zec)?;
    let l = with_db(&st, move |db| {
        let v = db.create_vesting(
            &a,
            NewVesting {
                organization_id: org.clone(),
                employee_user_id: r.employee_user_id,
                total_zats: total,
                start_at: r.start_at,
                duration_secs: r.duration_secs,
                cliff_secs: r.cliff_secs,
                method: VestingMethod::Linear,
            },
        )?;
        Ok(db.vesting_detail(&a, &v.id)?.listing)
    })
    .await?;
    Ok((StatusCode::CREATED, Json((&l).into())))
}

async fn lifecycle(st: AppState, h: HeaderMap, id: String, cancel: bool) -> ApiResult<VestingView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let l = with_db(&st, move |db| {
        if cancel {
            db.cancel_vesting(&a, &id)?;
        } else {
            db.activate_vesting(&a, &id)?;
        }
        Ok(db.vesting_detail(&a, &id)?.listing)
    })
    .await?;
    Ok(Json((&l).into()))
}

pub async fn activate(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<VestingView> {
    lifecycle(st, h, id, false).await
}

pub async fn cancel(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<VestingView> {
    lifecycle(st, h, id, true).await
}

// ---- employee and managers ---------------------------------------------------------

pub async fn mine(State(st): State<AppState>, h: HeaderMap) -> ApiResult<Vec<VestingView>> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let v = with_db(&st, move |db| db.list_my_vesting(&a)).await?;
    Ok(Json(v.iter().map(VestingView::from).collect()))
}

pub async fn detail(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<VestingDetailView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let a2 = a.clone();
    let d = with_db(&st, move |db| db.vesting_detail(&a2, &id)).await?;
    Ok(Json(detail_view(&st, &a, &d)?))
}

/// The agreement for its employee, or a 403 for anyone else who may view it.
async fn employee_detail(st: &AppState, actor: &str, id: &str) -> Result<VestingDetail, ApiError> {
    let (a2, id2) = (actor.to_string(), id.to_string());
    let d = with_db(st, move |db| db.vesting_detail(&a2, &id2)).await?;
    if d.listing.agreement.employee_user_id != actor {
        return Err(err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Only the employee can do that.",
        ));
    }
    Ok(d)
}

/// Sets the agreement's payout address, once. The wallet validates it first.
pub async fn set_address(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<AddressReq>,
) -> Result<(StatusCode, Json<VestingDetailView>), ApiError> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let po = enabled(&st)?;
    let address = clean_address(&r.address)?;
    // Only the employee gets as far as the wallet (no address probing by
    // others), and an already-locked agreement answers without it.
    let d = employee_detail(&st, &a, &id).await?;
    let fingerprint = recipient_fingerprint(&id, &address);
    if !d.address_locked {
        let (po2, addr2) = (po.clone(), address.clone());
        let q = blocking(move || po2.wallet.quote(&addr2, 1, None).map_err(wallet_err)).await?;
        if !q.valid_recipient {
            return Err(err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_recipient",
                q.reason
                    .unwrap_or_else(|| "Enter a shielded Zcash testnet Unified Address.".into()),
            ));
        }
    }
    let sealed = po.sealer.seal(&seal_id(&id), &address).map_err(|e| {
        tracing::error!("sealing payout address failed: {e}");
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    })?;
    let a2 = a.clone();
    let (d, dup) = with_db(&st, move |db| {
        let (_, dup) = db.set_vesting_payout_address(&a2, &id, &fingerprint, &sealed)?;
        Ok((db.vesting_detail(&a2, &id)?, dup))
    })
    .await?;
    let status = if dup {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(detail_view(&st, &a, &d)?)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteReq {
    amount_zec: String,
}

/// What withdrawing `amount_zec` now would cost: the network fee (paid by
/// the organization's payout wallet) and whether that wallet can cover it.
/// Nothing is reserved.
pub async fn quote(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<QuoteReq>,
) -> ApiResult<QuoteView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let po = enabled(&st)?;
    let amount = parse_amount("amount_zec", &r.amount_zec)?;
    let d = employee_detail(&st, &a, &id).await?;
    let org = d.listing.agreement.organization_id.clone();
    if !po.orgs.allows(&org) {
        return Err(err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Payouts are not enabled for this organization.",
        ));
    }
    let x = d.address.as_ref().ok_or_else(|| {
        err(
            StatusCode::CONFLICT,
            "invalid_state",
            "Set your payout address before withdrawing.",
        )
    })?;
    if d.open_payout().is_some() {
        return Err(err(
            StatusCode::CONFLICT,
            "invalid_state",
            "A withdrawal is already in progress.",
        ));
    }
    let available = d.listing.balance.available;
    if amount > available {
        return Err(hushush_payroll::PayrollError::InsufficientAvailable {
            requested: amount,
            available,
        }
        .into());
    }
    let address = po
        .sealer
        .open(&seal_id(&id), &x.address_sealed)
        .map_err(|e| {
            tracing::error!(agreement_id = %id, "cannot open sealed payout address: {e}");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "The stored payout address could not be read.",
            )
        })?;
    let q = blocking(move || {
        po.wallet.ensure_account(&org).map_err(wallet_err)?;
        let q = po
            .wallet
            .quote(&address, amount, Some(&org))
            .map_err(wallet_err)?;
        Ok(quote_view("", amount, &q, &po.policy))
    })
    .await?;
    Ok(Json(q))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawReq {
    amount_zec: String,
    idempotency_key: String,
}

/// Withdraws `amount_zec` of vested compensation to the agreement's locked
/// address. Exactly-once: a repeat (same key and amount), or any request
/// while an earlier withdrawal is unfinished, returns that withdrawal.
pub async fn withdraw(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<WithdrawReq>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let po = enabled(&st)?;
    let need = po.policy.required_confirmations;
    let amount = parse_amount("amount_zec", &r.amount_zec)?;
    let d = employee_detail(&st, &a, &id).await?;
    let x = d.address.as_ref().ok_or_else(|| {
        err(
            StatusCode::CONFLICT,
            "invalid_state",
            "Set your payout address before withdrawing.",
        )
    })?;
    let address = po
        .sealer
        .open(&seal_id(&id), &x.address_sealed)
        .map_err(|e| {
            tracing::error!(agreement_id = %id, "cannot open sealed payout address: {e}");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "The stored payout address could not be read.",
            )
        })?;
    // Paid from the organization's own payout account.
    let org = d.listing.agreement.organization_id.clone();

    // Quote only a withdrawal that can happen; otherwise let the engine answer
    // (a repeat returns its payout; anything else gets the precise reason).
    let possible =
        d.open_payout().is_none() && amount <= d.listing.balance.available && po.orgs.allows(&org);
    let (fee, max_fee) = if possible {
        let (po2, addr2, org2) = (po.clone(), address.clone(), org.clone());
        let q = blocking(move || {
            po2.wallet.ensure_account(&org2).map_err(wallet_err)?;
            po2.wallet
                .quote(&addr2, amount, Some(&org2))
                .map_err(wallet_err)
        })
        .await?;
        let fee = q.fee_zats.unwrap_or_default();
        match (q.valid_recipient, q.sufficient, po.policy.max_fee_for(fee)) {
            (true, true, Some(m)) => (fee, m),
            _ => {
                // A concurrent tap may have just started this agreement's
                // withdrawal and spent the notes the quote needed: then this
                // is a repeat; answer with that withdrawal.
                let (a3, id3) = (a.clone(), id.clone());
                let now_d = with_db(&st, move |db| db.vesting_detail(&a3, &id3)).await?;
                if let Some(p) = now_d.open_payout() {
                    return Ok((
                        StatusCode::OK,
                        Json(json!({ "payout": PayoutView::new(p, need), "duplicate": true })),
                    ));
                }
                let v = quote_view("", amount, &q, &po.policy);
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    json!({
                        "error": "insufficient_wallet_balance",
                        "message": v.reason.clone().map(|r| format!("The company's payout wallet can't cover this withdrawal yet: {r}")).unwrap_or_else(|| "The company's payout wallet can't cover this withdrawal and its fee right now.".into()),
                        "quote": v,
                    }),
                ));
            }
        }
    } else {
        (0, 0)
    };

    let pool = st.pool.clone();
    let clock = st.clock.clone();
    let (p, duplicate) = blocking(move || {
        let conn = pool.get().map_err(|_| {
            err(
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                "Database unavailable.",
            )
        })?;
        let mut db = hushush_payroll::Payroll::with_client(crate::Pooled(conn), clock);
        let agreement_fp = recipient_fingerprint(&id, &address);
        let addr_fp = address.clone();
        let orgs = po.orgs.clone();
        let out = db
            .withdraw_vesting(
                &a,
                &VestingWithdrawTerms {
                    agreement_id: &id,
                    idempotency_key: &r.idempotency_key,
                    amount_zats: amount,
                    fee_estimate_zats: fee,
                    max_fee_zats: max_fee,
                    address_fingerprint: &agreement_fp,
                    lease_owner: &po.owner,
                    lease_secs: po.policy.submit_lease_secs,
                },
                &move |pid| recipient_fingerprint(pid, &addr_fp),
                &|o| orgs.allows(o),
            )
            .map_err(ApiError::from)?;
        if out.duplicate {
            return Ok((out.payout, true));
        }
        tracing::info!(payout_id = %out.payout.id, agreement_id = %id, amount_zats = out.payout.amount_zats, "vesting withdrawal reserved; submitting to wallet");
        let wallet = OrgWallet {
            wallet: &po.wallet,
            org: &org,
        };
        let p = submit_payout(&mut db, &wallet, &po.policy, &out.payout, &address)
            .map_err(ApiError::from)?;
        tracing::info!(payout_id = %p.id, state = p.state.as_str(), txid = ?p.txid, "vesting withdrawal submission recorded");
        Ok((p, false))
    })
    .await?;
    let status = if duplicate {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(json!({ "payout": PayoutView::new(&p, need), "duplicate": duplicate })),
    ))
}

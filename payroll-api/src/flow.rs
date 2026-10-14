//! Phase 5 flow endpoints: stream offers and worker links, the locked payout
//! address, and one-action withdrawals. Rules live in the engine
//! (`hushush_payroll::flow`); payouts are submitted and confirmed by the
//! unchanged Phase 3 machinery (`crate::payouts`).
//!
//! The payout address is accepted once from the worker, validated by the
//! payout wallet (testnet Unified Address with an Orchard receiver), sealed
//! (`crate::seal`) and stored. It is never logged; it is returned only to
//! the stream's own worker.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;

use hushush_payroll::flow::MILESTONES;
use hushush_payroll::money::format_zec;
use hushush_payroll::payout::{recipient_fingerprint, submit_payout};
use hushush_payroll::{
    FlowStream, NewOffer, OfferStatus, StreamOffer, StreamStatus, WithdrawTerms,
};

use crate::payouts::{
    AddressReq, PayoutView, blocking, clean_address, enabled, quote_view, wallet_err,
};
use crate::{ApiError, ApiResult, AppState, BalanceView, actor, err, require_actor, with_db};

// ---- views ---------------------------------------------------------------------

#[derive(Serialize)]
pub struct OfferView {
    id: String,
    organization_id: String,
    organization_name: String,
    worker_name: String,
    total_zats: i64,
    total_zec: String,
    start_at: i64,
    end_at: i64,
    created_at: i64,
    /// open | bound | cancelled
    status: OfferStatus,
    stream_id: Option<String>,
    cancelled_at: Option<i64>,
}

impl From<&StreamOffer> for OfferView {
    fn from(o: &StreamOffer) -> Self {
        Self {
            id: o.id.clone(),
            organization_id: o.organization_id.clone(),
            organization_name: o.organization_name.clone(),
            worker_name: o.worker_name.clone(),
            total_zats: o.total_zats,
            total_zec: format_zec(o.total_zats),
            start_at: o.start_at,
            end_at: o.end_at,
            created_at: o.created_at,
            status: o.status(),
            stream_id: o.stream_id.clone(),
            cancelled_at: o.cancelled_at,
        }
    }
}

/// What a worker link shows before sign-in: the terms only.
#[derive(Serialize)]
pub struct LinkPreview {
    organization_name: String,
    worker_name: String,
    total_zats: i64,
    total_zec: String,
    start_at: i64,
    end_at: i64,
    /// open | bound | cancelled
    status: OfferStatus,
}

#[derive(Serialize)]
pub struct FlowStreamView {
    id: String,
    offer_id: String,
    organization_id: String,
    organization_name: String,
    /// The name the employer gave the worker.
    worker_name: String,
    recipient_user_id: String,
    total_zats: i64,
    total_zec: String,
    start_at: i64,
    end_at: i64,
    /// active | ended (effective at `balance.as_of`)
    status: StreamStatus,
    ended_at: Option<i64>,
    balance: BalanceView,
    /// One milestone (20% of the total).
    milestone_zats: i64,
    /// What "Withdraw" would pay right now, if it is enabled.
    withdrawable_zats: Option<i64>,
    withdrawable_zec: Option<String>,
    address_locked: bool,
    /// The locked address; only for the stream's worker.
    address: Option<String>,
    payouts: Vec<PayoutView>,
    payouts_enabled: bool,
}

fn flow_view(st: &AppState, f: &FlowStream) -> Result<FlowStreamView, ApiError> {
    let po = st.payouts.as_ref();
    let need = po.map_or(0, |p| p.policy.required_confirmations);
    let enabled = po.is_some_and(|p| p.orgs.allows(&f.listing.stream.organization_id));
    let address = match (&f.address, po) {
        (Some(a), Some(p)) => Some(p.sealer.open(&a.stream_id, &a.address_sealed).map_err(
            |e| {
                tracing::error!(stream_id = %a.stream_id, "cannot open sealed payout address: {e}");
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "The stored payout address could not be read.",
                )
            },
        )?),
        _ => None,
    };
    let withdrawable = f.withdrawable_now().filter(|_| enabled);
    let s = &f.listing.stream;
    Ok(FlowStreamView {
        id: s.id.clone(),
        offer_id: f.offer.id.clone(),
        organization_id: s.organization_id.clone(),
        organization_name: f.listing.organization_name.clone(),
        worker_name: f.offer.worker_name.clone(),
        recipient_user_id: s.recipient_user_id.clone(),
        total_zats: f.offer.total_zats,
        total_zec: format_zec(f.offer.total_zats),
        start_at: f.offer.start_at,
        end_at: f.offer.end_at,
        status: f.listing.balance.status,
        ended_at: s.ended_at,
        balance: (&f.listing.balance).into(),
        milestone_zats: (f.offer.total_zats + MILESTONES - 1) / MILESTONES,
        withdrawable_zats: withdrawable,
        withdrawable_zec: withdrawable.map(format_zec),
        address_locked: f.address_locked,
        address,
        payouts: f
            .payouts
            .iter()
            .rev()
            .map(|p| PayoutView::new(p, need))
            .collect(),
        payouts_enabled: enabled,
    })
}

// ---- employer ------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewOfferReq {
    worker_name: String,
    total_zec: String,
    start_at: Option<i64>,
    end_at: i64,
    /// SHA-256 (hex) of the worker-link token; the token never reaches us.
    link_hash: String,
}

pub async fn create_offer(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(org): Path<String>,
    Json(r): Json<NewOfferReq>,
) -> Result<(StatusCode, Json<OfferView>), ApiError> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let total = crate::parse_amount("total_zec", &r.total_zec)?;
    let o = with_db(&st, move |db| {
        db.create_offer(
            &a,
            NewOffer {
                organization_id: org,
                worker_name: r.worker_name,
                total_zats: total,
                start_at: r.start_at,
                end_at: r.end_at,
            },
            &r.link_hash,
        )
    })
    .await?;
    Ok((StatusCode::CREATED, Json((&o).into())))
}

#[derive(Serialize)]
pub struct EmployerHome {
    /// Offers no worker has bound yet (open or cancelled), newest first.
    offers: Vec<OfferView>,
    /// Streams with a worker, newest first.
    streams: Vec<FlowStreamView>,
    payouts_enabled: bool,
}

pub async fn employer_home(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(org): Path<String>,
) -> ApiResult<EmployerHome> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let o2 = org.clone();
    let (offers, streams) = with_db(&st, move |db| {
        Ok((db.list_offers(&a, &o2)?, db.employer_streams(&a, &o2)?))
    })
    .await?;
    Ok(Json(EmployerHome {
        offers: offers
            .iter()
            .filter(|o| o.status() != OfferStatus::Bound)
            .map(OfferView::from)
            .collect(),
        streams: streams
            .iter()
            .map(|f| flow_view(&st, f))
            .collect::<Result<_, _>>()?,
        payouts_enabled: st.payouts.as_ref().is_some_and(|p| p.orgs.allows(&org)),
    }))
}

pub async fn cancel_offer(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<OfferView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let o = with_db(&st, move |db| db.cancel_offer(&a, &id)).await?;
    Ok(Json((&o).into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelinkReq {
    link_hash: String,
}

pub async fn relink_offer(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<RelinkReq>,
) -> ApiResult<OfferView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let o = with_db(&st, move |db| db.relink_offer(&a, &id, &r.link_hash)).await?;
    Ok(Json((&o).into()))
}

// ---- worker link -----------------------------------------------------------------

/// Service-authenticated only: the preview is shown before sign-in.
pub async fn link_preview(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(link_hash): Path<String>,
) -> ApiResult<LinkPreview> {
    actor(&st, &h)?;
    let o = with_db(&st, move |db| db.offer_preview(&link_hash)).await?;
    Ok(Json(LinkPreview {
        organization_name: o.organization_name.clone(),
        worker_name: o.worker_name.clone(),
        total_zats: o.total_zats,
        total_zec: format_zec(o.total_zats),
        start_at: o.start_at,
        end_at: o.end_at,
        status: o.status(),
    }))
}

pub async fn bind_link(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(link_hash): Path<String>,
) -> ApiResult<serde_json::Value> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let b = with_db(&st, move |db| db.bind_offer(&a, &link_hash)).await?;
    Ok(Json(
        json!({ "stream_id": b.stream.id, "duplicate": b.duplicate }),
    ))
}

// ---- worker ----------------------------------------------------------------------

pub async fn worker_streams(
    State(st): State<AppState>,
    h: HeaderMap,
) -> ApiResult<Vec<FlowStreamView>> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let v = with_db(&st, move |db| db.worker_streams(&a)).await?;
    Ok(Json(
        v.iter()
            .map(|f| flow_view(&st, f))
            .collect::<Result<_, _>>()?,
    ))
}

pub async fn flow_stream(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<FlowStreamView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let f = with_db(&st, move |db| db.flow_stream(&a, &id)).await?;
    Ok(Json(flow_view(&st, &f)?))
}

/// Sets the stream's payout address, once. The wallet validates it first.
pub async fn set_address(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<AddressReq>,
) -> Result<(StatusCode, Json<FlowStreamView>), ApiError> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let po = enabled(&st)?;
    let address = clean_address(&r.address)?;
    // Only the worker gets as far as the wallet (no address probing by others),
    // and an already-locked stream answers without it.
    let (a2, id2) = (a.clone(), id.clone());
    let f = with_db(&st, move |db| db.flow_stream(&a2, &id2)).await?;
    if f.listing.stream.recipient_user_id != a {
        return Err(err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Only the worker can set the payout address.",
        ));
    }
    let fingerprint = recipient_fingerprint(&id, &address);
    if !f.address_locked {
        let (po2, addr2) = (po.clone(), address.clone());
        let q = blocking(move || po2.wallet.quote(&addr2, 1).map_err(wallet_err)).await?;
        if !q.valid_recipient {
            return Err(err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_recipient",
                q.reason
                    .unwrap_or_else(|| "Enter a shielded Zcash testnet Unified Address.".into()),
            ));
        }
    }
    let sealed = po.sealer.seal(&id, &address).map_err(|e| {
        tracing::error!("sealing payout address failed: {e}");
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    })?;
    let (a2, id2) = (a.clone(), id.clone());
    let (f, dup) = with_db(&st, move |db| {
        let (_, dup) = db.set_payout_address(&a2, &id2, &fingerprint, &sealed)?;
        Ok((db.flow_stream(&a2, &id2)?, dup))
    })
    .await?;
    let status = if dup {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(flow_view(&st, &f)?)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawReq {
    idempotency_key: String,
}

/// "Withdraw": pays everything available to the stream's locked address.
/// Exactly-once: a repeat (same key), or any request while an earlier
/// withdrawal is unfinished, returns that withdrawal's payout.
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
    let (a2, id2) = (a.clone(), id.clone());
    let f = with_db(&st, move |db| db.flow_stream(&a2, &id2)).await?;
    if f.listing.stream.recipient_user_id != a {
        return Err(err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Only the worker can withdraw.",
        ));
    }
    let addr = f.address.as_ref().ok_or_else(|| {
        err(
            StatusCode::CONFLICT,
            "invalid_state",
            "Set your payout address before withdrawing.",
        )
    })?;
    let address = po.sealer.open(&id, &addr.address_sealed).map_err(|e| {
        tracing::error!(stream_id = %id, "cannot open sealed payout address: {e}");
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The stored payout address could not be read.",
        )
    })?;
    let amount = f.listing.balance.available;

    // Quote only a withdrawal that can happen; otherwise let the engine answer
    // (a repeat returns its payout; anything else gets the precise reason).
    let (fee, max_fee) = if f.withdrawable_now().is_some() {
        let (po2, addr2) = (po.clone(), address.clone());
        let q = blocking(move || po2.wallet.quote(&addr2, amount).map_err(wallet_err)).await?;
        let fee = q.fee_zats.unwrap_or_default();
        match (q.valid_recipient, q.sufficient, po.policy.max_fee_for(fee)) {
            (true, true, Some(m)) => (fee, m),
            _ => {
                let v = quote_view("", amount, &q, &po.policy);
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    json!({
                        "error": "insufficient_wallet_balance",
                        "message": v.reason.clone().unwrap_or_else(|| "The payout wallet cannot cover this withdrawal and its fee right now.".into()),
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
            err(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable", "Database unavailable.")
        })?;
        let mut db = hushush_payroll::Payroll::with_client(crate::Pooled(conn), clock);
        let stream_fp = recipient_fingerprint(&id, &address);
        let addr_fp = address.clone();
        let orgs = po.orgs.clone();
        let out = db
            .withdraw(
                &a,
                &WithdrawTerms {
                    stream_id: &id,
                    idempotency_key: &r.idempotency_key,
                    amount_zats: amount,
                    fee_estimate_zats: fee,
                    max_fee_zats: max_fee,
                    address_fingerprint: &stream_fp,
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
        tracing::info!(payout_id = %out.payout.id, stream_id = %id, amount_zats = out.payout.amount_zats, "withdrawal reserved; submitting to wallet");
        let p = submit_payout(&mut db, &po.wallet, &po.policy, &out.payout, &address)
            .map_err(ApiError::from)?;
        tracing::info!(payout_id = %p.id, state = p.state.as_str(), txid = ?p.txid, "withdrawal submission recorded");
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

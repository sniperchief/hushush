//! `payroll-api` — internal HTTP front for the hushush payroll engine.
//!
//! * Listens on `PAYROLL_API_ADDR` (default `127.0.0.1:7878`). It is meant to
//!   be reachable only by the web app's server, never by browsers.
//! * Every `/v1` request must carry `Authorization: Bearer $PAYROLL_API_TOKEN`
//!   and `X-Actor-Id: <user id>` — the user the web app authenticated. All
//!   authorization decisions are made by the engine from that actor.
//! * Storage is PostgreSQL (`PAYROLL_DATABASE_URL`); migrations run at start.
//! * There is deliberately no endpoint to settle (mark paid) a claim by hand.
//!   A claim becomes paid only when its shielded payout is confirmed on chain
//!   (Phase 3, `payouts` module), and payouts are enabled only when the
//!   payout wallet service is configured (`WALLET_URL`).

mod payouts;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use subtle::ConstantTimeEq;

use hushush_payroll::clock::{Clock, SystemClock};
use hushush_payroll::money::{format_zec, parse_zec};
use hushush_payroll::pg::{PgBackend, connect_client};
use hushush_payroll::{
    Balance, Claim, ClaimStatus, Member, Membership, NewStream, Payroll, PayrollError, Role,
    StreamEvent, StreamListing, StreamStatus,
};

type Pool = r2d2::Pool<PgManager>;

/// Adapts a pooled connection to the engine's `BorrowMut<Client>` bound.
struct Pooled(r2d2::PooledConnection<PgManager>);

impl std::borrow::Borrow<postgres::Client> for Pooled {
    fn borrow(&self) -> &postgres::Client {
        &self.0
    }
}
impl std::borrow::BorrowMut<postgres::Client> for Pooled {
    fn borrow_mut(&mut self) -> &mut postgres::Client {
        &mut self.0
    }
}

/// r2d2 manager built on the engine's own connector (plain or rustls TLS).
struct PgManager {
    url: String,
    schema: Option<String>,
}

impl r2d2::ManageConnection for PgManager {
    type Connection = postgres::Client;
    type Error = PayrollError;
    fn connect(&self) -> Result<postgres::Client, PayrollError> {
        let mut c = connect_client(&self.url)?;
        if let Some(s) = &self.schema {
            hushush_payroll::pg::use_schema(&mut c, s)?;
        }
        Ok(c)
    }
    fn is_valid(&self, c: &mut postgres::Client) -> Result<(), PayrollError> {
        c.simple_query("SELECT 1")?;
        Ok(())
    }
    fn has_broken(&self, c: &mut postgres::Client) -> bool {
        c.is_closed()
    }
}

#[derive(Clone)]
struct AppState {
    pool: Pool,
    token: Arc<Vec<u8>>,
    clock: Arc<dyn Clock>,
    /// `None` when payouts are disabled (no `WALLET_URL`).
    payouts: Option<Arc<payouts::Payouts>>,
}

// ---- errors ------------------------------------------------------------------

struct ApiError(StatusCode, serde_json::Value);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

fn err(status: StatusCode, code: &str, message: impl Into<String>) -> ApiError {
    ApiError(status, json!({ "error": code, "message": message.into() }))
}

impl From<PayrollError> for ApiError {
    fn from(e: PayrollError) -> Self {
        use PayrollError::*;
        match e {
            Validation(m) => err(StatusCode::UNPROCESSABLE_ENTITY, "validation", m),
            Forbidden(m) => err(StatusCode::FORBIDDEN, "forbidden", m),
            NotFound(what) => err(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("{what} not found"),
            ),
            InvalidState(m) => err(StatusCode::CONFLICT, "invalid_state", m),
            InsufficientAvailable {
                requested,
                available,
            } => ApiError(
                StatusCode::CONFLICT,
                json!({
                    "error": "insufficient_available",
                    "message": format!(
                        "requested {} ZEC but only {} ZEC is available",
                        format_zec(requested),
                        format_zec(available)
                    ),
                    "requested_zats": requested,
                    "available_zats": available,
                }),
            ),
            IdempotencyConflict => err(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "this request key was already used for a different claim",
            ),
            Conflict(m) => err(StatusCode::CONFLICT, "conflict", m),
            ClockSkew => err(
                StatusCode::SERVICE_UNAVAILABLE,
                "clock_skew",
                "server clock skew; retry later",
            ),
            other => {
                tracing::error!("internal error: {other}");
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error",
                )
            }
        }
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

// ---- auth & engine access ------------------------------------------------------

fn actor(state: &AppState, headers: &HeaderMap) -> Result<String, ApiError> {
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !bool::from(presented.as_bytes().ct_eq(state.token.as_slice())) {
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid service token",
        ));
    }
    let actor = headers
        .get("x-actor-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    Ok(actor)
}

fn require_actor(a: &str) -> Result<(), ApiError> {
    if a.is_empty() {
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing X-Actor-Id",
        ));
    }
    Ok(())
}

/// Runs a blocking engine operation on a pooled connection.
async fn with_db<T: Send + 'static>(
    state: &AppState,
    f: impl FnOnce(&mut Payroll<PgBackend<Pooled>>) -> hushush_payroll::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    let pool = state.pool.clone();
    let clock = state.clock.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool
            .get()
            .map_err(|e| PayrollError::InvalidState(format!("database unavailable: {e}")))?;
        let mut db = Payroll::with_client(Pooled(conn), clock);
        f(&mut db)
    })
    .await
    .map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "worker panicked",
        )
    })?
    .map_err(ApiError::from)
}

// ---- views ---------------------------------------------------------------------

#[derive(Serialize)]
struct BalanceView {
    as_of: i64,
    earned_zats: i64,
    earned_zec: String,
    reserved_zats: i64,
    reserved_zec: String,
    claimed_zats: i64,
    claimed_zec: String,
    available_zats: i64,
    available_zec: String,
}

impl From<&Balance> for BalanceView {
    fn from(b: &Balance) -> Self {
        Self {
            as_of: b.as_of,
            earned_zats: b.earned,
            earned_zec: format_zec(b.earned),
            reserved_zats: b.reserved,
            reserved_zec: format_zec(b.reserved),
            claimed_zats: b.claimed,
            claimed_zec: format_zec(b.claimed),
            available_zats: b.available,
            available_zec: format_zec(b.available),
        }
    }
}

#[derive(Serialize)]
struct StreamView {
    id: String,
    organization_id: String,
    organization_name: String,
    recipient_user_id: String,
    recipient_display_name: String,
    recipient_email: String,
    created_by: String,
    rate_zats: i64,
    rate_zec: String,
    rate_period_secs: i64,
    start_at: i64,
    end_at: Option<i64>,
    /// Effective status as of `balance.as_of` (active past its end date = ended).
    status: StreamStatus,
    ended_at: Option<i64>,
    created_at: i64,
    balance: BalanceView,
}

impl From<&StreamListing> for StreamView {
    fn from(l: &StreamListing) -> Self {
        let s = &l.stream;
        Self {
            id: s.id.clone(),
            organization_id: s.organization_id.clone(),
            organization_name: l.organization_name.clone(),
            recipient_user_id: s.recipient_user_id.clone(),
            recipient_display_name: l.recipient_display_name.clone(),
            recipient_email: l.recipient_email.clone(),
            created_by: s.created_by.clone(),
            rate_zats: s.accrual.rate_zats,
            rate_zec: format_zec(s.accrual.rate_zats),
            rate_period_secs: s.accrual.rate_period_secs,
            start_at: s.accrual.start_at,
            end_at: s.accrual.end_at,
            status: l.balance.status,
            ended_at: s.ended_at,
            created_at: s.created_at,
            balance: (&l.balance).into(),
        }
    }
}

#[derive(Serialize)]
struct ClaimView {
    id: String,
    stream_id: String,
    requested_by: String,
    amount_zats: i64,
    amount_zec: String,
    /// `reserved` = pending and UNPAID, even while a payout is in flight.
    /// `settled` = paid: its payout was confirmed on chain.
    status: ClaimStatus,
    created_at: i64,
    resolved_at: Option<i64>,
    settlement_ref: Option<String>,
    /// The claim's shielded payout, if one was requested.
    payout: Option<payouts::PayoutView>,
}

impl From<&Claim> for ClaimView {
    fn from(c: &Claim) -> Self {
        Self {
            id: c.id.clone(),
            stream_id: c.stream_id.clone(),
            requested_by: c.requested_by.clone(),
            amount_zats: c.amount_zats,
            amount_zec: format_zec(c.amount_zats),
            status: c.status,
            created_at: c.created_at,
            resolved_at: c.resolved_at,
            settlement_ref: c.settlement_ref.clone(),
            payout: None,
        }
    }
}

#[derive(Serialize)]
struct StreamDetail {
    stream: StreamView,
    events: Vec<StreamEvent>,
    claims: Vec<ClaimView>,
    /// Whether this stream's claims can be paid out from this server.
    payouts_enabled: bool,
}

// ---- requests ------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewUserReq {
    email: String,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewOrgReq {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddMemberReq {
    user_id: String,
    role: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleReq {
    role: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewStreamReq {
    recipient_user_id: String,
    /// Decimal ZEC per period, e.g. "1.5". Parsed exactly into zatoshis.
    rate_zec: String,
    rate_period_secs: i64,
    start_at: Option<i64>,
    end_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimReq {
    /// Decimal ZEC, e.g. "0.25". Parsed exactly into zatoshis.
    amount_zec: String,
    idempotency_key: String,
}

fn parse_role(s: &str) -> Result<Role, ApiError> {
    Role::parse(s).ok_or_else(|| {
        err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation",
            "role must be owner, admin or member",
        )
    })
}

fn parse_amount(field: &str, s: &str) -> Result<i64, ApiError> {
    match parse_zec(s) {
        Some(z) if z > 0 => Ok(z),
        _ => Err(err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation",
            format!("{field} must be a positive ZEC amount with at most 8 decimal places"),
        )),
    }
}

// ---- handlers ------------------------------------------------------------------

async fn healthz() -> &'static str {
    "ok"
}

async fn create_user(
    State(st): State<AppState>,
    h: HeaderMap,
    Json(r): Json<NewUserReq>,
) -> ApiResult<serde_json::Value> {
    actor(&st, &h)?; // service-authenticated; sign-up has no actor yet
    let id = with_db(&st, move |db| db.create_user(&r.email, &r.display_name)).await?;
    Ok(Json(json!({ "id": id })))
}

async fn list_orgs(State(st): State<AppState>, h: HeaderMap) -> ApiResult<Vec<Membership>> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    Ok(Json(
        with_db(&st, move |db| db.list_organizations(&a)).await?,
    ))
}

async fn create_org(
    State(st): State<AppState>,
    h: HeaderMap,
    Json(r): Json<NewOrgReq>,
) -> ApiResult<serde_json::Value> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let id = with_db(&st, move |db| db.create_organization(&a, &r.name)).await?;
    Ok(Json(json!({ "id": id })))
}

async fn list_members(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(org): Path<String>,
) -> ApiResult<Vec<Member>> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    Ok(Json(
        with_db(&st, move |db| db.list_members(&a, &org)).await?,
    ))
}

async fn add_member(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(org): Path<String>,
    Json(r): Json<AddMemberReq>,
) -> ApiResult<serde_json::Value> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let role = parse_role(&r.role)?;
    with_db(&st, move |db| db.add_member(&a, &org, &r.user_id, role)).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn set_role(
    State(st): State<AppState>,
    h: HeaderMap,
    Path((org, user)): Path<(String, String)>,
    Json(r): Json<RoleReq>,
) -> ApiResult<serde_json::Value> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let role = parse_role(&r.role)?;
    with_db(&st, move |db| db.set_member_role(&a, &org, &user, role)).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn remove_member(
    State(st): State<AppState>,
    h: HeaderMap,
    Path((org, user)): Path<(String, String)>,
) -> ApiResult<serde_json::Value> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    with_db(&st, move |db| db.remove_member(&a, &org, &user)).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn list_org_streams(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(org): Path<String>,
) -> ApiResult<Vec<StreamView>> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let v = with_db(&st, move |db| db.list_org_streams(&a, &org)).await?;
    Ok(Json(v.iter().map(StreamView::from).collect()))
}

async fn create_stream(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(org): Path<String>,
    Json(r): Json<NewStreamReq>,
) -> ApiResult<StreamView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let rate_zats = parse_amount("rate_zec", &r.rate_zec)?;
    let listing = with_db(&st, move |db| {
        let s = db.create_stream(
            &a,
            NewStream {
                organization_id: org,
                recipient_user_id: r.recipient_user_id,
                rate_zats,
                rate_period_secs: r.rate_period_secs,
                start_at: r.start_at,
                end_at: r.end_at,
            },
        )?;
        db.stream_listing(&a, &s.id)
    })
    .await?;
    Ok(Json((&listing).into()))
}

async fn my_streams(State(st): State<AppState>, h: HeaderMap) -> ApiResult<Vec<StreamView>> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let v = with_db(&st, move |db| db.list_my_streams(&a)).await?;
    Ok(Json(v.iter().map(StreamView::from).collect()))
}

async fn stream_detail(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StreamDetail> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let (l, events, claims, payouts) = with_db(&st, move |db| {
        Ok((
            db.stream_listing(&a, &id)?,
            db.stream_events(&a, &id)?,
            db.list_claims(&a, &id)?,
            db.list_payouts(&a, &id)?,
        ))
    })
    .await?;
    let need = st
        .payouts
        .as_ref()
        .map_or(0, |p| p.policy.required_confirmations);
    let by_claim: HashMap<&str, &hushush_payroll::Payout> =
        payouts.iter().map(|p| (p.claim_id.as_str(), p)).collect();
    Ok(Json(StreamDetail {
        stream: (&l).into(),
        events,
        claims: claims
            .iter()
            .map(|c| {
                let mut v = ClaimView::from(c);
                v.payout = by_claim
                    .get(c.id.as_str())
                    .map(|p| payouts::PayoutView::new(p, need));
                v
            })
            .collect(),
        payouts_enabled: st
            .payouts
            .as_ref()
            .is_some_and(|p| p.orgs.allows(&l.stream.organization_id)),
    }))
}

async fn lifecycle(
    st: AppState,
    h: HeaderMap,
    id: String,
    op: &'static str,
) -> ApiResult<StreamView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let l = with_db(&st, move |db| {
        match op {
            "pause" => db.pause_stream(&a, &id)?,
            "resume" => db.resume_stream(&a, &id)?,
            _ => db.end_stream(&a, &id)?,
        };
        db.stream_listing(&a, &id)
    })
    .await?;
    Ok(Json((&l).into()))
}

async fn pause(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StreamView> {
    lifecycle(st, h, id, "pause").await
}
async fn resume(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StreamView> {
    lifecycle(st, h, id, "resume").await
}
async fn end(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StreamView> {
    lifecycle(st, h, id, "end").await
}

async fn reserve_claim(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<ClaimReq>,
) -> ApiResult<serde_json::Value> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let amount = parse_amount("amount_zec", &r.amount_zec)?;
    let out = with_db(&st, move |db| {
        db.reserve_claim(&a, &id, amount, &r.idempotency_key)
    })
    .await?;
    Ok(Json(
        json!({ "claim": ClaimView::from(&out.claim), "duplicate": out.duplicate }),
    ))
}

async fn release_claim(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<ClaimView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let c = with_db(&st, move |db| db.release_claim(&a, &id)).await?;
    Ok(Json((&c).into()))
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/users", post(create_user))
        .route("/v1/orgs", get(list_orgs).post(create_org))
        .route("/v1/orgs/{org}/members", get(list_members).post(add_member))
        .route(
            "/v1/orgs/{org}/members/{user}",
            patch(set_role).delete(remove_member),
        )
        .route(
            "/v1/orgs/{org}/streams",
            get(list_org_streams).post(create_stream),
        )
        .route("/v1/me/streams", get(my_streams))
        .route("/v1/streams/{id}", get(stream_detail))
        .route("/v1/streams/{id}/pause", post(pause))
        .route("/v1/streams/{id}/resume", post(resume))
        .route("/v1/streams/{id}/end", post(end))
        .route("/v1/streams/{id}/claims", post(reserve_claim))
        .route("/v1/claims/{id}/release", post(release_claim))
        .route("/v1/claims/{id}/payout/quote", post(payouts::quote))
        .route("/v1/claims/{id}/payout", post(payouts::request))
        .route("/v1/orgs/{org}/payout-wallet", get(payouts::wallet_summary))
        .with_state(state)
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let url =
        std::env::var("PAYROLL_DATABASE_URL").map_err(|_| "PAYROLL_DATABASE_URL is required")?;
    let token = std::env::var("PAYROLL_API_TOKEN").map_err(|_| "PAYROLL_API_TOKEN is required")?;
    if token.len() < 32 {
        return Err("PAYROLL_API_TOKEN must be at least 32 characters".into());
    }
    let addr: SocketAddr = std::env::var("PAYROLL_API_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:7878".into())
        .parse()?;
    // It trusts X-Actor-Id from anyone holding the service token: keep it off
    // public interfaces unless explicitly placed behind a private network.
    if !addr.ip().is_loopback() && std::env::var("PAYROLL_API_ALLOW_NONLOCAL").as_deref() != Ok("1")
    {
        return Err(format!(
            "refusing to listen on non-loopback {addr}; set PAYROLL_API_ALLOW_NONLOCAL=1 only on a private network"
        )
        .into());
    }
    let schema = std::env::var("PAYROLL_DATABASE_SCHEMA")
        .ok()
        .filter(|s| !s.is_empty());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

    // Migrate once at start-up (advisory-locked, safe with several instances).
    {
        let (url, schema, now) = (url.clone(), schema.clone(), clock.now());
        tokio::task::spawn_blocking(move || -> hushush_payroll::Result<Vec<i64>> {
            let mut c = connect_client(&url)?;
            if let Some(s) = &schema {
                hushush_payroll::pg::use_schema(&mut c, s)?;
            }
            hushush_payroll::migrations::migrate_pg(&mut c, now)
        })
        .await??;
    }

    let pool = r2d2::Pool::builder()
        .max_size(16)
        .build(PgManager { url, schema })?;
    let payouts = payouts::Payouts::from_env()?.map(Arc::new);
    let state = AppState {
        pool,
        token: Arc::new(token.into_bytes()),
        clock,
        payouts: payouts.clone(),
    };
    match &payouts {
        Some(po) => {
            tracing::info!(
                confirmations = po.policy.required_confirmations,
                "payouts ENABLED (testnet payout wallet configured)"
            );
            payouts::spawn_worker(state.clone(), po.clone());
        }
        None => tracing::info!("payouts disabled (WALLET_URL not set)"),
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("payroll-api listening on {addr}");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

//! The narrow HTTP interface of the payout wallet. Every `/v1` route needs
//! `Authorization: Bearer $WALLET_API_TOKEN`; only payroll-api holds it.
//!
//! | Route | Purpose |
//! |---|---|
//! | `GET  /v1/status` | balances, sync state, funding address |
//! | `POST /v1/recipients/validate` | payout address policy check |
//! | `POST /v1/quotes` | real-proposal fee + balance sufficiency |
//! | `POST /v1/payments` | pay (idempotent per `payment_id`) |
//! | `GET  /v1/payments/{id}` | payment state + chain status + verification |
//! | `POST /v1/payments/{id}/cancel` | fence an id the wallet never saw |
//! | `GET  /v1/transactions/{txid}` | keyless observer view + confirmations |
//! | `POST /v1/accounts` | create (or find) a company's payout account |
//! | `GET  /v1/accounts/{key}` | that account's balance, funding address, faucet |
//! | `POST /v1/accounts/{key}/faucet` | testnet top-up from the treasury |
//!
//! Quotes and payments take an optional `account` (a company's key); without
//! it they use the treasury.
//!
//! Addresses arrive only in request bodies and are never logged.

use std::sync::{Arc, Mutex};

use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, oneshot};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId};

use hushush_wallet_proof::{NETWORK, observer, remote, wallet};

use crate::actor::{AccountStatusView, AccountView, Cmd, CmdError, SharedUfvks, SubmitReq};
use crate::chain::{self, Fetched};
use crate::intents::{Intent, IntentState, IntentStore};
use crate::policy::{self, TxVerification};

#[derive(Clone)]
pub struct AppState {
    pub token: Arc<Vec<u8>>,
    pub actor: mpsc::Sender<Cmd>,
    pub reader: Arc<Mutex<IntentStore>>,
    pub client: remote::Client,
    /// Every account's viewing key (updated when an account is created).
    pub ufvks: SharedUfvks,
    /// The wallet DB, opened read-only per lookup for the wallet's own view.
    pub wallet_db: std::path::PathBuf,
    pub chain_ok: Arc<std::sync::atomic::AtomicBool>,
}

/// The wallet's own scanned view of a transaction (compact-block scanning,
/// independent of `GetTransaction`): (mined height, expired unmined).
fn wallet_view(
    db: &std::path::Path,
    txid: &zcash_primitives::transaction::TxId,
) -> Option<(Option<i64>, bool)> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .ok()?;
    c.query_row(
        "SELECT mined_height, expired_unmined FROM v_transactions WHERE txid = ?1",
        [txid.as_ref().to_vec()],
        |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, Option<bool>>(1)?.unwrap_or(false),
            ))
        },
    )
    .ok()
}

pub struct ApiError(StatusCode, &'static str, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1, "message": self.2 }))).into_response()
    }
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    tracing::error!("internal error: {e}");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "internal error".into(),
    )
}

fn unavailable(e: impl std::fmt::Display) -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "chain_unavailable",
        e.to_string(),
    )
}

type ApiResult<T> = Result<Json<T>, ApiError>;

async fn auth(
    State(st): State<AppState>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Response {
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !bool::from(presented.as_bytes().ct_eq(st.token.as_slice())) {
        return ApiError(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid token".into(),
        )
        .into_response();
    }
    next.run(req).await
}

async fn ask<T>(
    st: &AppState,
    make: impl FnOnce(oneshot::Sender<Result<T, CmdError>>) -> Cmd,
) -> Result<T, ApiError> {
    let (tx, rx) = oneshot::channel();
    st.actor
        .send(make(tx))
        .await
        .map_err(|_| internal("wallet actor stopped"))?;
    match rx
        .await
        .map_err(|_| internal("wallet actor dropped a request"))?
    {
        Ok(v) => Ok(v),
        Err(CmdError::Conflict(m)) => {
            Err(ApiError(StatusCode::CONFLICT, "idempotency_conflict", m))
        }
        Err(CmdError::UnknownAccount(k)) => Err(ApiError(
            StatusCode::CONFLICT,
            "unknown_account",
            format!("no payout account is registered for {k:?}"),
        )),
        Err(CmdError::Refused(m)) => Err(ApiError(StatusCode::CONFLICT, "refused", m)),
        Err(CmdError::Internal(e)) => Err(internal(format!("{e:#}"))),
    }
}

// ---- views --------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ChainView {
    pub tip_height: i64,
    pub mined_height: Option<i64>,
    pub confirmations: i64,
    pub in_mempool: bool,
    /// From the wallet's own scanned chain state (None: not in the wallet DB).
    pub wallet_mined_height: Option<i64>,
    pub wallet_expired_unmined: Option<bool>,
}

#[derive(Serialize)]
pub struct PaymentView {
    pub payment_id: String,
    pub state: IntentState,
    pub recipient_fingerprint: String,
    pub amount_zats: i64,
    pub failure_reason: Option<String>,
    pub txid: Option<String>,
    pub fee_zats: Option<i64>,
    pub expiry_height: Option<i64>,
    pub chain: Option<ChainView>,
    pub verification: Option<TxVerification>,
    pub broadcast_attempts: i64,
    pub last_broadcast_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The payment record plus fresh chain status and verification. Chain errors
/// are reported as 503 so the caller waits instead of deciding.
async fn payment_view(st: &AppState, i: Intent) -> Result<PaymentView, ApiError> {
    let mut v = PaymentView {
        recipient_fingerprint: i.recipient_fingerprint(),
        payment_id: i.payment_id.clone(),
        state: i.state,
        amount_zats: i.amount_zats,
        failure_reason: i.failure_reason.clone(),
        txid: i.txid.clone(),
        fee_zats: i.fee_zats,
        expiry_height: i.expiry_height,
        chain: None,
        verification: None,
        broadcast_attempts: i.broadcast_attempts,
        last_broadcast_error: i.last_broadcast_error.clone(),
        created_at: i.created_at,
        updated_at: i.updated_at,
    };
    let (Some(txid_s), Some(address)) = (i.txid.as_deref(), i.address.as_deref()) else {
        return Ok(v);
    };
    let txid = wallet::parse_txid(txid_s).map_err(internal)?;
    if !st.chain_ok.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(unavailable("lightwalletd failed its network/branch check"));
    }
    let wv = wallet_view(&st.wallet_db, &txid);
    let ufvks = st
        .ufvks
        .read()
        .map_err(|_| internal("ufvk lock poisoned"))?
        .clone();
    let mut c = st.client.clone();
    let tip = remote::chain_tip(&mut c).await.map_err(unavailable)?;
    let tip_h = BlockHeight::from_u32(tip);
    let verify = |tx: &Transaction, len: usize, mined: Option<BlockHeight>, source: &str| {
        policy::verify_tx(
            tx,
            len,
            mined,
            tip_h,
            &ufvks,
            &i.payment_id,
            address,
            source,
        )
    };
    let (chain_view, verification) = match chain::fetch(&mut c, tip_h, txid)
        .await
        .map_err(unavailable)?
    {
        // The tip was read before the transaction; if a block landed in
        // between, the transaction's own height is the better tip.
        Fetched::Mined(tx, h, raw) => (
            ChainView {
                tip_height: u32::from(tip_h.max(h)).into(),
                mined_height: Some(u32::from(h).into()),
                confirmations: wallet::confirmations(Some(h), Some(tip_h.max(h))).into(),
                in_mempool: false,
                wallet_mined_height: wv.and_then(|w| w.0),
                wallet_expired_unmined: wv.map(|w| w.1),
            },
            verify(&tx, raw.len(), Some(h), "chain"),
        ),
        Fetched::Mempool(tx, raw) => (
            ChainView {
                tip_height: tip.into(),
                mined_height: None,
                confirmations: 0,
                in_mempool: true,
                wallet_mined_height: wv.and_then(|w| w.0),
                wallet_expired_unmined: wv.map(|w| w.1),
            },
            verify(&tx, raw.len(), None, "chain"),
        ),
        Fetched::Missing => {
            let raw = i.raw_tx.clone().unwrap_or_default();
            let tx = Transaction::read(&raw[..], BranchId::for_height(&NETWORK, tip_h + 1))
                .map_err(internal)?;
            (
                ChainView {
                    tip_height: tip.into(),
                    mined_height: None,
                    confirmations: 0,
                    in_mempool: false,
                    wallet_mined_height: wv.and_then(|w| w.0),
                    wallet_expired_unmined: wv.map(|w| w.1),
                },
                verify(&tx, raw.len(), None, "local"),
            )
        }
    };
    v.chain = Some(chain_view);
    v.verification = Some(verification);
    Ok(v)
}

fn check_payment_id(id: &str) -> Result<(), ApiError> {
    match uuid::Uuid::parse_str(id) {
        Ok(u) if u.hyphenated().to_string() == id => Ok(()),
        _ => Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation",
            "payment_id must be a lowercase hyphenated UUID".into(),
        )),
    }
}

// ---- handlers -----------------------------------------------------------------------

async fn healthz() -> &'static str {
    "ok"
}

async fn status(State(st): State<AppState>) -> ApiResult<crate::actor::StatusView> {
    Ok(Json(ask(&st, Cmd::Status).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddressReq {
    address: String,
}

async fn validate(Json(r): Json<AddressReq>) -> Json<serde_json::Value> {
    Json(match policy::payout_recipient(&r.address) {
        Ok(rec) => json!({ "valid": true, "has_sapling_receiver": rec.sapling.is_some() }),
        Err(reason) => json!({ "valid": false, "reason": reason }),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuoteReq {
    #[serde(default)]
    account: Option<String>,
    address: String,
    amount_zats: i64,
}

async fn quote(
    State(st): State<AppState>,
    Json(r): Json<QuoteReq>,
) -> ApiResult<crate::actor::QuoteView> {
    Ok(Json(
        ask(&st, |reply| Cmd::Quote {
            account: r.account,
            address: r.address,
            amount_zats: r.amount_zats,
            reply,
        })
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaymentReq {
    payment_id: String,
    #[serde(default)]
    account: Option<String>,
    address: String,
    amount_zats: i64,
    max_fee_zats: i64,
}

async fn submit(State(st): State<AppState>, Json(r): Json<PaymentReq>) -> ApiResult<PaymentView> {
    check_payment_id(&r.payment_id)?;
    if let Some(k) = &r.account {
        check_account_key(k)?;
    }
    let i = ask(&st, |reply| Cmd::Submit {
        req: SubmitReq {
            payment_id: r.payment_id,
            account: r.account,
            address: r.address,
            amount_zats: r.amount_zats,
            max_fee_zats: r.max_fee_zats,
        },
        reply,
    })
    .await?;
    Ok(Json(recorded_view(&st, i).await))
}

/// The payment is recorded; if the chain is momentarily unreachable, report
/// the record without chain status rather than an error.
async fn recorded_view(st: &AppState, i: Intent) -> PaymentView {
    let fallback = i.clone();
    match payment_view(st, i).await {
        Ok(v) => v,
        Err(_) => PaymentView {
            recipient_fingerprint: fallback.recipient_fingerprint(),
            payment_id: fallback.payment_id,
            state: fallback.state,
            amount_zats: fallback.amount_zats,
            failure_reason: fallback.failure_reason,
            txid: fallback.txid,
            fee_zats: fallback.fee_zats,
            expiry_height: fallback.expiry_height,
            chain: None,
            verification: None,
            broadcast_attempts: fallback.broadcast_attempts,
            last_broadcast_error: fallback.last_broadcast_error,
            created_at: fallback.created_at,
            updated_at: fallback.updated_at,
        },
    }
}

/// Account keys are opaque ids chosen by payroll-api (organization ids).
fn check_account_key(k: &str) -> Result<(), ApiError> {
    if (1..=64).contains(&k.len())
        && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Ok(())
    } else {
        Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation",
            "account key must be 1-64 characters of [A-Za-z0-9_-]".into(),
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountReq {
    key: String,
}

async fn ensure_account(
    State(st): State<AppState>,
    Json(r): Json<AccountReq>,
) -> ApiResult<AccountView> {
    check_account_key(&r.key)?;
    Ok(Json(
        ask(&st, |reply| Cmd::EnsureAccount { key: r.key, reply }).await?,
    ))
}

async fn account_status(
    State(st): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<AccountStatusView> {
    check_account_key(&key)?;
    Ok(Json(
        ask(&st, |reply| Cmd::AccountStatus { key, reply }).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FaucetReq {
    payment_id: String,
}

async fn faucet(
    State(st): State<AppState>,
    Path(key): Path<String>,
    Json(r): Json<FaucetReq>,
) -> ApiResult<PaymentView> {
    check_account_key(&key)?;
    check_payment_id(&r.payment_id)?;
    let i = ask(&st, |reply| Cmd::Faucet {
        key,
        payment_id: r.payment_id,
        reply,
    })
    .await?;
    Ok(Json(recorded_view(&st, i).await))
}

async fn lookup(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, ApiError> {
    check_payment_id(&id)?;
    let found = {
        let r = st
            .reader
            .lock()
            .map_err(|_| internal("intent store lock poisoned"))?;
        r.get(&id).map_err(internal)?
    };
    match found {
        None => Ok(ApiError(
            StatusCode::NOT_FOUND,
            "not_found",
            "unknown payment id".into(),
        )
        .into_response()),
        Some(i) => Ok(Json(payment_view(&st, i).await?).into_response()),
    }
}

async fn cancel(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<PaymentView> {
    check_payment_id(&id)?;
    let i = ask(&st, |reply| Cmd::Cancel {
        payment_id: id,
        reply,
    })
    .await?;
    Ok(Json(payment_view(&st, i).await?))
}

async fn transaction(
    State(st): State<AppState>,
    Path(txid): Path<String>,
) -> Result<Response, ApiError> {
    let id = wallet::parse_txid(&txid).map_err(|_| {
        ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation",
            "bad txid".into(),
        )
    })?;
    let mut c = st.client.clone();
    let tip = remote::chain_tip(&mut c).await.map_err(unavailable)?;
    let tip_h = BlockHeight::from_u32(tip);
    let body = match chain::fetch(&mut c, tip_h, id).await.map_err(unavailable)? {
        Fetched::Missing => json!({ "txid": txid, "found": false, "tip_height": tip }),
        Fetched::Mempool(tx, raw) => json!({
            "txid": txid, "found": true, "tip_height": tip, "in_mempool": true,
            "confirmations": 0, "observer": observer::observe(&tx, raw.len(), None),
        }),
        Fetched::Mined(tx, h, raw) => json!({
            "txid": txid, "found": true, "tip_height": tip, "in_mempool": false,
            "mined_height": u32::from(h),
            "confirmations": wallet::confirmations(Some(h), Some(tip_h)),
            "observer": observer::observe(&tx, raw.len(), Some(u32::from(h))),
        }),
    };
    Ok(Json(body).into_response())
}

pub fn app(state: AppState) -> Router {
    let v1 = Router::new()
        .route("/v1/status", get(status))
        .route("/v1/recipients/validate", post(validate))
        .route("/v1/quotes", post(quote))
        .route("/v1/payments", post(submit))
        .route("/v1/payments/{id}", get(lookup))
        .route("/v1/payments/{id}/cancel", post(cancel))
        .route("/v1/transactions/{txid}", get(transaction))
        .route("/v1/accounts", post(ensure_account))
        .route("/v1/accounts/{key}", get(account_status))
        .route("/v1/accounts/{key}/faucet", post(faucet))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));
    Router::new()
        .route("/healthz", get(healthz))
        .merge(v1)
        .with_state(state)
}

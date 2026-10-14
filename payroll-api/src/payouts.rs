//! Shielded payouts (Phase 3): configuration, the HTTP client for the payout
//! wallet service, the payout endpoints and the reconciliation worker.
//!
//! payroll-api never holds keys. It asks the payout wallet (a separate
//! process with its own token) to quote, pay and report; the payout state
//! machine itself lives in the engine (`hushush_payroll::payout`).
//!
//! The destination address is accepted from the claimant, passed to the
//! wallet, fingerprinted, and dropped. It is never stored here or logged.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;

use hushush_payroll::money::format_zec;
use hushush_payroll::payout::{
    PayoutPolicy, PayoutTerms, WalletApi, WalletError, WalletPayment, WalletSubmit,
    recipient_fingerprint, reconcile_due, submit_payout,
};
use hushush_payroll::{Payout, PayoutState, PayrollError};

use crate::{ApiError, ApiResult, AppState, actor, err, require_actor, with_db};

// ---- configuration -----------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum OrgAllow {
    All,
    Only(HashSet<String>),
}

impl OrgAllow {
    pub fn allows(&self, org: &str) -> bool {
        match self {
            OrgAllow::All => true,
            OrgAllow::Only(s) => s.contains(org),
        }
    }
}

pub struct Payouts {
    pub wallet: HttpWallet,
    /// Seals locked payout addresses (Phase 5); see `crate::seal`.
    pub sealer: crate::seal::AddressSealer,
    pub policy: PayoutPolicy,
    pub orgs: OrgAllow,
    /// Lease owner identity of this process.
    pub owner: String,
    pub poll: Duration,
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> Result<T, String> {
    match std::env::var(name) {
        Ok(v) => v
            .parse()
            .map_err(|_| format!("{name} has an invalid value")),
        Err(_) => Ok(default),
    }
}

impl Payouts {
    /// Payouts are enabled only when `WALLET_URL` is set.
    pub fn from_env() -> Result<Option<Self>, String> {
        let Ok(url) = std::env::var("WALLET_URL") else {
            return Ok(None);
        };
        let token =
            std::env::var("WALLET_API_TOKEN").map_err(|_| "WALLET_API_TOKEN is required")?;
        if token.len() < 32 {
            return Err("WALLET_API_TOKEN must be at least 32 characters".into());
        }
        let orgs =
            match std::env::var("PAYOUT_ENABLED_ORGS")
                .unwrap_or_default()
                .trim()
            {
                "" => return Err(
                    "PAYOUT_ENABLED_ORGS is required with WALLET_URL (org ids, or * on testnet)"
                        .into(),
                ),
                "*" => OrgAllow::All,
                list => OrgAllow::Only(list.split(',').map(|s| s.trim().to_string()).collect()),
            };
        let sealer = crate::seal::AddressSealer::from_hex(
            &std::env::var("PAYOUT_ADDRESS_KEY")
                .map_err(|_| "PAYOUT_ADDRESS_KEY is required with WALLET_URL (64 hex chars)")?,
        )?;
        let submit_timeout = Duration::from_secs(env_or("WALLET_SUBMIT_TIMEOUT_SECS", 300)?);
        let policy = PayoutPolicy {
            required_confirmations: env_or("PAYOUT_CONFIRMATIONS", 10)?,
            expiry_margin_blocks: env_or("PAYOUT_EXPIRY_MARGIN_BLOCKS", 20)?,
            max_fee_zats: env_or("PAYOUT_MAX_FEE_ZATS", 100_000)?,
            fee_headroom_zats: env_or("PAYOUT_FEE_HEADROOM_ZATS", 10_000)?,
            // The request's lease must outlive its wallet call.
            submit_lease_secs: submit_timeout.as_secs() as i64 + 120,
            worker_lease_secs: 120,
        };
        policy.validate().map_err(|e| e.to_string())?;
        Ok(Some(Self {
            wallet: HttpWallet::new(
                &url,
                token,
                Duration::from_secs(env_or("WALLET_TIMEOUT_SECS", 60)?),
                submit_timeout,
            )?,
            sealer,
            policy,
            orgs,
            owner: format!("payroll-api-{}", uuid::Uuid::new_v4()),
            poll: Duration::from_secs(env_or("PAYOUT_POLL_SECS", 20)?),
        }))
    }
}

// ---- wallet client ---------------------------------------------------------------------

pub struct HttpWallet {
    base: String,
    token: String,
    short: ureq::Agent,
    long: ureq::Agent,
}

#[derive(Deserialize, Serialize)]
pub struct WalletQuote {
    pub valid_recipient: bool,
    pub fee_zats: Option<i64>,
    pub spendable_zats: i64,
    pub pending_zats: i64,
    pub sufficient: bool,
    pub pool: Option<String>,
    pub reason: Option<String>,
}

impl HttpWallet {
    pub fn new(url: &str, token: String, short: Duration, long: Duration) -> Result<Self, String> {
        let base = url.trim_end_matches('/').to_string();
        let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
            .iter()
            .any(|p| base.starts_with(p));
        if !(base.starts_with("https://") || local) {
            return Err("WALLET_URL must be https:// (http:// only for localhost)".into());
        }
        // No connection reuse. In the live run the first call after the wallet
        // process was hard-killed and replaced failed at once ("Network Error");
        // a stale pooled keep-alive socket is the likely cause. Wallet calls are
        // local and infrequent, so a fresh connection per call costs nothing.
        let agent = |t| {
            ureq::AgentBuilder::new()
                .timeout(t)
                .max_idle_connections(0)
                .max_idle_connections_per_host(0)
                .build()
        };
        Ok(Self {
            base,
            token,
            short: agent(short),
            long: agent(long),
        })
    }

    /// `Ok(None)` for 404. 5xx and transport errors are `Unavailable` (the
    /// outcome is unknown); other 4xx are `Refused`.
    fn call<T: DeserializeOwned>(
        &self,
        agent: &ureq::Agent,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Option<T>, WalletError> {
        let req = agent
            .request(method, &format!("{}{path}", self.base))
            .set("Authorization", &format!("Bearer {}", self.token));
        let res = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match res {
            Ok(r) => r
                .into_json::<T>()
                .map(Some)
                .map_err(|e| WalletError::Unavailable(format!("unreadable wallet response: {e}"))),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(ureq::Error::Status(code, r)) if (400..500).contains(&code) => {
                let v: serde_json::Value = r.into_json().unwrap_or_default();
                Err(WalletError::Refused {
                    code: v["error"].as_str().unwrap_or("refused").to_string(),
                    message: v["message"].as_str().unwrap_or("").to_string(),
                })
            }
            Err(ureq::Error::Status(code, _)) => Err(WalletError::Unavailable(format!(
                "wallet returned HTTP {code}"
            ))),
            // Includes the URL and I/O error; request bodies (addresses) are not part of it.
            Err(ureq::Error::Transport(t)) => Err(WalletError::Unavailable(t.to_string())),
        }
    }

    /// Read-only calls are retried once on a transport failure; payments
    /// never are (an ambiguous submit is left to the reconciler).
    fn read<T: DeserializeOwned>(
        &self,
        agent: &ureq::Agent,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Option<T>, WalletError> {
        match self.call(agent, method, path, body.clone()) {
            Err(WalletError::Unavailable(e)) => {
                tracing::warn!("wallet read failed ({e}); retrying once");
                std::thread::sleep(Duration::from_millis(500));
                self.call(agent, method, path, body)
            }
            other => other,
        }
    }

    pub fn quote(&self, address: &str, amount_zats: i64) -> Result<WalletQuote, WalletError> {
        self.read(
            &self.long,
            "POST",
            "/v1/quotes",
            Some(json!({ "address": address, "amount_zats": amount_zats })),
        )?
        .ok_or_else(|| WalletError::Unavailable("quote endpoint missing".into()))
    }

    pub fn status(&self) -> Result<serde_json::Value, WalletError> {
        self.read(&self.short, "GET", "/v1/status", None)?
            .ok_or_else(|| WalletError::Unavailable("status endpoint missing".into()))
    }
}

impl WalletApi for HttpWallet {
    fn submit(&self, req: &WalletSubmit) -> Result<WalletPayment, WalletError> {
        let body = serde_json::to_value(req).expect("serializable");
        self.call(&self.long, "POST", "/v1/payments", Some(body))?
            .ok_or_else(|| WalletError::Unavailable("payments endpoint missing".into()))
    }
    fn lookup(&self, payment_id: &str) -> Result<Option<WalletPayment>, WalletError> {
        self.call(
            &self.short,
            "GET",
            &format!("/v1/payments/{payment_id}"),
            None,
        )
    }
    fn cancel(&self, payment_id: &str) -> Result<WalletPayment, WalletError> {
        // Waits behind any in-flight build in the wallet, so use the long timeout.
        self.call(
            &self.long,
            "POST",
            &format!("/v1/payments/{payment_id}/cancel"),
            Some(json!({})),
        )?
        .ok_or_else(|| WalletError::Unavailable("cancel endpoint missing".into()))
    }
}

// ---- views ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct PayoutView {
    pub id: String,
    pub claim_id: String,
    /// reserved | submitted | confirmed | failed | requires_reconciliation.
    /// Only `confirmed` means paid.
    pub state: PayoutState,
    pub amount_zats: i64,
    pub amount_zec: String,
    pub fee_estimate_zats: i64,
    pub fee_zats: Option<i64>,
    pub fee_zec: Option<String>,
    pub txid: Option<String>,
    pub mined_height: Option<i64>,
    pub confirmations: i64,
    pub required_confirmations: i64,
    pub expiry_height: Option<i64>,
    pub detail: Option<String>,
    pub network: &'static str,
    pub created_at: i64,
    pub submitted_at: Option<i64>,
    pub resolved_at: Option<i64>,
    pub updated_at: i64,
}

impl PayoutView {
    pub fn new(p: &Payout, required_confirmations: i64) -> Self {
        Self {
            id: p.id.clone(),
            claim_id: p.claim_id.clone(),
            state: p.state,
            amount_zats: p.amount_zats,
            amount_zec: format_zec(p.amount_zats),
            fee_estimate_zats: p.fee_estimate_zats,
            fee_zats: p.fee_zats,
            fee_zec: p.fee_zats.map(format_zec),
            txid: p.txid.clone(),
            mined_height: p.mined_height,
            confirmations: p.confirmations,
            required_confirmations,
            expiry_height: p.expiry_height,
            detail: p.detail.clone(),
            network: "testnet",
            created_at: p.created_at,
            submitted_at: p.submitted_at,
            resolved_at: p.resolved_at,
            updated_at: p.updated_at,
        }
    }
}

// ---- handlers --------------------------------------------------------------------------

pub(crate) fn enabled(st: &AppState) -> Result<Arc<Payouts>, ApiError> {
    st.payouts.clone().ok_or_else(|| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "payouts_disabled",
            "Payouts are not enabled on this server.",
        )
    })
}

pub(crate) fn wallet_err(e: WalletError) -> ApiError {
    tracing::warn!("payout wallet call failed: {e}");
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "wallet_unavailable",
        "The payout wallet is unavailable. Try again shortly.",
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddressReq {
    pub(crate) address: String,
}

pub(crate) fn clean_address(a: &str) -> Result<String, ApiError> {
    let a = a.trim();
    if a.is_empty() || a.len() > 1_000 || !a.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_recipient",
            "Enter a shielded Zcash testnet Unified Address.",
        ));
    }
    Ok(a.to_string())
}

/// Blocking work off the async runtime (wallet calls can take minutes).
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "worker panicked",
        )
    })?
}

#[derive(Serialize)]
pub struct QuoteView {
    pub claim_id: String,
    pub amount_zats: i64,
    pub amount_zec: String,
    pub fee_zats: Option<i64>,
    pub fee_zec: Option<String>,
    /// Paid by the organization's payout wallet in addition to the amount.
    pub wallet_debit_zats: Option<i64>,
    pub valid_recipient: bool,
    pub sufficient: bool,
    pub reason: Option<String>,
    pub network: &'static str,
    pub required_confirmations: i64,
}

pub(crate) fn quote_view(
    claim_id: &str,
    amount: i64,
    q: &WalletQuote,
    policy: &PayoutPolicy,
) -> QuoteView {
    let fee_ok = q.fee_zats.and_then(|f| policy.max_fee_for(f)).is_some();
    QuoteView {
        claim_id: claim_id.to_string(),
        amount_zats: amount,
        amount_zec: format_zec(amount),
        fee_zats: q.fee_zats,
        fee_zec: q.fee_zats.map(format_zec),
        wallet_debit_zats: q.fee_zats.map(|f| f + amount),
        valid_recipient: q.valid_recipient,
        sufficient: q.sufficient && fee_ok,
        reason: if q.sufficient && !fee_ok {
            Some("The network fee is above this server's limit.".into())
        } else {
            q.reason.clone()
        },
        network: "testnet",
        required_confirmations: policy.required_confirmations,
    }
}

pub async fn quote(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(claim): Path<String>,
    Json(r): Json<AddressReq>,
) -> ApiResult<QuoteView> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let po = enabled(&st)?;
    let address = clean_address(&r.address)?;
    let orgs = po.orgs.clone();
    let (cid, a2) = (claim.clone(), a.clone());
    let c = with_db(&st, move |db| {
        db.payout_eligibility(&a2, &cid, &|o| orgs.allows(o))
    })
    .await?;
    let q = blocking(move || {
        let q = po
            .wallet
            .quote(&address, c.amount_zats)
            .map_err(wallet_err)?;
        Ok(quote_view(&c.id, c.amount_zats, &q, &po.policy))
    })
    .await?;
    Ok(Json(q))
}

/// The answer to a repeated request for a claim that already has payout `p`.
fn existing_payout(
    p: &Payout,
    actor: &str,
    address: &str,
    need: i64,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    // Only the claimant may repeat the request. Otherwise a manager (who can
    // view the claim) could test guessed addresses against the payout's
    // fingerprint via the duplicate/conflict answer.
    if p.requested_by != actor {
        return Err(err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Only the claimant can request a payout.",
        ));
    }
    if recipient_fingerprint(&p.id, address) != p.recipient_fingerprint {
        return Err(err(
            StatusCode::CONFLICT,
            "payout_conflict",
            "A payout to a different address was already requested for this claim.",
        ));
    }
    let v = PayoutView::new(p, need);
    Ok((
        StatusCode::OK,
        Json(json!({ "payout": v, "duplicate": true })),
    ))
}

/// The contributor's deliberate "pay this claim" action.
pub async fn request(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(claim): Path<String>,
    Json(r): Json<AddressReq>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let po = enabled(&st)?;
    let address = clean_address(&r.address)?;
    let need = po.policy.required_confirmations;

    // A repeat of an earlier request returns that payout, whatever its state,
    // without touching the wallet.
    let (cid, a2) = (claim.clone(), a.clone());
    if let Some(p) = with_db(&st, move |db| db.claim_payout(&a2, &cid)).await? {
        return existing_payout(&p, &a, &address, need);
    }

    // Eligibility and amount come from the engine; fee and balance from a
    // real proposal in the wallet.
    let orgs = po.orgs.clone();
    let (cid, a2) = (claim.clone(), a.clone());
    let c = match with_db(&st, move |db| {
        db.payout_eligibility(&a2, &cid, &|o| orgs.allows(o))
    })
    .await
    {
        Ok(c) => c,
        Err(e) => {
            // A concurrent request may have created the payout since the check
            // above: answer as the repeat it is, not with an error.
            let (cid, a2) = (claim.clone(), a.clone());
            if let Ok(Some(p)) = with_db(&st, move |db| db.claim_payout(&a2, &cid)).await {
                return existing_payout(&p, &a, &address, need);
            }
            return Err(e);
        }
    };
    let (po2, addr2) = (po.clone(), address.clone());
    let q = blocking(move || po2.wallet.quote(&addr2, c.amount_zats).map_err(wallet_err)).await?;
    if !q.valid_recipient {
        return Err(err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_recipient",
            q.reason.unwrap_or_else(|| "Invalid payout address.".into()),
        ));
    }
    let fee = q.fee_zats.unwrap_or_default();
    let max_fee = match (q.sufficient, po.policy.max_fee_for(fee)) {
        (true, Some(m)) => m,
        _ => {
            let v = quote_view(&claim, c.amount_zats, &q, &po.policy);
            return Err(ApiError(
                StatusCode::CONFLICT,
                json!({
                    "error": "insufficient_wallet_balance",
                    "message": v.reason.clone().unwrap_or_else(|| "The payout wallet cannot cover this payout and its fee right now.".into()),
                    "quote": v,
                }),
            ));
        }
    };

    // Record the payout (and lock the claim to it) BEFORE asking the wallet to
    // pay; then submit under this process's lease.
    let pool = st.pool.clone();
    let clock = st.clock.clone();
    let out = blocking(move || {
        let conn = pool.get().map_err(|_| {
            err(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable", "Database unavailable.")
        })?;
        let mut db = hushush_payroll::Payroll::with_client(crate::Pooled(conn), clock);
        let addr_fp = address.clone();
        let orgs = po.orgs.clone();
        let outcome = db
            .request_payout(
                &a,
                &PayoutTerms {
                    claim_id: &claim,
                    fee_estimate_zats: fee,
                    max_fee_zats: max_fee,
                    lease_owner: &po.owner,
                    lease_secs: po.policy.submit_lease_secs,
                },
                &move |id| recipient_fingerprint(id, &addr_fp),
                &|o| orgs.allows(o),
            )
            .map_err(ApiError::from)?;
        if outcome.duplicate {
            return Ok((outcome.payout, true));
        }
        tracing::info!(payout_id = %outcome.payout.id, claim_id = %outcome.payout.claim_id, "payout reserved; submitting to wallet");
        let p = submit_payout(&mut db, &po.wallet, &po.policy, &outcome.payout, &address)
            .map_err(ApiError::from)?;
        tracing::info!(payout_id = %p.id, state = p.state.as_str(), txid = ?p.txid, "payout submission recorded");
        Ok((p, false))
    })
    .await?;
    let (p, duplicate) = out;
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

#[derive(Serialize)]
pub struct WalletSummary {
    pub network: &'static str,
    pub spendable_zats: i64,
    pub spendable_zec: String,
    pub pending_zats: i64,
    pub pending_zec: String,
    pub funding_address: String,
    pub chain_tip: Option<i64>,
    pub last_sync_at: Option<i64>,
    pub required_confirmations: i64,
}

/// Payout wallet balance and funding address, for the organization's managers.
pub async fn wallet_summary(
    State(st): State<AppState>,
    h: HeaderMap,
    Path(org): Path<String>,
) -> ApiResult<WalletSummary> {
    let a = actor(&st, &h)?;
    require_actor(&a)?;
    let po = enabled(&st)?;
    let o2 = org.clone();
    with_db(&st, move |db| db.require_payroll_manager(&a, &o2)).await?;
    if !po.orgs.allows(&org) {
        return Err(err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Payouts are not enabled for this organization.",
        ));
    }
    let need = po.policy.required_confirmations;
    let s = blocking(move || po.wallet.status().map_err(wallet_err)).await?;
    let n = |k: &str| s[k].as_i64().unwrap_or_default();
    Ok(Json(WalletSummary {
        network: "testnet",
        spendable_zats: n("spendable_zats"),
        spendable_zec: format_zec(n("spendable_zats")),
        pending_zats: n("pending_zats"),
        pending_zec: format_zec(n("pending_zats")),
        funding_address: s["funding_address"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        chain_tip: s["chain_tip"].as_i64(),
        last_sync_at: s["last_sync_at"].as_i64(),
        required_confirmations: need,
    }))
}

// ---- worker ---------------------------------------------------------------------------

/// Background confirmation tracking and reconciliation. Runs one pass at
/// start-up (restart recovery) and then every `poll`. Safe to run in several
/// processes at once: payouts are leased with `FOR UPDATE SKIP LOCKED`.
pub fn spawn_worker(st: AppState, po: Arc<Payouts>) {
    std::thread::Builder::new()
        .name("payout-worker".into())
        .spawn(move || {
            tracing::info!(owner = %po.owner, "payout worker started");
            loop {
                match st.pool.get() {
                    Ok(conn) => {
                        let mut db = hushush_payroll::Payroll::with_client(
                            crate::Pooled(conn),
                            st.clock.clone(),
                        );
                        match reconcile_due(&mut db, &po.wallet, &po.policy, &po.owner, 25) {
                            Ok(updated) => {
                                for p in updated {
                                    tracing::info!(
                                        payout_id = %p.id,
                                        claim_id = %p.claim_id,
                                        state = p.state.as_str(),
                                        confirmations = p.confirmations,
                                        txid = ?p.txid,
                                        detail = ?p.detail,
                                        "payout reconciled"
                                    );
                                }
                            }
                            Err(PayrollError::Conflict(_)) => {}
                            Err(e) => tracing::error!("payout worker pass failed: {e}"),
                        }
                    }
                    Err(e) => tracing::error!("payout worker: database unavailable: {e}"),
                }
                std::thread::sleep(po.poll);
            }
        })
        .expect("spawn payout worker");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// A server that answers with `Connection: keep-alive` but closes every
    /// connection. (This does NOT reproduce the live-run failure — ureq copes
    /// with cleanly closed sockets either way; it checks the no-reuse client
    /// works against a server that never keeps connections.)
    fn closing_server() -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let json = br#"{"valid_recipient":true,"fee_zats":10000,"spendable_zats":1,"pending_zats":0,"sufficient":true}"#;
                write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: keep-alive\r\nContent-Length: {}\r\n\r\n", json.len()).unwrap();
                s.write_all(json).unwrap();
                // dropped here: the connection is gone
            }
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    #[test]
    fn wallet_calls_work_when_the_server_closes_each_connection() {
        let w = HttpWallet::new(
            &closing_server(),
            "t".repeat(32),
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .unwrap();
        for _ in 0..3 {
            let q = w
                .quote("utest1x", 1)
                .expect("each call must open a fresh connection");
            assert_eq!(q.fee_zats, Some(10_000));
        }
    }

    #[test]
    fn wallet_url_must_be_https_or_localhost() {
        let mk = |u: &str| {
            HttpWallet::new(
                u,
                "t".repeat(32),
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
        };
        assert!(mk("http://10.0.0.5:7979").is_err());
        assert!(mk("ftp://localhost").is_err());
        assert!(mk("https://wallet.internal").is_ok());
        assert!(mk("http://127.0.0.1:7979").is_ok());
    }

    #[test]
    fn org_allow_list() {
        assert!(OrgAllow::All.allows("x"));
        let only = OrgAllow::Only(["a".to_string()].into_iter().collect());
        assert!(only.allows("a") && !only.allows("b"));
    }
}

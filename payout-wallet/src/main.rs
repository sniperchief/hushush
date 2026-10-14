//! `hushush-payout-wallet` — see the crate docs and docs/PHASE3_PAYOUTS.md.
//!
//! Configuration (environment):
//!
//! | Variable | Default | |
//! |---|---|---|
//! | `WALLET_API_TOKEN` | — (required, ≥ 32 chars) | bearer token; only payroll-api holds it |
//! | `WALLET_API_ADDR` | `127.0.0.1:7979` | non-loopback refused unless `WALLET_API_ALLOW_NONLOCAL=1` |
//! | `WALLET_NAME` | — (required) | Phase 0 wallet name (`init` / `restore` with the CLI) |
//! | `WALLET_LIGHTWALLETD_URL` | — (required) | no default: choose the server deliberately |
//! | `WALLET_MAX_PAYMENT_ZATS` | `100000000` (1 TAZ) | per-payment cap |
//! | `WALLET_MAX_FEE_ZATS` | `100000` | absolute fee cap |
//! | `WALLET_SYNC_SECS` | `30` | background sync / rebroadcast interval |
//! | `HUSHUSH_DATA_DIR`, `HUSHUSH_SECRETS_DIR` | Phase 0 defaults | wallet + key locations (outside git) |

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, bail};

use hushush_payout_wallet::{STACK_SIZE, actor, chain, http, intents::IntentStore};
use hushush_wallet_proof::config::WalletPaths;

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> anyhow::Result<T> {
    match std::env::var(name) {
        Ok(v) => v
            .parse()
            .map_err(|_| anyhow::anyhow!("{name} has an invalid value")),
        Err(_) => Ok(default),
    }
}

/// `RUST_LOG` (default `info`), but upstream wallet/crypto crates are capped
/// at `info` whatever `RUST_LOG` says: their debug/trace output can include
/// addresses, notes or key material, which must never reach the logs.
fn log_filter() -> tracing_subscriber::EnvFilter {
    let mut f = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,zcash_client_backend=warn".into());
    for target in [
        "zcash_client_backend",
        "zcash_client_sqlite",
        "zcash_primitives",
        "zcash_keys",
        "zcash_proofs",
        "orchard",
        "sapling",
        "halo2_proofs",
        "tonic",
        "h2",
        "hyper",
    ] {
        if let Ok(d) = format!("{target}=info").parse() {
            f = f.add_directive(d);
        }
    }
    f
}

fn main() -> anyhow::Result<()> {
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()?
                .block_on(run())
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("main thread panicked"))?
}

async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(log_filter())
        .init();

    let token = std::env::var("WALLET_API_TOKEN").context("WALLET_API_TOKEN is required")?;
    if token.len() < 32 {
        bail!("WALLET_API_TOKEN must be at least 32 characters");
    }
    let addr: SocketAddr = env_or("WALLET_API_ADDR", "127.0.0.1:7979".parse()?)?;
    if !addr.ip().is_loopback() && std::env::var("WALLET_API_ALLOW_NONLOCAL").as_deref() != Ok("1")
    {
        bail!(
            "refusing to listen on non-loopback {addr}; set WALLET_API_ALLOW_NONLOCAL=1 behind a TLS proxy on a private network"
        );
    }
    let name = std::env::var("WALLET_NAME").context("WALLET_NAME is required")?;
    let url = std::env::var("WALLET_LIGHTWALLETD_URL").context(
        "WALLET_LIGHTWALLETD_URL is required (no default: pick the server deliberately)",
    )?;

    let paths = WalletPaths::for_name(&name)?;
    paths.ensure_outside_source_control()?;
    anyhow::ensure!(
        paths.exists(),
        "wallet '{name}' does not exist; create it with the wallet-proof CLI"
    );
    anyhow::ensure!(
        paths.encrypted_mnemonic.exists() && paths.identity.exists(),
        "wallet '{name}' has no spend authority (encrypted mnemonic + identity) here"
    );

    // One service per wallet: a second one would race on note selection.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.dir.join("payout-wallet.lock"))?;
    lock.try_lock().map_err(|_| {
        anyhow::anyhow!("another payout-wallet is already running for wallet '{name}'")
    })?;

    let intents_path = paths.dir.join("payout-intents.sqlite");
    let chain_ok = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let cfg = actor::ActorConfig {
        paths: paths.clone(),
        intents_path: intents_path.clone(),
        lightwalletd_url: url.clone(),
        max_payment_zats: env_or("WALLET_MAX_PAYMENT_ZATS", 100_000_000)?,
        max_fee_zats: env_or("WALLET_MAX_FEE_ZATS", 100_000)?,
        sync_interval: Duration::from_secs(env_or("WALLET_SYNC_SECS", 30)?),
        fault: std::env::var("WALLET_FAULT").ok(),
        chain_ok: chain_ok.clone(),
    };
    #[cfg(feature = "fault-injection")]
    tracing::warn!(
        "built with fault injection (test-only); WALLET_FAULT={:?}",
        cfg.fault
    );

    tracing::info!(wallet = %name, lightwalletd = %url, "starting payout wallet (TESTNET); syncing and recovering");
    let (actor_tx, ready) = actor::spawn(cfg)?;
    let state = http::AppState {
        token: Arc::new(token.into_bytes()),
        actor: actor_tx,
        reader: Arc::new(Mutex::new(IntentStore::open(&intents_path)?)),
        client: chain::connect_with_retry(&url).await?,
        ufvks: Arc::new(ready.ufvks),
        wallet_db: paths.data_db.clone(),
        chain_ok,
    };
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("payout wallet listening on {addr}");
    axum::serve(listener, http::app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    drop(lock);
    Ok(())
}

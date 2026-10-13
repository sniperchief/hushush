//! lightwalletd gRPC connection (TLS, webpki roots). Testnet only.

use anyhow::Context;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use zcash_client_backend::proto::service::{
    self, compact_tx_streamer_client::CompactTxStreamerClient,
};

pub const DEFAULT_TESTNET_SERVER: &str = "https://testnet.zec.rocks:443";

pub type Client = CompactTxStreamerClient<Channel>;

/// Server URL from `HUSHUSH_LIGHTWALLETD`, defaulting to the zec.rocks testnet
/// lightwalletd. Plain `http://` is only allowed for localhost.
pub fn server_url() -> String {
    std::env::var("HUSHUSH_LIGHTWALLETD").unwrap_or_else(|_| DEFAULT_TESTNET_SERVER.to_string())
}

pub async fn connect(url: &str) -> anyhow::Result<Client> {
    // Select rustls' `ring` provider explicitly (no-op if already installed).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let uri: tonic::transport::Uri = url.parse().context("invalid lightwalletd URL")?;
    let host = uri.host().context("lightwalletd URL has no host")?.to_string();
    let mut ep = Endpoint::from(uri.clone())
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(120));
    match uri.scheme_str() {
        Some("https") => {
            ep = ep.tls_config(
                ClientTlsConfig::new()
                    .domain_name(host)
                    .assume_http2(true)
                    .with_webpki_roots(),
            )?;
        }
        Some("http") if matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]") => {}
        _ => anyhow::bail!("lightwalletd must be https:// (http:// allowed for localhost only)"),
    }
    let channel = ep
        .connect()
        .await
        .with_context(|| format!("connecting to lightwalletd at {url}"))?;
    Ok(CompactTxStreamerClient::new(channel))
}

/// Verifies the server is serving Zcash testnet and returns its info.
pub async fn check_testnet(client: &mut Client) -> anyhow::Result<service::LightdInfo> {
    let info = client
        .get_lightd_info(service::Empty {})
        .await?
        .into_inner();
    anyhow::ensure!(
        info.chain_name == "test",
        "lightwalletd reports chain '{}', expected 'test' — refusing to continue",
        info.chain_name
    );
    Ok(info)
}

pub async fn chain_tip(client: &mut Client) -> anyhow::Result<u32> {
    let b = client
        .get_latest_block(service::ChainSpec::default())
        .await?
        .into_inner();
    Ok(u32::try_from(b.height)?)
}

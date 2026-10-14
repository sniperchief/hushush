//! Chain access on top of the Phase 0 `remote` / `wallet::fetch_transaction`.

use anyhow::Context;
use zcash_primitives::transaction::{Transaction, TxId};
use zcash_protocol::consensus::{BlockHeight, BranchId};

use hushush_wallet_proof::{NETWORK, remote, wallet};

/// Connects and refuses a server that is not on testnet or not on the
/// consensus branch this build expects at its tip. Around network upgrades
/// public servers have sat on dead forks (Phase 0 report §3).
pub async fn connect_checked(url: &str) -> anyhow::Result<remote::Client> {
    let mut c = remote::connect(url).await?;
    verify_server(&mut c, url).await?;
    Ok(c)
}

/// [`connect_checked`] with retries, for start-up: a transient DNS or TLS
/// failure (seen in the Phase 4 live run) shouldn't take the service down.
/// The network/branch check applies on every attempt.
pub async fn connect_with_retry(url: &str) -> anyhow::Result<remote::Client> {
    const ATTEMPTS: u32 = 20;
    let mut attempt = 1;
    loop {
        match connect_checked(url).await {
            Ok(c) => return Ok(c),
            Err(e) if attempt < ATTEMPTS => {
                tracing::warn!(
                    "lightwalletd not usable (attempt {attempt}/{ATTEMPTS}): {e:#}; retrying in 15 s"
                );
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Re-checks that the server is still on testnet and on the expected
/// consensus branch (run at start-up and on every background sync).
pub async fn verify_server(c: &mut remote::Client, url: &str) -> anyhow::Result<()> {
    let info = remote::check_testnet(c).await?;
    let tip = BlockHeight::from_u32(u32::try_from(info.block_height)?);
    let served = u32::from_str_radix(info.consensus_branch_id.trim_start_matches("0x"), 16)
        .context("lightwalletd reported an unparseable consensus branch id")?;
    let expected = [
        u32::from(BranchId::for_height(&NETWORK, tip)),
        u32::from(BranchId::for_height(&NETWORK, tip + 1)),
    ];
    anyhow::ensure!(
        expected.contains(&served),
        "lightwalletd at {url} reports consensus branch {served:08x} at height {tip}, but this \
         build expects {:08x}; refusing a server on a different fork or network upgrade",
        expected[0]
    );
    Ok(())
}

pub enum Fetched {
    Mined(Transaction, BlockHeight, Vec<u8>),
    Mempool(Transaction, Vec<u8>),
    /// The server knows no such transaction (neither mined nor in its mempool).
    Missing,
}

fn is_not_found(e: &anyhow::Error) -> bool {
    e.downcast_ref::<tonic::Status>().is_some_and(|s| {
        let m = s.message().to_lowercase();
        s.code() == tonic::Code::NotFound
            || m.contains("no such mempool or blockchain transaction")
            || m.contains("not found")
    })
}

pub async fn fetch(
    client: &mut remote::Client,
    tip: BlockHeight,
    txid: TxId,
) -> anyhow::Result<Fetched> {
    match wallet::fetch_transaction(client, &NETWORK, tip + 1, txid).await {
        Ok(Some((tx, Some(h), raw))) => Ok(Fetched::Mined(tx, h, raw)),
        Ok(Some((tx, None, raw))) => Ok(Fetched::Mempool(tx, raw)),
        Ok(None) => Ok(Fetched::Missing),
        Err(e) if is_not_found(&e) => Ok(Fetched::Missing),
        Err(e) => Err(e),
    }
}

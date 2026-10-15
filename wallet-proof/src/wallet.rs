//! Wallet persistence on `zcash_client_sqlite`, plus read helpers.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, anyhow};
use rand::{rand_core::UnwrapErr, rngs::SysRng};
use secrecy::SecretVec;
use serde::Serialize;
use tonic::Code;
use zcash_client_backend::data_api::{
    Account as _, AccountBirthday, TransactionDataRequest, TransactionStatus, WalletRead,
    WalletWrite, wallet::ConfirmationsPolicy, wallet::decrypt_and_store_transaction,
};
use zcash_client_backend::proto::service;
use zcash_client_sqlite::{AccountUuid, WalletDb, util::SystemClock, wallet::init::init_wallet_db};
use zcash_keys::address::UnifiedAddress;
use zcash_keys::keys::{ReceiverRequirement, UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_primitives::transaction::{Transaction, TxId};
use zcash_protocol::consensus::{BlockHeight, BranchId, Parameters};

use crate::{NETWORK, remote::Client};

/// OS-backed CSPRNG, as used by upstream examples.
pub type Rng = UnwrapErr<SysRng>;

pub type Db = WalletDb<rusqlite::Connection, zcash_protocol::consensus::Network, SystemClock, Rng>;

/// Receive addresses carry Orchard + Sapling receivers and NO transparent
/// receiver, so a payer cannot accidentally fall back to a transparent payment.
pub const SHIELDED_ONLY_UA: UnifiedAddressRequest = UnifiedAddressRequest::unsafe_custom(
    ReceiverRequirement::Require,
    ReceiverRequirement::Require,
    ReceiverRequirement::Omit,
);

pub fn open(data_db: &Path) -> anyhow::Result<Db> {
    anyhow::ensure!(data_db.exists(), "wallet database {} not found", data_db.display());
    let mut db = WalletDb::for_path(data_db, NETWORK, SystemClock, UnwrapErr(SysRng))?;
    // Apply any schema migrations shipped with a newer zcash_client_sqlite.
    init_wallet_db(&mut db, None).map_err(|e| anyhow!("wallet DB migration failed: {e:?}"))?;
    Ok(db)
}

/// Creates the wallet DB (running all schema migrations) and a single ZIP 32
/// account (index 0) derived from `seed`.
pub fn create(
    data_db: &Path,
    name: &str,
    seed: &SecretVec<u8>,
    birthday: &AccountBirthday,
) -> anyhow::Result<(Db, AccountUuid, UnifiedSpendingKey)> {
    if let Some(parent) = data_db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut db = WalletDb::for_path(data_db, NETWORK, SystemClock, UnwrapErr(SysRng))?;
    init_wallet_db(&mut db, None).map_err(|e| anyhow!("wallet DB init failed: {e:?}"))?;
    let (account, usk) = db.create_account(name, seed, birthday, None)?;
    Ok((db, account, usk))
}

/// The wallet's account for the CLI: its only account, or — when the payout
/// service has added company accounts (Phase 6) — the primary one, derived at
/// ZIP 32 index 0.
pub fn sole_account<DbT: WalletRead<AccountId = AccountUuid>>(db: &DbT) -> anyhow::Result<AccountUuid>
where
    DbT::Error: std::error::Error + Send + Sync + 'static,
{
    let ids = db.get_account_ids()?;
    match ids.as_slice() {
        [one] => Ok(*one),
        [] => Err(anyhow!("wallet has no accounts")),
        many => {
            for id in many {
                let a = db.get_account(*id)?.context("account vanished")?;
                if a.source().key_derivation().map(|d| u32::from(d.account_index())) == Some(0) {
                    return Ok(*id);
                }
            }
            Err(anyhow!("wallet has {} accounts and none at ZIP 32 index 0", many.len()))
        }
    }
}

/// Derives the spending key for the account from the seed, checking that it
/// matches the account's stored viewing key.
pub fn spending_key(db: &Db, account: AccountUuid, seed: &SecretVec<u8>) -> anyhow::Result<UnifiedSpendingKey> {
    use secrecy::ExposeSecret;
    let acct = db.get_account(account)?.context("account missing")?;
    let derivation = acct
        .source()
        .key_derivation()
        .context("account is view-only; cannot spend")?;
    let usk = UnifiedSpendingKey::from_seed(&NETWORK, seed.expose_secret(), derivation.account_index())
        .map_err(|e| anyhow!("key derivation failed: {e:?}"))?;
    let stored = acct.ufvk().context("account has no UFVK")?.encode(&NETWORK)?;
    anyhow::ensure!(
        usk.to_unified_full_viewing_key().encode(&NETWORK)? == stored,
        "seed does not match this wallet's account"
    );
    Ok(usk)
}

/// The account's shielded-only receive address (creating it on first use).
pub fn receive_address(db: &mut Db, account: AccountUuid) -> anyhow::Result<UnifiedAddress> {
    if let Some(ua) = db.get_last_generated_address_matching(account, SHIELDED_ONLY_UA)? {
        return Ok(ua);
    }
    let (ua, _) = db
        .get_next_available_address(account, SHIELDED_ONLY_UA)?
        .context("could not generate a shielded address")?;
    Ok(ua)
}

#[derive(Debug, Serialize)]
pub struct BalanceReport {
    pub chain_tip_height: u32,
    pub fully_scanned_height: u32,
    pub sapling_spendable: u64,
    pub orchard_spendable: u64,
    pub ironwood_spendable: u64,
    pub spendable_total: u64,
    pub pending_change: u64,
    pub pending_spendability: u64,
    pub total: u64,
    pub transparent_total: u64,
}

pub fn balance(db: &Db, account: AccountUuid) -> anyhow::Result<Option<BalanceReport>> {
    let Some(summary) = db.get_wallet_summary(ConfirmationsPolicy::default())? else {
        return Ok(None);
    };
    let b = summary
        .account_balances()
        .get(&account)
        .context("no balance for account")?;
    Ok(Some(BalanceReport {
        chain_tip_height: summary.chain_tip_height().into(),
        fully_scanned_height: summary.fully_scanned_height().into(),
        sapling_spendable: b.sapling_balance().spendable_value().into_u64(),
        orchard_spendable: b.orchard_balance().spendable_value().into_u64(),
        ironwood_spendable: b.ironwood_balance().spendable_value().into_u64(),
        spendable_total: b.spendable_value().into_u64(),
        pending_change: b.change_pending_confirmation().into_u64(),
        pending_spendability: b.value_pending_spendability().into_u64(),
        total: b.total().into_u64(),
        transparent_total: b.unshielded_balance().total().into_u64(),
    }))
}

#[derive(Debug, Serialize)]
pub struct TxStatus {
    pub txid: String,
    pub mined_height: Option<u32>,
    pub chain_tip: Option<u32>,
    pub confirmations: u32,
}

pub fn tx_status(db: &Db, txid: TxId) -> anyhow::Result<TxStatus> {
    let mined = db.get_tx_height(txid)?;
    let tip = db.chain_height()?;
    Ok(TxStatus {
        txid: txid.to_string(),
        mined_height: mined.map(u32::from),
        chain_tip: tip.map(u32::from),
        confirmations: confirmations(mined, tip),
    })
}

/// Number of confirmations: 1 when mined in the tip block, 0 when unmined.
pub fn confirmations(mined: Option<BlockHeight>, tip: Option<BlockHeight>) -> u32 {
    match (mined, tip) {
        (Some(m), Some(t)) if t >= m => u32::from(t) - u32::from(m) + 1,
        _ => 0,
    }
}

pub fn parse_txid(s: &str) -> anyhow::Result<TxId> {
    // TxIds are displayed byte-reversed, as in zcashd/explorers.
    let mut bytes: [u8; 32] = hex::decode(s.trim())?
        .try_into()
        .map_err(|_| anyhow!("txid must be 32 bytes of hex"))?;
    bytes.reverse();
    Ok(TxId::from_bytes(bytes))
}

/// Fetches a full transaction from lightwalletd. Returns the parsed tx and its
/// mined height (None if in the mempool).
pub async fn fetch_transaction<P: Parameters>(
    client: &mut Client,
    params: &P,
    fallback_height: BlockHeight,
    txid: TxId,
) -> anyhow::Result<Option<(Transaction, Option<BlockHeight>, Vec<u8>)>> {
    let req = service::TxFilter {
        hash: txid.as_ref().to_vec(),
        ..Default::default()
    };
    let raw = match client.get_transaction(req).await {
        Ok(r) => r.into_inner(),
        Err(s) if s.code() == Code::NotFound => return Ok(None),
        Err(s) => return Err(s.into()),
    };
    let mined = (raw.height > 0 && raw.height < u64::from(u32::MAX))
        .then(|| BlockHeight::from_u32(raw.height as u32));
    let tx = Transaction::read(
        &raw.data[..],
        BranchId::for_height(params, mined.unwrap_or(fallback_height)),
    )?;
    Ok(Some((tx, mined, raw.data)))
}

/// Services the wallet's outstanding transaction-data requests: fetches full
/// transactions so memos and sent-tx status are recorded (compact blocks carry
/// neither).
pub async fn enhance(db: &mut Db, client: &mut Client) -> anyhow::Result<usize> {
    let tip = db.chain_height()?.context("sync first")?;
    let mut done = BTreeSet::new();
    loop {
        let reqs: Vec<_> = db
            .transaction_data_requests()?
            .into_iter()
            .filter(|r| !done.contains(r))
            .collect();
        if reqs.is_empty() {
            return Ok(done.len());
        }
        for req in reqs {
            match &req {
                TransactionDataRequest::GetStatus(txid) => {
                    let status = fetch_transaction(client, &NETWORK, tip, *txid)
                        .await?
                        .map_or(TransactionStatus::TxidNotRecognized, |(_, h, _)| {
                            h.map_or(TransactionStatus::NotInMainChain, TransactionStatus::Mined)
                        });
                    db.set_transaction_status(*txid, status)?;
                }
                TransactionDataRequest::Enhancement(txid) => {
                    match fetch_transaction(client, &NETWORK, tip, *txid).await? {
                        Some((tx, h, _)) => decrypt_and_store_transaction(&NETWORK, db, &tx, h)?,
                        None => db.set_transaction_status(*txid, TransactionStatus::TxidNotRecognized)?,
                    }
                }
                // Only reachable with transparent-inputs, which this proof does not enable.
                #[allow(unreachable_patterns)]
                _ => {}
            }
            done.insert(req);
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HistoryTx {
    pub txid: String,
    pub mined_height: Option<u32>,
    pub account_balance_delta_zats: i64,
    pub fee_paid_zats: Option<i64>,
    pub expired_unmined: bool,
    pub outputs: Vec<HistoryOutput>,
}

#[derive(Debug, Serialize)]
pub struct HistoryOutput {
    pub pool: String,
    pub value_zats: i64,
    pub is_change: bool,
    /// True for an output this wallet sent to someone else.
    pub is_sent: bool,
    pub to_address: Option<String>,
    pub memo: Option<String>,
}

fn txid_display(blob: &[u8]) -> String {
    let mut b = blob.to_vec();
    b.reverse();
    hex::encode(b)
}

fn pool_name(code: i64) -> String {
    match code {
        0 => "transparent".into(),
        2 => "sapling".into(),
        3 => "orchard".into(),
        4 => "ironwood".into(),
        other => format!("pool#{other}"),
    }
}

fn memo_text(blob: Option<Vec<u8>>) -> Option<String> {
    use zcash_protocol::memo::{Memo, MemoBytes};
    let mb = MemoBytes::from_bytes(&blob?).ok()?;
    match Memo::try_from(mb).ok()? {
        Memo::Empty => None,
        Memo::Text(t) => Some(t.to_string()),
        other => Some(format!("{other:?}")),
    }
}

/// Transaction history, read directly from the wallet DB's public views.
pub fn history(data_db: &Path) -> anyhow::Result<Vec<HistoryTx>> {
    let conn = rusqlite::Connection::open_with_flags(
        data_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut txs = conn.prepare(
        "SELECT txid, mined_height, account_balance_delta, fee_paid, expired_unmined
         FROM v_transactions ORDER BY mined_height IS NULL, mined_height, tx_index",
    )?;
    let mut outs = conn.prepare(
        "SELECT output_pool, value, is_change, (to_account_uuid IS NULL) AS is_sent, to_address, memo
         FROM v_tx_outputs WHERE txid = ?1 ORDER BY output_pool, output_index",
    )?;
    let rows = txs
        .query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, Option<u32>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<bool>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = vec![];
    for (txid, mined, delta, fee, expired) in rows {
        let outputs = outs
            .query_map([&txid], |r| {
                Ok(HistoryOutput {
                    pool: pool_name(r.get(0)?),
                    value_zats: r.get(1)?,
                    is_change: r.get::<_, Option<bool>>(2)?.unwrap_or(false),
                    is_sent: r.get::<_, Option<bool>>(3)?.unwrap_or(false),
                    to_address: r.get(4)?,
                    memo: memo_text(r.get(5)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        out.push(HistoryTx {
            txid: txid_display(&txid),
            mined_height: mined,
            account_balance_delta_zats: delta,
            fee_paid_zats: fee,
            expired_unmined: expired.unwrap_or(false),
            outputs,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_counting() {
        let h = BlockHeight::from_u32;
        assert_eq!(confirmations(None, Some(h(10))), 0);
        assert_eq!(confirmations(Some(h(10)), Some(h(10))), 1);
        assert_eq!(confirmations(Some(h(10)), Some(h(19))), 10);
        assert_eq!(confirmations(Some(h(10)), None), 0);
    }

    #[test]
    fn txid_display_round_trip() {
        let s = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        assert_eq!(parse_txid(s).unwrap().to_string(), s);
        assert!(parse_txid("abcd").is_err());
    }
}

#[derive(Debug, Serialize)]
pub struct PendingIncoming {
    pub txid: String,
    pub value_zats: u64,
    pub pool: String,
    pub memo: Option<String>,
}

/// Trial-decrypts the server's current mempool with the wallet's viewing keys
/// to show incoming payments that are not yet mined. Read-only: nothing is
/// stored, and spendability still requires confirmations.
pub async fn pending_incoming(db: &Db, client: &mut Client) -> anyhow::Result<(usize, Vec<PendingIncoming>)> {
    use futures_util::StreamExt;
    use zcash_client_backend::{TransferType, decrypt_transaction};
    let ufvks = db.get_unified_full_viewing_keys()?;
    let tip = db.chain_height()?.context("sync first")?;
    let target = tip + 1;
    let mut stream = client
        .get_mempool_stream(service::Empty {})
        .await?
        .into_inner();
    let (mut seen, mut found) = (0, vec![]);
    // The stream stays open until the next block; stop after a short idle gap.
    while let Ok(Some(item)) =
        tokio::time::timeout(std::time::Duration::from_secs(5), stream.next()).await
    {
        let raw = item?;
        let Ok(tx) = Transaction::read(&raw.data[..], BranchId::for_height(&NETWORK, target)) else {
            continue;
        };
        seen += 1;
        let d = decrypt_transaction(&NETWORK, None, Some(tip), &tx, &ufvks);
        let txid = tx.txid().to_string();
        for o in d.sapling_outputs() {
            if o.transfer_type() == TransferType::Incoming {
                found.push(PendingIncoming {
                    txid: txid.clone(),
                    value_zats: o.note().value().inner(),
                    pool: "sapling".into(),
                    memo: memo_text(Some(o.memo().as_slice().to_vec())),
                });
            }
        }
        for (pool, outs) in [("orchard", d.orchard_outputs()), ("ironwood", d.ironwood_outputs())] {
            for o in outs {
                if o.transfer_type() == TransferType::Incoming {
                    found.push(PendingIncoming {
                        txid: txid.clone(),
                        value_zats: o.note().0.value().inner(),
                        pool: pool.into(),
                        memo: memo_text(Some(o.memo().as_slice().to_vec())),
                    });
                }
            }
        }
    }
    Ok((seen, found))
}

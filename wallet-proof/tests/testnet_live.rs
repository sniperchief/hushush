//! LIVE TESTNET TESTS — real network, real lightwalletd, real chain data.
//!
//! Ignored by default. Run with:
//!     cargo test --release --test testnet_live -- --ignored --test-threads=1
//! Server: $HUSHUSH_LIGHTWALLETD (default https://testnet.zec.rocks:443).
//!
//! These need no funds. The funded payment flow (send/confirm/receive/restore)
//! is scripts/testnet_e2e.sh, because it must wait on real block times.

use zcash_client_backend::data_api::{AccountBirthday, WalletRead};
use zcash_client_backend::proto::service;
use zcash_protocol::consensus::{BlockHeight, NetworkUpgrade, Parameters};

use hushush_wallet_proof::{NETWORK, observer, remote, secrets, sync, wallet};

async fn client() -> remote::Client {
    remote::connect(&remote::server_url()).await.expect("connect to lightwalletd")
}

#[tokio::test]
#[ignore = "live testnet"]
async fn live_server_is_testnet_and_current() {
    let mut c = client().await;
    let info = remote::check_testnet(&mut c).await.unwrap();
    let tip = remote::chain_tip(&mut c).await.unwrap();
    let nu6_3 = u32::from(NETWORK.activation_height(NetworkUpgrade::Nu6_3).unwrap());
    eprintln!("lightwalletd {} tip {} (NU6.3 at {})", info.version, tip, nu6_3);
    assert!(tip > nu6_3, "testnet tip should be past NU6.3 activation");
}

/// Decodes real recent testnet transactions with NO keys, proving our observer
/// parses current-format (post-Ironwood) transactions as a third party would.
#[tokio::test]
#[ignore = "live testnet"]
async fn live_observer_decodes_recent_transactions() {
    let mut c = client().await;
    let tip = remote::chain_tip(&mut c).await.unwrap();
    let mut decoded = 0;
    let mut shielded = 0;
    for h in (tip.saturating_sub(300)..=tip).rev() {
        let block = c
            .get_block(service::BlockId { height: h.into(), ..Default::default() })
            .await
            .unwrap()
            .into_inner();
        for ctx in block.vtx {
            let mut txid = [0u8; 32];
            txid.copy_from_slice(&ctx.txid);
            let Some((tx, mined, raw)) = wallet::fetch_transaction(
                &mut c, &NETWORK, BlockHeight::from_u32(h), zcash_primitives::transaction::TxId::from_bytes(txid),
            )
            .await
            .unwrap() else { continue };
            assert_eq!(mined, Some(BlockHeight::from_u32(h)));
            let v = observer::observe(&tx, raw.len(), Some(h));
            decoded += 1;
            if v.orchard_actions + v.ironwood_actions + v.sapling_outputs > 0 {
                shielded += 1;
                eprintln!("{}", serde_json::to_string(&v).unwrap());
            }
        }
        if shielded >= 3 {
            break;
        }
    }
    eprintln!("decoded {decoded} transactions ({shielded} with shielded components)");
    assert!(decoded > 0);
}

/// Creates a brand-new wallet against the live chain, syncs it from a recent
/// birthday, closes it, reopens it, and checks the state survived.
#[tokio::test]
#[ignore = "live testnet"]
async fn live_fresh_wallet_sync_and_persistence() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("data.sqlite");
    let mut c = client().await;
    let tip = remote::chain_tip(&mut c).await.unwrap();
    let ts = c
        .get_tree_state(service::BlockId { height: (tip - 200).into(), ..Default::default() })
        .await
        .unwrap()
        .into_inner();
    let bday = AccountBirthday::from_treestate(ts, None).unwrap();
    let seed = secrets::seed_from_mnemonic(&secrets::generate_mnemonic());
    let (mut db, acct, _) = wallet::create(&db_path, "live", &seed, &bday).unwrap();
    let ua = wallet::receive_address(&mut db, acct).unwrap();

    sync::run(&mut c, &mut db).await.unwrap();
    let scanned = db.block_max_scanned().unwrap().unwrap().block_height();
    assert!(u32::from(scanned) >= tip, "scanned {scanned:?} < tip {tip}");
    let bal = wallet::balance(&db, acct).unwrap().unwrap();
    assert_eq!(bal.total, 0);
    drop(db);

    let mut db = wallet::open(&db_path).unwrap();
    assert_eq!(wallet::sole_account(&db).unwrap(), acct);
    assert_eq!(wallet::receive_address(&mut db, acct).unwrap(), ua);
    assert!(db.block_max_scanned().unwrap().unwrap().block_height() >= scanned);
    // A second sync after restart resumes rather than rescanning from birthday.
    sync::run(&mut c, &mut db).await.unwrap();
}

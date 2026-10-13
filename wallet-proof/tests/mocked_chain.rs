//! MOCKED TESTS — no network, no real chain.
//!
//! These run hushush's payment code (`payment::*`, `observer::*`) against the
//! librustzcash test harness: a temporary sqlite wallet, and a fake chain of
//! compact blocks generated in-process. Note decryption, Merkle witnesses,
//! zk-proof generation (Halo 2 / Groth16) and transaction serialisation are
//! REAL; consensus validation by a node, mempool acceptance, mining and
//! lightwalletd are NOT exercised. See tests/testnet_live.rs for those.

use std::collections::HashMap;

use zcash_client_backend::data_api::{
    Account as _, WalletRead,
    testing::{AddressType, IronwoodFvk, TestBuilder},
    wallet::ConfirmationsPolicy,
};
use zcash_client_backend::TransferType;
use zcash_client_backend::decrypt_transaction;
use zcash_client_sqlite::testing::{BlockCache, db::TestDbFactory};
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_primitives::block::BlockHash;
use zcash_protocol::consensus::BlockHeight;
use zcash_protocol::local_consensus::LocalNetwork;
use zcash_protocol::memo::{Memo, MemoBytes};
use zcash_protocol::value::Zatoshis;

use hushush_wallet_proof::{observer, payment};

const FUNDING: u64 = 1_000_000;
const PAY: u64 = 123_456;

/// Recipient wallet with an independent seed.
fn recipient(params: &LocalNetwork, seed_byte: u8) -> (UnifiedSpendingKey, String) {
    let usk = UnifiedSpendingKey::from_seed(params, &[seed_byte; 32], zip32::AccountId::ZERO).unwrap();
    let ua = usk
        .to_unified_full_viewing_key()
        .default_address(UnifiedAddressRequest::SHIELDED)
        .unwrap()
        .0;
    (usk, ua.encode(params))
}

fn ironwood_network() -> LocalNetwork {
    let h = Some(BlockHeight::from_u32(100_000));
    LocalNetwork {
        nu6: h,
        nu6_1: h,
        nu6_2: h,
        nu6_3: h,
        ..TestBuilder::DEFAULT_NETWORK
    }
}

/// Mirrors current testnet: NU6.3 (Ironwood) and NU7 both active.
fn nu7_network() -> LocalNetwork {
    LocalNetwork {
        nu7: Some(BlockHeight::from_u32(100_000)),
        ..ironwood_network()
    }
}

/// Sum of values (and memos) that `usk` can decrypt as *incoming* in `tx`.
fn incoming_for(
    params: &LocalNetwork,
    tx: &zcash_primitives::transaction::Transaction,
    usk: &UnifiedSpendingKey,
) -> (u64, Vec<MemoBytes>) {
    let ufvks: HashMap<u32, _> = [(0u32, usk.to_unified_full_viewing_key())].into();
    let d = decrypt_transaction(params, None, None, tx, &ufvks);
    let mut total = 0;
    let mut memos = vec![];
    for o in d.sapling_outputs() {
        if o.transfer_type() == TransferType::Incoming {
            total += o.note().value().inner();
            memos.push(o.memo().clone());
        }
    }
    for o in d.orchard_outputs().iter().chain(d.ironwood_outputs()) {
        if o.transfer_type() == TransferType::Incoming {
            total += o.note().0.value().inner();
            memos.push(o.memo().clone());
        }
    }
    (total, memos)
}

/// Funds the test account via `fund`, pays PAY with a memo to a fresh
/// recipient, and checks everything observable about the result.
fn run_payment_case(
    network: LocalNetwork,
    fund: impl FnOnce(&mut zcash_client_backend::data_api::testing::TestState<BlockCache, zcash_client_sqlite::testing::db::TestDb, LocalNetwork>) -> BlockHeight,
    expect_pool_crossing: bool,
) -> observer::ObserverView {
    let mut st = TestBuilder::new()
        .with_network(network)
        .with_data_store_factory(TestDbFactory::default())
        .with_block_cache(BlockCache::new())
        .with_account_from_sapling_activation(BlockHash([0; 32]))
        .build();
    let h = fund(&mut st);
    st.scan_cached_blocks(h, 1);
    let account = st.test_account().cloned().unwrap();
    let sender_usk = account.usk().clone();
    assert_eq!(st.get_total_balance(account.id()), Zatoshis::const_from_u64(FUNDING));

    let params = *st.network();
    let (rcpt_usk, rcpt_ua) = recipient(&params, 0x42);
    let to = payment::validate_recipient(&params, &rcpt_ua).unwrap();
    let memo = payment::parse_memo(Some("hushush phase0 payout #1")).unwrap();

    let db = st.wallet_mut().db_mut();
    let proposal = payment::propose_shielded_payment(
        db,
        &params,
        account.id(),
        to,
        Zatoshis::const_from_u64(PAY),
        memo.clone(),
        ConfirmationsPolicy::MIN,
    )
    .unwrap();
    let summary = payment::summarize(&proposal);
    assert!(!summary.payment_pools.iter().any(|p| p.contains("Transparent")));
    let fee = summary.fee_zats;

    let (txid, raw) = payment::build(db, &params, &sender_usk, &proposal).unwrap();
    let tx = st.wallet().get_transaction(txid).unwrap().unwrap();

    // Recipient (holding its own key) sees exactly the amount and memo.
    let (got, memos) = incoming_for(&params, &tx, &rcpt_usk);
    assert_eq!(got, PAY, "recipient must receive exactly the requested amount");
    assert_eq!(memos, vec![memo.unwrap()]);

    // An unrelated key learns nothing.
    let (stranger_usk, _) = recipient(&params, 0x99);
    assert_eq!(incoming_for(&params, &tx, &stranger_usk).0, 0);

    // A keyless observer sees no transparent components and only the fee.
    let view = observer::observe(&tx, raw.len(), None);
    assert!(view.fully_shielded, "{view:#?}");
    assert_eq!(view.transparent_inputs, 0);
    assert!(view.transparent_outputs.is_empty());
    assert_eq!(view.fee_zats, Some(fee as i64));
    let raw_hex = hex::encode(&raw);
    assert!(
        !raw_hex.contains(&hex::encode(PAY.to_le_bytes())),
        "payment amount must not appear in cleartext"
    );
    let crossing = view.disclosures.iter().any(|d| d.starts_with("Pool crossing"));
    assert_eq!(crossing, expect_pool_crossing, "{view:#?}");

    // Mine it and confirm the sender's wallet state.
    let (h2, _) = st.generate_next_block_including(txid);
    st.scan_cached_blocks(h2, 1);
    assert_eq!(st.wallet().get_tx_height(txid).unwrap(), Some(h2));
    assert_eq!(
        st.get_total_balance(account.id()),
        Zatoshis::const_from_u64(FUNDING - PAY - fee)
    );
    let _ = account.ufvk();
    view
}

#[test]
fn mocked_orchard_to_orchard_payment() {
    let v = run_payment_case(
        TestBuilder::DEFAULT_NETWORK,
        |st| {
            let fvk = st.test_account_orchard().cloned().unwrap();
            st.generate_next_block(&fvk, AddressType::DefaultExternal, Zatoshis::const_from_u64(FUNDING)).0
        },
        false,
    );
    assert!(v.orchard_actions >= 2);
    assert_eq!(v.sapling_spends + v.sapling_outputs, 0);
}

#[test]
fn mocked_ironwood_active_payment_from_ironwood_note() {
    let v = run_payment_case(
        ironwood_network(),
        |st| {
            let fvk = IronwoodFvk(st.test_account_orchard().cloned().unwrap());
            st.generate_next_block(&fvk, AddressType::DefaultExternal, Zatoshis::const_from_u64(FUNDING)).0
        },
        false,
    );
    assert!(v.ironwood_actions >= 2, "{v:#?}");
    assert_eq!(v.orchard_actions, 0);
}

#[test]
fn mocked_sapling_funds_to_unified_recipient_reveals_pool_crossing() {
    // Paying from Sapling to a UA with an Orchard receiver moves value across
    // pools; the observer view must flag that the crossing amount is public.
    let v = run_payment_case(
        TestBuilder::DEFAULT_NETWORK,
        |st| {
            let dfvk = st.test_account_sapling().cloned().unwrap();
            st.generate_next_block(&dfvk, AddressType::DefaultExternal, Zatoshis::const_from_u64(FUNDING)).0
        },
        true,
    );
    assert!(v.sapling_spends >= 1);
}

#[test]
fn mocked_insufficient_funds_is_rejected() {
    let mut st = TestBuilder::new()
        .with_data_store_factory(TestDbFactory::default())
        .with_block_cache(BlockCache::new())
        .with_account_from_sapling_activation(BlockHash([0; 32]))
        .build();
    let fvk = st.test_account_orchard().cloned().unwrap();
    let (h, _, _) = st.generate_next_block(&fvk, AddressType::DefaultExternal, Zatoshis::const_from_u64(50_000));
    st.scan_cached_blocks(h, 1);
    let account = st.test_account().cloned().unwrap();
    let params = *st.network();
    let (_, ua) = recipient(&params, 1);
    let to = payment::validate_recipient(&params, &ua).unwrap();
    let r = payment::propose_shielded_payment(
        st.wallet_mut().db_mut(),
        &params,
        account.id(),
        to,
        Zatoshis::const_from_u64(50_000), // + fee > balance
        None,
        ConfirmationsPolicy::MIN,
    );
    assert!(r.is_err());
}

#[test]
fn mocked_unconfirmed_funds_not_spendable_under_default_policy() {
    let mut st = TestBuilder::new()
        .with_data_store_factory(TestDbFactory::default())
        .with_block_cache(BlockCache::new())
        .with_account_from_sapling_activation(BlockHash([0; 32]))
        .build();
    let fvk = st.test_account_orchard().cloned().unwrap();
    let (h, _, _) = st.generate_next_block(&fvk, AddressType::DefaultExternal, Zatoshis::const_from_u64(FUNDING));
    st.scan_cached_blocks(h, 1);
    let account = st.test_account().cloned().unwrap();
    let params = *st.network();
    let (_, ua) = recipient(&params, 1);
    let to = payment::validate_recipient(&params, &ua).unwrap();
    // Default policy requires multiple confirmations; 1 block is not enough.
    assert!(
        payment::propose_shielded_payment(
            st.wallet_mut().db_mut(), &params, account.id(), to, Zatoshis::const_from_u64(PAY),
            None, ConfirmationsPolicy::default(),
        )
        .is_err()
    );
    let _ = Memo::Empty;
}

#[test]
fn mocked_nu7_active_payment_uses_v6_nu7_format() {
    // Testnet activated NU7 at 4,465,026 (branch 0x77190AD9). Transactions must
    // be built at the NU7 branch with the v6 format or nodes reject them.
    let v = run_payment_case(
        nu7_network(),
        |st| {
            let fvk = IronwoodFvk(st.test_account_orchard().cloned().unwrap());
            st.generate_next_block(&fvk, AddressType::DefaultExternal, Zatoshis::const_from_u64(FUNDING)).0
        },
        false,
    );
    assert_eq!(v.consensus_branch_id, "Nu7", "{v:#?}");
    assert_eq!(v.tx_version, "V6", "{v:#?}");
    assert!(v.ironwood_actions >= 2);
}

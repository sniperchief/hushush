//! OFFLINE TESTS (no network, real testnet parameters, real sqlite files).
//!
//! Persistence across restarts and seed-based recovery of the wallet's keys
//! and addresses. Chain-state recovery (rescanning and finding funds) needs the
//! network; that is covered by tests/testnet_live.rs.

use secrecy::SecretString;
use zcash_client_backend::data_api::{AccountBirthday, WalletRead, chain::ChainState};
use zcash_primitives::block::BlockHash;
use zcash_protocol::consensus::{NetworkUpgrade, Parameters};

use hushush_wallet_proof::{NETWORK, config::WalletPaths, secrets, wallet};

fn offline_birthday() -> AccountBirthday {
    let sapling = NETWORK.activation_height(NetworkUpgrade::Sapling).unwrap();
    AccountBirthday::from_parts(ChainState::empty(sapling - 1, BlockHash([0; 32])), None)
}

fn create_at(dir: &std::path::Path, name: &str, m: &bip0039::Mnemonic) -> (String, String) {
    let paths = WalletPaths::in_roots(name, dir, &dir.join("secrets")).unwrap();
    let seed = secrets::seed_from_mnemonic(m);
    let (mut db, acct, _) = wallet::create(&paths.data_db, name, &seed, &offline_birthday()).unwrap();
    let ua = wallet::receive_address(&mut db, acct).unwrap().encode(&NETWORK);
    let ufvk = db.get_unified_full_viewing_keys().unwrap()[&acct].encode(&NETWORK).unwrap();
    (ua, ufvk)
}

#[test]
fn wallet_state_persists_across_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let m = secrets::generate_mnemonic();
    let (ua, ufvk) = create_at(tmp.path(), "persist", &m);

    // "Restart": open the same DB from scratch in a new handle.
    let paths = WalletPaths::in_roots("persist", tmp.path(), &tmp.path().join("secrets")).unwrap();
    let mut db = wallet::open(&paths.data_db).unwrap();
    let acct = wallet::sole_account(&db).unwrap();
    assert_eq!(wallet::receive_address(&mut db, acct).unwrap().encode(&NETWORK), ua);
    assert_eq!(db.get_unified_full_viewing_keys().unwrap()[&acct].encode(&NETWORK).unwrap(), ufvk);
    assert_eq!(
        db.get_account_birthday(acct).unwrap(),
        offline_birthday().height()
    );
}

#[test]
fn restore_from_mnemonic_reproduces_keys_and_address() {
    let tmp = tempfile::tempdir().unwrap();
    let m = secrets::generate_mnemonic();
    let original = create_at(tmp.path(), "orig", &m);

    // Simulate total loss of the wallet directory: only the phrase survives.
    let phrase = SecretString::new(format!("  {}\n", m.phrase()));
    let restored_m = secrets::parse_mnemonic(&phrase).unwrap();
    let restored = create_at(tmp.path(), "restored", &restored_m);
    assert_eq!(original, restored);

    let other = create_at(tmp.path(), "other", &secrets::generate_mnemonic());
    assert_ne!(original, other);
}

#[test]
fn spending_key_requires_matching_seed() {
    let tmp = tempfile::tempdir().unwrap();
    let m = secrets::generate_mnemonic();
    create_at(tmp.path(), "w", &m);
    let paths = WalletPaths::in_roots("w", tmp.path(), &tmp.path().join("secrets")).unwrap();
    let db = wallet::open(&paths.data_db).unwrap();
    let acct = wallet::sole_account(&db).unwrap();
    assert!(wallet::spending_key(&db, acct, &secrets::seed_from_mnemonic(&m)).is_ok());
    let wrong = secrets::seed_from_mnemonic(&secrets::generate_mnemonic());
    assert!(wallet::spending_key(&db, acct, &wrong).is_err());
}

#[test]
fn encrypted_mnemonic_plus_identity_recovers_spend_authority() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = WalletPaths::in_roots("enc", tmp.path(), &tmp.path().join("secrets")).unwrap();
    std::fs::create_dir_all(&paths.dir).unwrap();
    let m = secrets::generate_mnemonic();
    let id = secrets::create_identity(&paths.identity).unwrap();
    secrets::write_private_file(
        &paths.encrypted_mnemonic,
        &secrets::encrypt_mnemonic(&m, &id.to_public()).unwrap(),
    )
    .unwrap();
    let back = secrets::load_mnemonic(&paths.encrypted_mnemonic, &paths.identity).unwrap();
    assert_eq!(back.phrase(), m.phrase());

    // Without the identity file the encrypted mnemonic is useless.
    std::fs::remove_file(&paths.identity).unwrap();
    assert!(secrets::load_mnemonic(&paths.encrypted_mnemonic, &paths.identity).is_err());
}

#[test]
fn refuses_wallet_inside_git_repo() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".git")).unwrap();
    let paths = WalletPaths::in_roots("x", &tmp.path().join("w"), &tmp.path().join("s")).unwrap();
    assert!(paths.ensure_outside_source_control().is_err());
}

#[test]
fn rejects_bad_wallet_names() {
    for bad in ["", "../etc", "a/b", "a b"] {
        assert!(WalletPaths::in_roots(bad, "w".as_ref(), "s".as_ref()).is_err(), "{bad}");
    }
}

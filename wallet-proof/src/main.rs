//! `hushush-wallet-proof` — Phase 0 testnet CLI.
//!
//! Every command is a separate process: wallet state is reloaded from disk each
//! time, so running the flow end to end also exercises persistence across
//! restarts.

use std::io::BufRead;
use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use secrecy::SecretString;
use zcash_client_backend::data_api::{AccountBirthday, WalletRead, wallet::ConfirmationsPolicy};
use zcash_client_backend::proto::service;
use zcash_protocol::{consensus::BlockHeight, value::Zatoshis};

use hushush_wallet_proof::{
    NETWORK,
    config::{WalletPaths, enclosing_git_repo},
    observer, payment, remote, secrets, sync, wallet,
};

#[derive(Parser)]
#[command(about = "hushush Phase 0: shielded ZEC payment feasibility proof (TESTNET ONLY)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show lightwalletd server info and chain tip.
    Info,
    /// Create a new wallet with a fresh 24-word mnemonic.
    Init {
        name: String,
        /// Birthday height (default: chain tip - 100).
        #[arg(long)]
        birthday: Option<u32>,
    },
    /// Restore a wallet from a mnemonic (read from a file, or stdin with `-`).
    Restore {
        name: String,
        #[arg(long)]
        mnemonic_file: PathBuf,
        /// Birthday height; scanning starts here. Use the original wallet's birthday.
        #[arg(long)]
        birthday: u32,
    },
    /// Write the decrypted mnemonic to a NEW file outside any git repository.
    BackupMnemonic {
        name: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Print the wallet's shielded-only Unified Address.
    Address {
        name: String,
        /// Print only the Sapling receiver (for services that reject Unified Addresses).
        #[arg(long)]
        sapling: bool,
    },
    /// Print the account's birthday height (needed for restore).
    Birthday { name: String },
    /// Print the account's Unified Full Viewing Key (view-only; cannot spend).
    Ufvk { name: String },
    /// Sync with the chain and fetch full transactions (memos, sent status).
    Sync { name: String },
    /// Show incoming payments still in the mempool (unmined).
    Pending { name: String },
    /// Show balances (requires a prior sync).
    Balance { name: String },
    /// List wallet transactions.
    History { name: String },
    /// Send a shielded payment.
    Send {
        name: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        zats: u64,
        #[arg(long)]
        memo: Option<String>,
        /// Only print the proposal; do not build or broadcast.
        #[arg(long)]
        dry_run: bool,
    },
    /// Confirmation status of a transaction according to the wallet.
    Status { name: String, txid: String },
    /// Keyless observer view of a transaction, fetched from lightwalletd.
    Observe { txid: String },
}

fn json<T: serde::Serialize>(v: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

async fn connect() -> anyhow::Result<remote::Client> {
    let url = remote::server_url();
    let mut c = remote::connect(&url).await?;
    remote::check_testnet(&mut c).await?;
    Ok(c)
}

async fn birthday_at(client: &mut remote::Client, height: u32) -> anyhow::Result<AccountBirthday> {
    // NOTE: requesting the tree state discloses the approximate birthday to the
    // lightwalletd operator.
    let ts = client
        .get_tree_state(service::BlockId {
            height: u64::from(height.saturating_sub(1)),
            ..Default::default()
        })
        .await?
        .into_inner();
    AccountBirthday::from_treestate(ts, None).map_err(|e| anyhow::anyhow!("bad tree state: {e:?}"))
}

fn create_wallet(
    paths: &WalletPaths,
    mnemonic: &bip0039::Mnemonic<bip0039::English>,
    birthday: &AccountBirthday,
) -> anyhow::Result<()> {
    paths.ensure_outside_source_control()?;
    if paths.dir.exists() || paths.identity.exists() {
        bail!("wallet '{}' already exists", paths.name);
    }
    std::fs::create_dir_all(&paths.dir)?;
    let id = secrets::create_identity(&paths.identity)?;
    let ct = secrets::encrypt_mnemonic(mnemonic, &id.to_public())?;
    secrets::write_private_file(&paths.encrypted_mnemonic, &ct)?;
    let seed = secrets::seed_from_mnemonic(mnemonic);
    let (mut db, account, _usk) = wallet::create(&paths.data_db, &paths.name, &seed, birthday)?;
    let ua = wallet::receive_address(&mut db, account)?;
    eprintln!("wallet:   {}", paths.dir.display());
    eprintln!("identity: {} (keep secret; required to spend)", paths.identity.display());
    eprintln!("birthday: {}", u32::from(birthday.height()));
    println!("{}", ua.encode(&NETWORK));
    Ok(())
}

/// Halo 2 proving recurses deeply; Windows gives the main thread only 1 MiB of
/// stack, which overflows while proving. Run everything on large-stack threads.
const STACK_SIZE: usize = 64 * 1024 * 1024;

fn main() -> anyhow::Result<()> {
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(STACK_SIZE)
                .build()?
                .block_on(run())
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("main thread panicked"))?
}

async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,zcash_client_backend::sync=info".into()),
        )
        .init();

    match Cli::parse().cmd {
        Cmd::Info => {
            let mut c = remote::connect(&remote::server_url()).await?;
            let info = remote::check_testnet(&mut c).await?;
            println!(
                "server={} chain={} version={} block_height={} sapling_activation={} branch_id={}",
                remote::server_url(),
                info.chain_name,
                info.version,
                info.block_height,
                info.sapling_activation_height,
                info.consensus_branch_id
            );
        }
        Cmd::Init { name, birthday } => {
            let paths = WalletPaths::for_name(&name)?;
            let mut c = connect().await?;
            let h = match birthday {
                Some(h) => h,
                None => remote::chain_tip(&mut c).await?.saturating_sub(100),
            };
            let bday = birthday_at(&mut c, h).await?;
            create_wallet(&paths, &secrets::generate_mnemonic(), &bday)?;
        }
        Cmd::Restore { name, mnemonic_file, birthday } => {
            let paths = WalletPaths::for_name(&name)?;
            let phrase = if mnemonic_file.as_os_str() == "-" {
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                SecretString::new(line)
            } else {
                SecretString::new(std::fs::read_to_string(&mnemonic_file)?)
            };
            let mnemonic = secrets::parse_mnemonic(&phrase)?;
            let mut c = connect().await?;
            let bday = birthday_at(&mut c, birthday).await?;
            create_wallet(&paths, &mnemonic, &bday)?;
        }
        Cmd::BackupMnemonic { name, out } => {
            let paths = WalletPaths::for_name(&name)?;
            if let Some(repo) = enclosing_git_repo(&out) {
                bail!("refusing to write a mnemonic inside git repository {}", repo.display());
            }
            let m = secrets::load_mnemonic(&paths.encrypted_mnemonic, &paths.identity)?;
            secrets::write_private_file(&out, format!("{}\n", m.phrase()).as_bytes())?;
            eprintln!("mnemonic written to {} — store offline, then delete this file", out.display());
        }
        Cmd::Address { name, sapling } => {
            let paths = WalletPaths::for_name(&name)?;
            let mut db = wallet::open(&paths.data_db)?;
            let acct = wallet::sole_account(&db)?;
            let ua = wallet::receive_address(&mut db, acct)?;
            if sapling {
                let pa = ua.sapling().context("address has no Sapling receiver")?;
                println!("{}", zcash_keys::address::Address::Sapling(Box::new(*pa)).encode(&NETWORK));
            } else {
                println!("{}", ua.encode(&NETWORK));
            }
        }
        Cmd::Birthday { name } => {
            let paths = WalletPaths::for_name(&name)?;
            let db = wallet::open(&paths.data_db)?;
            let acct = wallet::sole_account(&db)?;
            println!("{}", u32::from(db.get_account_birthday(acct)?));
        }
        Cmd::Ufvk { name } => {
            let paths = WalletPaths::for_name(&name)?;
            let db = wallet::open(&paths.data_db)?;
            let acct = wallet::sole_account(&db)?;
            let ufvks = db.get_unified_full_viewing_keys()?;
            println!("{}", ufvks.get(&acct).context("no UFVK")?.encode(&NETWORK)?);
        }
        Cmd::Sync { name } => {
            let paths = WalletPaths::for_name(&name)?;
            let mut db = wallet::open(&paths.data_db)?;
            let mut c = connect().await?;
            sync::run(&mut c, &mut db).await?;
            let n = wallet::enhance(&mut db, &mut c).await?;
            eprintln!("synced; serviced {n} transaction data request(s)");
            let acct = wallet::sole_account(&db)?;
            json(&wallet::balance(&db, acct)?)?;
        }
        Cmd::Pending { name } => {
            let paths = WalletPaths::for_name(&name)?;
            let db = wallet::open(&paths.data_db)?;
            let mut c = connect().await?;
            let (seen, found) = wallet::pending_incoming(&db, &mut c).await?;
            eprintln!("scanned {seen} mempool transaction(s)");
            json(&found)?;
        }
        Cmd::Balance { name } => {
            let paths = WalletPaths::for_name(&name)?;
            let db = wallet::open(&paths.data_db)?;
            let acct = wallet::sole_account(&db)?;
            json(&wallet::balance(&db, acct)?)?;
        }
        Cmd::History { name } => {
            let paths = WalletPaths::for_name(&name)?;
            json(&wallet::history(&paths.data_db)?)?;
        }
        Cmd::Send { name, to, zats, memo, dry_run } => {
            let paths = WalletPaths::for_name(&name)?;
            let to = payment::validate_recipient(&NETWORK, &to)?;
            let amount = Zatoshis::from_u64(zats).map_err(|_| anyhow::anyhow!("invalid amount"))?;
            let memo = payment::parse_memo(memo.as_deref())?;

            let mut c = connect().await?;
            let mut db = wallet::open(&paths.data_db)?;
            // Always sync first so the proposal uses current note/anchor state.
            sync::run(&mut c, &mut db).await?;
            wallet::enhance(&mut db, &mut c).await?;
            let acct = wallet::sole_account(&db)?;

            let proposal = payment::propose_shielded_payment(
                &mut db, &NETWORK, acct, to, amount, memo, ConfirmationsPolicy::default(),
            )?;
            eprintln!("proposal: {}", serde_json::to_string(&payment::summarize(&proposal))?);
            if dry_run {
                return Ok(());
            }
            let mnemonic = secrets::load_mnemonic(&paths.encrypted_mnemonic, &paths.identity)?;
            let seed = secrets::seed_from_mnemonic(&mnemonic);
            drop(mnemonic);
            let usk = wallet::spending_key(&db, acct, &seed)?;
            drop(seed);
            eprintln!("building and proving transaction...");
            let (txid, raw) = payment::build(&mut db, &NETWORK, &usk, &proposal)?;
            drop(usk);
            eprintln!("broadcasting {} bytes...", raw.len());
            payment::broadcast(&mut c, raw).await?;
            println!("{txid}");
        }
        Cmd::Status { name, txid } => {
            let paths = WalletPaths::for_name(&name)?;
            let db = wallet::open(&paths.data_db)?;
            json(&wallet::tx_status(&db, wallet::parse_txid(&txid)?)?)?;
        }
        Cmd::Observe { txid } => {
            let mut c = connect().await?;
            let tip = remote::chain_tip(&mut c).await?;
            let (tx, h, raw) = wallet::fetch_transaction(
                &mut c, &NETWORK, BlockHeight::from_u32(tip), wallet::parse_txid(&txid)?,
            )
            .await?
            .context("transaction not found by lightwalletd")?;
            let mut view = observer::observe(&tx, raw.len(), h.map(u32::from));
            if let Some(h) = h {
                view.disclosures.push(format!("Confirmations: {}", tip - u32::from(h) + 1));
            }
            json(&view)?;
        }
    }
    Ok(())
}

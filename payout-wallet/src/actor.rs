//! The wallet actor: one thread owns the wallet DB, the lightwalletd
//! connection used for spending, and the intent store, and handles every
//! wallet-mutating command strictly one at a time. That makes "check the
//! intent, build, record, broadcast" linearizable without further locking,
//! and keeps the (non-`Send`) upstream wallet futures off the HTTP runtime.
//!
//! All transaction logic is the Phase 0 code in `hushush_wallet_proof`:
//! `sync::run`, `wallet::enhance`, `payment::propose_shielded_payment`,
//! `payment::build` (upstream `create_proposed_transactions`, bundled
//! prover) and `payment::broadcast`. Nothing here implements cryptography.

use std::time::Duration;

use anyhow::Context;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use zcash_client_backend::data_api::{WalletRead, wallet::ConfirmationsPolicy};
use zcash_client_sqlite::AccountUuid;
use zcash_protocol::{consensus::BlockHeight, value::Zatoshis};

use hushush_wallet_proof::{NETWORK, config::WalletPaths, payment, remote, secrets, sync, wallet};

use crate::chain;
use crate::intents::{Intent, IntentState, IntentStore, memo_for, now};
use crate::policy::{self, pool_name};

pub struct ActorConfig {
    pub paths: WalletPaths,
    pub intents_path: std::path::PathBuf,
    pub lightwalletd_url: String,
    pub max_payment_zats: i64,
    pub max_fee_zats: i64,
    pub sync_interval: Duration,
    /// Test-only crash point (feature `fault-injection`).
    pub fault: Option<String>,
    /// False while lightwalletd fails its network/branch check; lookups then
    /// refuse to report chain status, so the payroll side never decides on it.
    pub chain_ok: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug, Serialize)]
pub struct StatusView {
    pub network: &'static str,
    pub wallet: String,
    pub lightwalletd: String,
    pub chain_tip: Option<u32>,
    pub fully_scanned_height: Option<u32>,
    pub last_sync_at: Option<i64>,
    pub last_sync_error: Option<String>,
    pub spendable_zats: u64,
    pub pending_zats: u64,
    pub total_zats: u64,
    pub spendable_by_pool: SpendableByPool,
    /// Shielded-only Unified Address that funds this payout wallet.
    pub funding_address: String,
    pub max_payment_zats: i64,
    pub unfinished_payments: i64,
}

#[derive(Debug, Serialize)]
pub struct SpendableByPool {
    pub sapling: u64,
    pub orchard: u64,
    pub ironwood: u64,
}

#[derive(Debug, Serialize)]
pub struct QuoteView {
    pub valid_recipient: bool,
    pub amount_zats: i64,
    /// ZIP 317 fee of the transaction the wallet would build now.
    pub fee_zats: Option<i64>,
    pub spendable_zats: u64,
    pub pending_zats: u64,
    pub sufficient: bool,
    pub pool: Option<&'static str>,
    pub reason: Option<String>,
}

pub struct SubmitReq {
    pub payment_id: String,
    pub address: String,
    pub amount_zats: i64,
    pub max_fee_zats: i64,
}

#[derive(Debug)]
pub enum CmdError {
    /// Payment id reused with different parameters.
    Conflict(String),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for CmdError {
    fn from(e: anyhow::Error) -> Self {
        CmdError::Internal(e)
    }
}

type Reply<T> = oneshot::Sender<Result<T, CmdError>>;

pub enum Cmd {
    Status(Reply<StatusView>),
    Quote {
        address: String,
        amount_zats: i64,
        reply: Reply<QuoteView>,
    },
    Submit {
        req: SubmitReq,
        reply: Reply<Intent>,
    },
    Cancel {
        payment_id: String,
        reply: Reply<Intent>,
    },
}

/// Facts the HTTP side needs, produced once the actor is ready.
pub struct Ready {
    pub account: AccountUuid,
    pub ufvks: std::collections::HashMap<AccountUuid, zcash_keys::keys::UnifiedFullViewingKey>,
}

struct Actor {
    cfg: ActorConfig,
    db: wallet::Db,
    client: remote::Client,
    intents: IntentStore,
    account: AccountUuid,
    funding_address: String,
    last_sync_at: Option<i64>,
    last_sync_error: Option<String>,
}

enum BuildError {
    /// Nothing was built: safe to record the payment as failed.
    NoTx(String),
    /// Building failed in a way that might have left a transaction behind.
    Unknown(String),
}

struct Built {
    txid: String,
    raw: Vec<u8>,
    fee: i64,
    expiry: Option<i64>,
    /// Recipient and amount as the wallet DB recorded them (for adoption).
    to_address: Option<String>,
    amount: i64,
}

/// Starts the actor thread (64 MiB stack: Halo 2 proving recurses deeply).
/// Returns its command channel once start-up recovery has finished.
pub fn spawn(cfg: ActorConfig) -> anyhow::Result<(mpsc::Sender<Cmd>, Ready)> {
    let (tx, rx) = mpsc::channel::<Cmd>(64);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("wallet-actor".into())
        .stack_size(crate::STACK_SIZE)
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!(e)));
                    return;
                }
            };
            rt.block_on(async move {
                let mut actor = match Actor::start(cfg).await {
                    Ok(a) => a,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let ready = Ready {
                    account: actor.account,
                    ufvks: match actor.db.get_unified_full_viewing_keys() {
                        Ok(u) => u,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e.into()));
                            return;
                        }
                    },
                };
                let _ = ready_tx.send(Ok(ready));
                actor.run(rx).await;
            });
        })?;
    let ready = ready_rx
        .recv()
        .context("wallet actor exited during start-up")??;
    Ok((tx, ready))
}

impl Actor {
    async fn start(cfg: ActorConfig) -> anyhow::Result<Self> {
        let mut client = chain::connect_with_retry(&cfg.lightwalletd_url).await?;
        let mut db = wallet::open(&cfg.paths.data_db)?;
        let account = wallet::sole_account(&db)?;
        let funding_address = wallet::receive_address(&mut db, account)?.encode(&NETWORK);
        let intents = IntentStore::open(&cfg.intents_path)?;
        sync::run(&mut client, &mut db).await?;
        wallet::enhance(&mut db, &mut client).await?;
        let mut a = Self {
            cfg,
            db,
            client,
            intents,
            account,
            funding_address,
            last_sync_at: Some(now()),
            last_sync_error: None,
        };
        a.recover().await?;
        a.rebroadcast().await;
        Ok(a)
    }

    async fn run(&mut self, mut rx: mpsc::Receiver<Cmd>) {
        let mut tick = tokio::time::interval(self.cfg.sync_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // start-up already synced
        loop {
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(cmd) => self.handle(cmd).await,
                    None => return,
                },
                _ = tick.tick() => self.background().await,
            }
        }
    }

    async fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Status(reply) => {
                let _ = reply.send(self.status().map_err(CmdError::from));
            }
            Cmd::Quote {
                address,
                amount_zats,
                reply,
            } => {
                let _ = reply.send(self.quote(&address, amount_zats).map_err(CmdError::from));
            }
            Cmd::Submit { req, reply } => {
                let _ = reply.send(self.submit(req).await);
            }
            Cmd::Cancel { payment_id, reply } => {
                let _ = reply.send(self.cancel(&payment_id).await.map_err(CmdError::from));
            }
        }
    }

    // ---- background ------------------------------------------------------------

    async fn sync(&mut self) -> anyhow::Result<()> {
        let r = async {
            sync::run(&mut self.client, &mut self.db).await?;
            wallet::enhance(&mut self.db, &mut self.client).await?;
            anyhow::Ok(())
        }
        .await;
        match &r {
            Ok(()) => {
                self.last_sync_at = Some(now());
                self.last_sync_error = None;
            }
            Err(e) => self.last_sync_error = Some(e.to_string()),
        }
        r
    }

    async fn background(&mut self) {
        let ok = chain::verify_server(&mut self.client, &self.cfg.lightwalletd_url).await;
        if let Err(e) = &ok {
            tracing::error!(
                "lightwalletd failed its network/branch check; chain status withheld: {e:#}"
            );
        }
        self.cfg
            .chain_ok
            .store(ok.is_ok(), std::sync::atomic::Ordering::SeqCst);
        if let Err(e) = self.sync().await {
            tracing::warn!("background sync failed: {e:#}");
            // The connection may be dead (public lightwalletd drops TLS).
            if let Ok(c) = chain::connect_checked(&self.cfg.lightwalletd_url).await {
                self.client = c;
            }
        }
        self.rebroadcast().await;
    }

    /// After a crash, a `pending` intent is either a transaction the wallet
    /// built but never recorded (found by its memo) or nothing at all.
    async fn recover(&mut self) -> anyhow::Result<()> {
        for i in self.intents.in_state(IntentState::Pending)? {
            match self.find_built(&i.payment_id)? {
                Some(b) => {
                    tracing::warn!(payment_id = %i.payment_id, txid = %b.txid, "recovered a built transaction after restart");
                    self.intents.mark_built(
                        &i.payment_id,
                        &b.txid,
                        &b.raw,
                        Some(b.fee),
                        b.expiry,
                    )?;
                }
                None => {
                    tracing::warn!(payment_id = %i.payment_id, "pending payment had no transaction after restart; marking failed");
                    self.intents
                        .mark_failed(&i.payment_id, "interrupted before a transaction was built")?;
                }
            }
        }
        Ok(())
    }

    /// Looks for a transaction this wallet sent for `payment_id` (by memo).
    fn find_built(&mut self, payment_id: &str) -> anyhow::Result<Option<Built>> {
        let memo = memo_for(payment_id);
        let history = wallet::history(&self.cfg.paths.data_db)?;
        let Some((h, out)) = history.into_iter().find_map(|t| {
            let o = t
                .outputs
                .iter()
                .find(|o| o.is_sent && o.memo.as_deref() == Some(memo.as_str()))
                .map(|o| (o.to_address.clone(), o.value_zats))?;
            Some((t, o))
        }) else {
            return Ok(None);
        };
        let txid = wallet::parse_txid(&h.txid)?;
        let tx = self
            .db
            .get_transaction(txid)?
            .context("wallet history lists a transaction it cannot load")?;
        let mut raw = vec![];
        tx.write(&mut raw)?;
        Ok(Some(Built {
            txid: h.txid,
            raw,
            fee: h.fee_paid_zats.unwrap_or_default(),
            expiry: Some(i64::from(u32::from(tx.expiry_height()))),
            to_address: out.0,
            amount: out.1,
        }))
    }

    /// Adopts a transaction found in the wallet DB for a payment id that has
    /// no intent record: never fence it, never build a second one.
    async fn adopt(&mut self, id: &str, b: Built) -> anyhow::Result<Intent> {
        self.intents.insert_recovered(
            id,
            b.to_address.as_deref().unwrap_or("unknown-recipient"),
            b.amount,
            &memo_for(id),
            &b.txid,
            &b.raw,
            b.fee,
            b.expiry,
        )?;
        tracing::warn!(payment_id = id, txid = %b.txid, "adopted an existing wallet transaction for a payment id with no intent record");
        let i = self.intents.get(id)?.context("intent vanished")?;
        self.broadcast(&i).await;
        self.intents.get(id)?.context("intent vanished")
    }

    /// (Re)broadcasts built transactions, and broadcast ones the node has
    /// forgotten, while they can still be mined.
    async fn rebroadcast(&mut self) {
        let Ok(tip) = remote::chain_tip(&mut self.client).await else {
            return;
        };
        let mut todo = vec![];
        for state in [IntentState::Built, IntentState::Broadcast] {
            if let Ok(v) = self.intents.in_state(state) {
                todo.extend(v);
            }
        }
        for i in todo {
            if i.expiry_height.is_some_and(|e| i64::from(tip) > e) {
                continue; // can no longer be mined; the payroll side will fail it
            }
            if i.state == IntentState::Broadcast {
                let Some(txid) = i.txid.as_deref().and_then(|t| wallet::parse_txid(t).ok()) else {
                    continue;
                };
                match chain::fetch(&mut self.client, BlockHeight::from_u32(tip), txid).await {
                    Ok(chain::Fetched::Missing) => {}
                    _ => continue, // mined, in the mempool, or unknown right now
                }
            }
            self.broadcast(&i).await;
        }
    }

    async fn broadcast(&mut self, i: &Intent) {
        let Some(raw) = i.raw_tx.clone() else { return };
        let result = payment::broadcast(&mut self.client, raw).await;
        if result.is_ok() {
            // Accepted by the network, not yet recorded as broadcast here.
            self.fault_point("crash_after_broadcast");
        }
        let err = match &result {
            Ok(()) => None,
            // The node already has it: that is an acknowledgement.
            Err(e) if format!("{e:#}").to_lowercase().contains("already") => None,
            Err(e) => Some(format!("{e:#}")),
        };
        match &err {
            None => {
                tracing::info!(payment_id = %i.payment_id, txid = ?i.txid, "broadcast accepted")
            }
            Some(e) => {
                tracing::warn!(payment_id = %i.payment_id, txid = ?i.txid, "broadcast failed: {e}")
            }
        }
        if let Err(e) = self.intents.record_broadcast(&i.payment_id, err.as_deref()) {
            tracing::error!(payment_id = %i.payment_id, "could not record broadcast: {e:#}");
        }
    }

    // ---- commands -----------------------------------------------------------------

    fn balances(&self) -> anyhow::Result<Option<wallet::BalanceReport>> {
        wallet::balance(&self.db, self.account)
    }

    fn status(&mut self) -> anyhow::Result<StatusView> {
        let b = self.balances()?;
        Ok(StatusView {
            network: "testnet",
            wallet: self.cfg.paths.name.clone(),
            lightwalletd: self.cfg.lightwalletd_url.clone(),
            chain_tip: b.as_ref().map(|b| b.chain_tip_height),
            fully_scanned_height: b.as_ref().map(|b| b.fully_scanned_height),
            last_sync_at: self.last_sync_at,
            last_sync_error: self.last_sync_error.clone(),
            spendable_zats: b.as_ref().map_or(0, |b| b.spendable_total),
            pending_zats: b
                .as_ref()
                .map_or(0, |b| b.pending_change + b.pending_spendability),
            total_zats: b.as_ref().map_or(0, |b| b.total),
            spendable_by_pool: SpendableByPool {
                sapling: b.as_ref().map_or(0, |b| b.sapling_spendable),
                orchard: b.as_ref().map_or(0, |b| b.orchard_spendable),
                ironwood: b.as_ref().map_or(0, |b| b.ironwood_spendable),
            },
            funding_address: self.funding_address.clone(),
            max_payment_zats: self.cfg.max_payment_zats,
            unfinished_payments: self.intents.count_unfinished()?,
        })
    }

    fn quote(&mut self, address: &str, amount_zats: i64) -> anyhow::Result<QuoteView> {
        let b = self.balances()?;
        let spendable = b.as_ref().map_or(0, |b| b.spendable_total);
        let pending = b
            .as_ref()
            .map_or(0, |b| b.pending_change + b.pending_spendability);
        let mut q = QuoteView {
            valid_recipient: false,
            amount_zats,
            fee_zats: None,
            spendable_zats: spendable,
            pending_zats: pending,
            sufficient: false,
            pool: None,
            reason: None,
        };
        if let Err(r) = policy::payout_recipient(address) {
            q.reason = Some(r);
            return Ok(q);
        }
        q.valid_recipient = true;
        if !(1..=self.cfg.max_payment_zats).contains(&amount_zats) {
            q.reason = Some(format!(
                "amount must be between 1 and {} zats for this payout wallet",
                self.cfg.max_payment_zats
            ));
            return Ok(q);
        }
        // A real proposal on current wallet state: the fee is the one the
        // transaction would actually pay. Nothing is stored.
        let memo = payment::parse_memo(Some(&memo_for(&uuid::Uuid::nil().to_string())))?;
        let to = payment::validate_recipient(&NETWORK, address).map_err(anyhow::Error::from)?;
        match payment::propose_shielded_payment(
            &mut self.db,
            &NETWORK,
            self.account,
            to,
            Zatoshis::from_u64(amount_zats as u64)?,
            memo,
            ConfirmationsPolicy::default(),
        ) {
            Ok(p) => {
                q.fee_zats = Some(payment::summarize(&p).fee_zats as i64);
                match policy::proposal_pool(&p) {
                    Ok(pool) => {
                        q.pool = Some(pool_name(pool));
                        q.sufficient = true;
                    }
                    Err(r) => q.reason = Some(r),
                }
            }
            Err(e) => {
                q.reason = Some(if pending > 0 {
                    format!(
                        "{e}; {pending} zats are waiting for confirmations and will become \
                         spendable shortly"
                    )
                } else {
                    e.to_string()
                })
            }
        }
        Ok(q)
    }

    async fn cancel(&mut self, payment_id: &str) -> anyhow::Result<Intent> {
        if let Some(i) = self.intents.get(payment_id)? {
            return Ok(i);
        }
        // No intent, but the wallet may still hold a transaction for this id
        // (intent store lost): a fence would wrongly declare it unpaid.
        if let Some(b) = self.find_built(payment_id)? {
            return self.adopt(payment_id, b).await;
        }
        self.intents.insert_failed(
            payment_id,
            None,
            0,
            0,
            &memo_for(payment_id),
            "cancelled before any transaction was built",
        )?;
        tracing::info!(
            payment_id,
            "payment id fenced (cancelled before submission)"
        );
        self.intents.get(payment_id)?.context("fence not recorded")
    }

    async fn submit(&mut self, req: SubmitReq) -> Result<Intent, CmdError> {
        let id = req.payment_id.as_str();
        let address = req.address.trim();
        if let Some(i) = self.intents.get(id)? {
            let same = i.address.as_deref() == Some(address)
                && i.amount_zats == req.amount_zats
                && i.max_fee_zats == req.max_fee_zats;
            if i.address.is_some() && !same {
                return Err(CmdError::Conflict(
                    "payment id was already used with different parameters".into(),
                ));
            }
            // Idempotent replay: never build again; re-send if unacknowledged.
            if i.state == IntentState::Built {
                self.broadcast(&i).await;
            }
            return Ok(self.intents.get(id)?.context("intent vanished")?);
        }

        if let Some(b) = self.find_built(id)? {
            return Ok(self.adopt(id, b).await?);
        }

        let memo = memo_for(id);
        let reject = |this: &mut Self, reason: String| -> Result<Intent, CmdError> {
            this.intents.insert_failed(
                id,
                Some(address),
                req.amount_zats,
                req.max_fee_zats,
                &memo,
                &reason,
            )?;
            tracing::info!(payment_id = id, "payment refused: {reason}");
            Ok(this.intents.get(id)?.context("intent vanished")?)
        };
        if !(1..=self.cfg.max_payment_zats).contains(&req.amount_zats) {
            let r = format!(
                "amount must be between 1 and {} zats for this payout wallet",
                self.cfg.max_payment_zats
            );
            return reject(self, r);
        }
        if !(0..=self.cfg.max_fee_zats).contains(&req.max_fee_zats) {
            return reject(self, "max fee is out of range".into());
        }
        if let Err(r) = policy::payout_recipient(address) {
            return reject(self, r);
        }

        // Durable record BEFORE any wallet work.
        self.intents
            .insert_pending(id, address, req.amount_zats, req.max_fee_zats, &memo)?;
        self.fault_point("crash_before_build");
        tracing::info!(
            payment_id = id,
            amount_zats = req.amount_zats,
            "payment accepted; building"
        );

        let built = match self
            .build(address, req.amount_zats, req.max_fee_zats, &memo)
            .await
        {
            Ok(b) => {
                // Stored in the wallet DB, not yet recorded as built here.
                self.fault_point("crash_after_wallet_store");
                Some(b)
            }
            Err(BuildError::NoTx(r)) => {
                self.intents.mark_failed(id, &r)?;
                tracing::info!(payment_id = id, "payment failed before building: {r}");
                None
            }
            Err(BuildError::Unknown(e)) => match self.find_built(id)? {
                Some(b) => Some(b),
                None => {
                    self.intents.mark_failed(id, &e)?;
                    tracing::warn!(payment_id = id, "payment failed while building: {e}");
                    None
                }
            },
        };
        if let Some(b) = built {
            self.intents
                .mark_built(id, &b.txid, &b.raw, Some(b.fee), b.expiry)?;
            tracing::info!(payment_id = id, txid = %b.txid, fee_zats = b.fee, "transaction built and recorded");
            self.fault_point("crash_after_build");
            let i = self.intents.get(id)?.context("intent vanished")?;
            self.broadcast(&i).await;
        }
        Ok(self.intents.get(id)?.context("intent vanished")?)
    }

    #[allow(unused_variables)]
    fn fault_point(&self, name: &str) {
        #[cfg(feature = "fault-injection")]
        if self.cfg.fault.as_deref() == Some(name) {
            tracing::error!("FAULT INJECTION: exiting at '{name}'");
            std::process::exit(86);
        }
    }

    async fn build(
        &mut self,
        address: &str,
        amount: i64,
        max_fee: i64,
        memo: &str,
    ) -> Result<Built, BuildError> {
        use BuildError::*;
        // Current note and anchor state, as the Phase 0 `send` command does.
        self.sync()
            .await
            .map_err(|e| NoTx(format!("could not sync the wallet: {e:#}")))?;
        let to = payment::validate_recipient(&NETWORK, address).map_err(|e| NoTx(e.to_string()))?;
        let value = Zatoshis::from_u64(amount as u64).map_err(|_| NoTx("invalid amount".into()))?;
        let memo = payment::parse_memo(Some(memo)).map_err(|e| NoTx(e.to_string()))?;
        let proposal = payment::propose_shielded_payment(
            &mut self.db,
            &NETWORK,
            self.account,
            to,
            value,
            memo,
            ConfirmationsPolicy::default(),
        )
        .map_err(|e| NoTx(e.to_string()))?;
        policy::proposal_pool(&proposal).map_err(NoTx)?;
        let fee = payment::summarize(&proposal).fee_zats as i64;
        if fee > max_fee {
            return Err(NoTx(format!(
                "fee {fee} zats exceeds the allowed maximum of {max_fee} zats"
            )));
        }
        let paths = &self.cfg.paths;
        let mnemonic = secrets::load_mnemonic(&paths.encrypted_mnemonic, &paths.identity)
            .map_err(|_| NoTx("could not unlock the spending key".into()))?;
        let seed = secrets::seed_from_mnemonic(&mnemonic);
        drop(mnemonic);
        let usk =
            wallet::spending_key(&self.db, self.account, &seed).map_err(|e| NoTx(e.to_string()))?;
        drop(seed);
        // Proving is CPU-bound; this thread exists for it.
        let res = payment::build(&mut self.db, &NETWORK, &usk, &proposal);
        drop(usk);
        let (txid, raw) = res.map_err(|e| Unknown(e.to_string()))?;
        let expiry = self
            .db
            .get_transaction(txid)
            .ok()
            .flatten()
            .map(|t| i64::from(u32::from(t.expiry_height())));
        Ok(Built {
            txid: txid.to_string(),
            raw,
            fee,
            expiry,
            to_address: Some(address.to_string()),
            amount,
        })
    }
}

//! Whether a stored transaction must be sent (again) after a restart or a lost
//! broadcast response.
//!
//! The chain decides, not the node's refusal text (OBS-1, Vesting Phase 4:
//! a mined transaction was refused with "…transaction was committed to the
//! best chain" and re-sent six times). Before re-sending, the transaction is
//! looked up by txid:
//!
//! * mined or in the mempool: the network has it. A `built` intent is
//!   recorded as broadcast without sending ([`IntentStore::mark_observed_on_chain`]).
//! * missing: send the stored bytes again (same transaction, same txid).
//! * the lookup failed: send nothing and change nothing; the next background
//!   sync tries again.
//!
//! This only changes what the wallet records. The payroll side still settles
//! a payout only at its confirmation depth, with verification of the bytes
//! served by the chain.

use zcash_primitives::transaction::TxId;
use zcash_protocol::consensus::BlockHeight;

use hushush_wallet_proof::{remote, wallet};

use crate::chain::{self, Fetched};
use crate::intents::{Intent, IntentState, IntentStore};

/// Where the network has a transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    Mined(BlockHeight),
    Mempool,
    Missing,
}

/// The chain lookup a resend decision needs: the seam for tests.
pub(crate) trait ChainLookup {
    async fn seen(&mut self, tip: BlockHeight, txid: TxId) -> anyhow::Result<Seen>;
}

impl ChainLookup for remote::Client {
    async fn seen(&mut self, tip: BlockHeight, txid: TxId) -> anyhow::Result<Seen> {
        let (tx, seen) = match chain::fetch(self, tip, txid).await? {
            Fetched::Mined(tx, h, _) => (tx, Seen::Mined(h)),
            Fetched::Mempool(tx, _) => (tx, Seen::Mempool),
            Fetched::Missing => return Ok(Seen::Missing),
        };
        // A server answering with another transaction is not evidence.
        anyhow::ensure!(
            tx.txid() == txid,
            "lightwalletd returned transaction {} for {txid}",
            tx.txid()
        );
        Ok(seen)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Send the intent's stored bytes.
    Send,
    /// Nothing to send now.
    Skip,
}

/// Decides for one `built` or `broadcast` intent; may record a `built`
/// intent's transaction as observed on chain. Never builds anything.
pub(crate) async fn step<C: ChainLookup>(
    store: &IntentStore,
    chain: &mut C,
    tip: BlockHeight,
    i: &Intent,
) -> Step {
    if !matches!(i.state, IntentState::Built | IntentState::Broadcast) {
        return Step::Skip;
    }
    if i.expiry_height
        .is_some_and(|e| i64::from(u32::from(tip)) > e)
    {
        return Step::Skip; // can no longer be mined; the payroll side will fail it
    }
    let Some(txid) = i.txid.as_deref().and_then(|t| wallet::parse_txid(t).ok()) else {
        tracing::error!(payment_id = %i.payment_id, "stored transaction has no valid txid; not sending");
        return Step::Skip;
    };
    match chain.seen(tip, txid).await {
        Ok(Seen::Missing) => Step::Send,
        Ok(seen) if i.state == IntentState::Built => {
            match store.mark_observed_on_chain(&i.payment_id) {
                Ok(()) => {
                    tracing::info!(payment_id = %i.payment_id, txid = ?i.txid, ?seen, "transaction already on the network; recorded as broadcast without resending")
                }
                Err(e) => {
                    tracing::error!(payment_id = %i.payment_id, "could not record an observed transaction: {e:#}")
                }
            }
            Step::Skip
        }
        Ok(_) => Step::Skip, // broadcast, and the network has it
        Err(e) => {
            tracing::warn!(payment_id = %i.payment_id, txid = ?i.txid, "chain lookup failed; not resending until the next sync: {e:#}");
            Step::Skip
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intents::memo_for;

    const ID: &str = "33333333-3333-3333-3333-333333333333";
    // A well-formed txid (lowercase hex, 64 chars).
    fn txid_hex() -> String {
        "3fc96b8f496f948efaf3c7959ddeb64db8903e093e4120d42b7df3d3b962a02d".into()
    }
    const RAW: &[u8] = b"the signed transaction bytes";
    const TIP: u32 = 1_000;

    #[derive(Clone, Copy)]
    enum Answer {
        Mined,
        Mempool,
        Missing,
        Error,
    }

    struct FakeChain {
        answer: Answer,
        lookups: Vec<(u32, TxId)>,
    }

    impl FakeChain {
        fn new(answer: Answer) -> Self {
            Self {
                answer,
                lookups: vec![],
            }
        }
    }

    impl ChainLookup for FakeChain {
        async fn seen(&mut self, tip: BlockHeight, txid: TxId) -> anyhow::Result<Seen> {
            self.lookups.push((u32::from(tip), txid));
            match self.answer {
                Answer::Mined => Ok(Seen::Mined(BlockHeight::from_u32(TIP - 2))),
                Answer::Mempool => Ok(Seen::Mempool),
                Answer::Missing => Ok(Seen::Missing),
                Answer::Error => anyhow::bail!("status: Unavailable, message: \"transport error\""),
            }
        }
    }

    fn store() -> (tempfile::TempDir, IntentStore) {
        let d = tempfile::tempdir().unwrap();
        let s = IntentStore::open(&d.path().join("i.sqlite")).unwrap();
        (d, s)
    }

    /// A built intent, optionally after `refused` refused sends.
    fn built(s: &IntentStore, expiry: i64, refused: usize) -> Intent {
        s.insert_pending(ID, Some("org"), "utest1x", 20_000, 20_000, &memo_for(ID))
            .unwrap();
        s.mark_built(ID, &txid_hex(), RAW, Some(10_000), Some(expiry))
            .unwrap();
        for _ in 0..refused {
            s.record_broadcast(ID, Some("lightwalletd rejected transaction (code -1): …"))
                .unwrap();
        }
        s.get(ID).unwrap().unwrap()
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
    }

    fn run(s: &IntentStore, c: &mut FakeChain, i: &Intent) -> Step {
        rt().block_on(step(s, c, BlockHeight::from_u32(TIP), i))
    }

    /// The transaction itself is write-once: nothing here may replace it.
    fn same_tx(s: &IntentStore) {
        let i = s.get(ID).unwrap().unwrap();
        assert_eq!(i.txid.as_deref(), Some(txid_hex().as_str()));
        assert_eq!(i.raw_tx.as_deref(), Some(RAW));
        assert_eq!(i.fee_zats, Some(10_000));
    }

    #[test]
    fn built_and_mined_is_recorded_not_resent() {
        let (_d, s) = store();
        let i = built(&s, 2_000, 5);
        let mut c = FakeChain::new(Answer::Mined);
        assert_eq!(run(&s, &mut c, &i), Step::Skip);
        assert_eq!(
            c.lookups,
            vec![(TIP, wallet::parse_txid(&txid_hex()).unwrap())]
        );
        let after = s.get(ID).unwrap().unwrap();
        assert_eq!(after.state, IntentState::Broadcast);
        assert_eq!(
            after.broadcast_attempts, 5,
            "no attempt counted for an observation"
        );
        assert!(after.last_broadcast_error.is_some(), "last refusal kept");
        assert_eq!(s.count_unfinished().unwrap(), 0);
        same_tx(&s);
        // Later passes leave it alone: mined, so nothing to send.
        assert_eq!(run(&s, &mut c, &after), Step::Skip);
        assert_eq!(s.get(ID).unwrap().unwrap().broadcast_attempts, 5);
    }

    #[test]
    fn built_and_in_mempool_is_recorded_not_resent() {
        let (_d, s) = store();
        let i = built(&s, 2_000, 0);
        let mut c = FakeChain::new(Answer::Mempool);
        assert_eq!(run(&s, &mut c, &i), Step::Skip);
        let after = s.get(ID).unwrap().unwrap();
        assert_eq!(
            (after.state, after.broadcast_attempts),
            (IntentState::Broadcast, 0)
        );
        same_tx(&s);
    }

    #[test]
    fn built_and_missing_resends_the_stored_transaction() {
        let (_d, s) = store();
        let i = built(&s, 2_000, 1);
        let mut c = FakeChain::new(Answer::Missing);
        assert_eq!(run(&s, &mut c, &i), Step::Send);
        // The caller sends `i.raw_tx`: exactly the stored bytes, same txid.
        assert_eq!(i.raw_tx.as_deref(), Some(RAW));
        let after = s.get(ID).unwrap().unwrap();
        assert_eq!(
            (after.state, after.broadcast_attempts),
            (IntentState::Built, 1)
        );
        same_tx(&s);
    }

    #[test]
    fn lookup_error_sends_nothing_and_changes_nothing() {
        let (_d, s) = store();
        let i = built(&s, 2_000, 2);
        let mut c = FakeChain::new(Answer::Error);
        assert_eq!(run(&s, &mut c, &i), Step::Skip);
        let after = s.get(ID).unwrap().unwrap();
        assert_eq!(after.state, IntentState::Built);
        assert_eq!(after.broadcast_attempts, 2);
        assert_eq!(after.updated_at, i.updated_at);
        assert_eq!(after.last_broadcast_error, i.last_broadcast_error);
        same_tx(&s);
        // Once the lookup works again, the next pass decides normally.
        let mut c = FakeChain::new(Answer::Missing);
        assert_eq!(run(&s, &mut c, &after), Step::Send);
    }

    #[test]
    fn broadcast_intents_keep_their_behaviour() {
        let (_d, s) = store();
        built(&s, 2_000, 0);
        s.record_broadcast(ID, None).unwrap();
        let i = s.get(ID).unwrap().unwrap();
        // Forgotten by the node: resend; known or unknown: leave it.
        assert_eq!(
            run(&s, &mut FakeChain::new(Answer::Missing), &i),
            Step::Send
        );
        for a in [Answer::Mined, Answer::Mempool, Answer::Error] {
            assert_eq!(run(&s, &mut FakeChain::new(a), &i), Step::Skip);
        }
        let after = s.get(ID).unwrap().unwrap();
        assert_eq!(
            (after.state, after.broadcast_attempts),
            (IntentState::Broadcast, 1)
        );
        same_tx(&s);
    }

    #[test]
    fn expired_or_unsendable_intents_are_not_looked_up() {
        let (_d, s) = store();
        let i = built(&s, i64::from(TIP) - 1, 0);
        let mut c = FakeChain::new(Answer::Missing);
        assert_eq!(run(&s, &mut c, &i), Step::Skip);
        assert!(c.lookups.is_empty());
        assert_eq!(s.get(ID).unwrap().unwrap().state, IntentState::Built);
        // Not built/broadcast: nothing to send.
        let mut pending = i.clone();
        pending.state = IntentState::Pending;
        assert_eq!(run(&s, &mut c, &pending), Step::Skip);
        // A malformed txid is never sent on a guess.
        let mut bad = i.clone();
        bad.expiry_height = Some(2_000);
        bad.txid = Some("not a txid".into());
        assert_eq!(run(&s, &mut c, &bad), Step::Skip);
        assert!(c.lookups.is_empty());
    }
}

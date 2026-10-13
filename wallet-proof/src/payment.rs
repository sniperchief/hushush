//! Shielded payment construction. Policy enforced here, on top of upstream:
//!
//! * the recipient must be a shielded (Sapling or Unified-with-shielded-receiver)
//!   testnet address; transparent and TEX addresses are rejected;
//! * the proposal must have no transparent inputs, outputs or change in any step;
//! * fees follow ZIP 317, inputs are chosen by the upstream greedy selector, and
//!   proofs are produced by the upstream `LocalTxProver` with bundled Sapling
//!   parameters (Orchard/Ironwood use Halo 2, which needs no parameters).
//!
//! Everything is generic over the `WalletDb` clock/RNG/network so the same code
//! runs against the real testnet wallet and the librustzcash mock-chain harness.

use anyhow::{anyhow, bail};
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use serde::Serialize;
use zcash_address::ZcashAddress;
use zcash_client_backend::{
    data_api::{
        WalletRead,
        wallet::{
            ConfirmationsPolicy, SpendingKeys, create_proposed_transactions,
            input_selection::{GreedyInputSelector, SpendPolicy},
            propose_transfer,
        },
    },
    fees::{DustOutputPolicy, StandardFeeRule, standard::SingleOutputChangeStrategy},
    proposal::Proposal,
    proto::service,
    wallet::OvkPolicy,
};
use zcash_client_sqlite::{AccountUuid, ReceivedNoteId, WalletDb, util::{Clock, SystemClock}};
use zcash_keys::{address::Address, keys::UnifiedSpendingKey};
use zcash_primitives::transaction::TxId;
use zcash_proofs::prover::LocalTxProver;
use zcash_protocol::{
    PoolType, ShieldedPool,
    consensus::Parameters,
    memo::{Memo, MemoBytes},
    value::Zatoshis,
};
use zip321::{Payment, TransactionRequest};

use crate::remote::Client;

pub type ShieldedProposal = Proposal<StandardFeeRule, ReceivedNoteId>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecipientError {
    #[error("not a valid Zcash address for this network")]
    Unparseable,
    #[error("transparent addresses are not allowed: payouts must be shielded")]
    Transparent,
    #[error("TEX (transparent-source-only) addresses are not allowed: payouts must be shielded")]
    Tex,
    #[error("unified address has no shielded receiver")]
    NoShieldedReceiver,
}

/// Accepts only shielded recipients on `params`' network.
pub fn validate_recipient<P: Parameters>(
    params: &P,
    encoded: &str,
) -> Result<ZcashAddress, RecipientError> {
    let encoded = encoded.trim();
    let addr = Address::decode(params, encoded).ok_or(RecipientError::Unparseable)?;
    match &addr {
        Address::Sapling(_) => {}
        Address::Unified(ua) => {
            if !(ua.has_orchard() || ua.has_sapling()) {
                return Err(RecipientError::NoShieldedReceiver);
            }
        }
        Address::Transparent(_) => return Err(RecipientError::Transparent),
        Address::Tex(_) => return Err(RecipientError::Tex),
    }
    ZcashAddress::try_from_encoded(encoded).map_err(|_| RecipientError::Unparseable)
}

pub fn parse_memo(text: Option<&str>) -> anyhow::Result<Option<MemoBytes>> {
    text.map(|t| {
        t.parse::<Memo>()
            .map(MemoBytes::from)
            .map_err(|e| anyhow!("invalid memo: {e:?}"))
    })
    .transpose()
}

/// Proposes a single payment of `amount` to `to` from `account`.
pub fn propose_shielded_payment<P, CL, R>(
    db: &mut WalletDb<rusqlite::Connection, P, CL, R>,
    params: &P,
    account: AccountUuid,
    to: ZcashAddress,
    amount: Zatoshis,
    memo: Option<MemoBytes>,
    confirmations: ConfirmationsPolicy,
) -> anyhow::Result<ShieldedProposal>
where
    P: Parameters + Clone + 'static,
    CL: Clock + Clone + 'static,
    R: rand::Rng + Clone + 'static,
{
    if amount == Zatoshis::ZERO {
        bail!("amount must be greater than zero");
    }
    let payment = Payment::new(to, Some(amount), memo, None, None, vec![])
        .map_err(|e| anyhow!("invalid payment: {e:?}"))?;
    let request = TransactionRequest::new(vec![payment]).map_err(|e| anyhow!("{e}"))?;

    let change = SingleOutputChangeStrategy::new(
        StandardFeeRule::Zip317,
        None,
        // Fallback only; upstream redirects change to Ironwood per the NU6.3
        // Orchard turnstile rules when Ironwood is active.
        ShieldedPool::Orchard,
        DustOutputPolicy::default(),
    );
    let proposal = propose_transfer::<_, _, _, _, std::convert::Infallible>(
        db,
        params,
        account,
        &GreedyInputSelector::new(),
        &change,
        request,
        confirmations,
        &SpendPolicy::default(),
        None,
        None,
    )
    .map_err(|e| anyhow!("could not propose payment: {e}"))?;

    assert_fully_shielded(&proposal)?;
    Ok(proposal)
}

/// Rejects any proposal that would touch the transparent pool.
pub fn assert_fully_shielded(proposal: &ShieldedProposal) -> anyhow::Result<()> {
    for (i, step) in proposal.steps().iter().enumerate() {
        if !step.transparent_inputs().is_empty() {
            bail!("step {i} spends transparent inputs");
        }
        if step.payment_pools().values().any(|p| *p == PoolType::Transparent) {
            bail!("step {i} pays to the transparent pool");
        }
        if step.change_in_pool(PoolType::Transparent) {
            bail!("step {i} sends change to the transparent pool");
        }
        if step.involves(PoolType::Transparent) {
            bail!("step {i} involves the transparent pool");
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ProposalSummary {
    pub steps: usize,
    pub fee_zats: u64,
    pub payment_pools: Vec<String>,
    pub input_pools: Vec<String>,
    pub change_pools: Vec<String>,
}

pub fn summarize(proposal: &ShieldedProposal) -> ProposalSummary {
    let pools = [
        PoolType::SAPLING,
        PoolType::ORCHARD,
        PoolType::Shielded(ShieldedPool::Ironwood),
        PoolType::Transparent,
    ];
    let name = |p: &PoolType| format!("{p:?}");
    let steps = proposal.steps();
    ProposalSummary {
        steps: steps.len(),
        fee_zats: steps.iter().map(|s| s.balance().fee_required().into_u64()).sum(),
        payment_pools: steps
            .iter()
            .flat_map(|s| s.payment_pools().values().map(name).collect::<Vec<_>>())
            .collect(),
        input_pools: pools
            .iter()
            .filter(|p| steps.iter().any(|s| s.input_in_pool(**p)))
            .map(name)
            .collect(),
        change_pools: pools
            .iter()
            .filter(|p| steps.iter().any(|s| s.change_in_pool(**p)))
            .map(name)
            .collect(),
    }
}

/// Builds, proves and signs the proposed transaction, storing it in the wallet.
/// Returns the txid and the raw transaction bytes ready for broadcast.
pub fn build<P, CL, R>(
    db: &mut WalletDb<rusqlite::Connection, P, CL, R>,
    params: &P,
    usk: &UnifiedSpendingKey,
    proposal: &ShieldedProposal,
) -> anyhow::Result<(TxId, Vec<u8>)>
where
    P: Parameters + Clone + 'static,
    CL: Clock + Clone + 'static,
    R: rand::Rng + Clone + 'static,
{
    let prover = LocalTxProver::bundled();
    let txids = create_proposed_transactions::<_, _, std::convert::Infallible, _, std::convert::Infallible, _>(
        db,
        params,
        &SystemClock,
        &mut UnwrapErr(SysRng),
        &prover,
        &prover,
        &SpendingKeys::from_unified_spending_key(usk.clone()),
        OvkPolicy::Sender,
        proposal,
        None,
    )
    .map_err(|e| anyhow!("could not build transaction: {e}"))?;
    if txids.len() != 1 {
        bail!("expected a single transaction, got {}", txids.len());
    }
    let txid = *txids.first();
    let tx = db
        .get_transaction(txid)?
        .ok_or_else(|| anyhow!("built transaction {txid} not found in wallet"))?;
    let mut raw = vec![];
    tx.write(&mut raw)?;
    Ok((txid, raw))
}

pub async fn broadcast(client: &mut Client, raw: Vec<u8>) -> anyhow::Result<()> {
    let resp = client
        .send_transaction(service::RawTransaction {
            data: raw,
            height: 0,
        })
        .await?
        .into_inner();
    if resp.error_code != 0 {
        bail!(
            "lightwalletd rejected transaction (code {}): {}",
            resp.error_code,
            resp.error_message
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zcash_address::ToAddress as _;
    use zcash_keys::keys::{UnifiedAddressRequest, UnifiedSpendingKey};
    use zcash_protocol::consensus::{MainNetwork, NetworkType, TestNetwork};

    fn usk<P: Parameters>(p: &P) -> UnifiedSpendingKey {
        UnifiedSpendingKey::from_seed(p, &[7; 32], zip32::AccountId::ZERO).unwrap()
    }

    #[test]
    fn accepts_shielded_unified_and_sapling() {
        let k = usk(&TestNetwork).to_unified_full_viewing_key();
        let ua = k.default_address(crate::wallet::SHIELDED_ONLY_UA).unwrap().0;
        assert!(ua.transparent().is_none());
        assert!(validate_recipient(&TestNetwork, &ua.encode(&TestNetwork)).is_ok());
        let sapling = Address::Sapling(Box::new(*ua.sapling().unwrap())).encode(&TestNetwork);
        assert!(validate_recipient(&TestNetwork, &sapling).is_ok());
    }

    #[test]
    fn rejects_transparent_and_tex() {
        let t = ZcashAddress::from_transparent_p2pkh(NetworkType::Test, [1; 20]).encode();
        assert_eq!(validate_recipient(&TestNetwork, &t), Err(RecipientError::Transparent));
        let tex = ZcashAddress::from_tex(NetworkType::Test, [1; 20]).encode();
        assert_eq!(validate_recipient(&TestNetwork, &tex), Err(RecipientError::Tex));
    }

    #[test]
    fn rejects_mainnet_addresses_and_garbage() {
        let k = usk(&MainNetwork).to_unified_full_viewing_key();
        let ua = k.default_address(UnifiedAddressRequest::SHIELDED).unwrap().0;
        assert_eq!(
            validate_recipient(&TestNetwork, &ua.encode(&MainNetwork)),
            Err(RecipientError::Unparseable)
        );
        assert_eq!(validate_recipient(&TestNetwork, "utest1nope"), Err(RecipientError::Unparseable));
    }

    #[test]
    fn memo_parsing() {
        assert!(parse_memo(None).unwrap().is_none());
        assert!(parse_memo(Some("hello")).unwrap().is_some());
        assert!(parse_memo(Some(&"x".repeat(513))).is_err());
    }
}

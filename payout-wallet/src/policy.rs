//! Payout policy on top of the Phase 0 wallet's own checks
//! (`payment::validate_recipient`, `payment::assert_fully_shielded`):
//!
//! * **Recipients** must be testnet Unified Addresses with an Orchard
//!   receiver. Sapling-only addresses are refused: paying one from the payout
//!   wallet (whose funds live in Ironwood) would move value between pools,
//!   which publishes the amount (Phase 0 report §5).
//! * **Proposals** must touch exactly one shielded pool, in one step.
//! * **Verification** decrypts the transaction's outgoing outputs with the
//!   wallet's viewing key and checks recipient, amount and memo; the keyless
//!   observer view checks it is fully shielded and single-pool.

use std::collections::HashMap;

use serde::Serialize;
use zcash_client_backend::{TransferType, decrypt_transaction};
use zcash_client_sqlite::AccountUuid;
use zcash_keys::address::Address;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::{
    PoolType,
    consensus::BlockHeight,
    memo::{Memo, MemoBytes},
};

use hushush_wallet_proof::{NETWORK, observer, payment};

use crate::intents::{memo_for, recipient_fingerprint};

/// Raw receivers of an accepted payout address.
#[derive(Clone, Debug)]
pub struct Recipient {
    pub orchard: [u8; 43],
    pub sapling: Option<[u8; 43]>,
}

pub const SAPLING_ONLY: &str = "Sapling-only addresses are not accepted: paying one from the payout \
     wallet moves value between shielded pools, which publicly reveals the amount. Use a Unified \
     Address with an Orchard receiver (e.g. from Zashi).";

pub fn payout_recipient(address: &str) -> Result<Recipient, String> {
    let address = address.trim();
    if address.len() > 1_000 {
        return Err("address is too long".into());
    }
    payment::validate_recipient(&NETWORK, address).map_err(|e| e.to_string())?;
    match Address::decode(&NETWORK, address) {
        Some(Address::Unified(ua)) => {
            let orchard = ua.orchard().ok_or_else(|| {
                "this Unified Address has no Orchard receiver; payouts need one".to_string()
            })?;
            Ok(Recipient {
                orchard: orchard.to_raw_address_bytes(),
                sapling: ua.sapling().map(|s| s.to_bytes()),
            })
        }
        Some(Address::Sapling(_)) => Err(SAPLING_ONLY.into()),
        _ => Err("not a shielded testnet address".into()),
    }
}

pub fn pool_name(p: PoolType) -> &'static str {
    match p {
        PoolType::Transparent => "transparent",
        PoolType::SAPLING => "sapling",
        PoolType::ORCHARD => "orchard",
        _ => "ironwood",
    }
}

/// The single shielded pool a proposal moves value within, or why it is refused.
pub fn proposal_pool(p: &payment::ShieldedProposal) -> Result<PoolType, String> {
    let steps = p.steps();
    if steps.len() != 1 {
        return Err(format!(
            "payment would need {} transactions; only single-transaction payouts are allowed",
            steps.len()
        ));
    }
    let step = steps.first();
    let touched: Vec<PoolType> = [PoolType::SAPLING, PoolType::ORCHARD, PoolType::IRONWOOD]
        .into_iter()
        .filter(|pool| step.involves(*pool))
        .collect();
    match touched.as_slice() {
        [one] => Ok(*one),
        [] => Err("payment touches no shielded pool".into()),
        many => Err(format!(
            "payment would move value between shielded pools ({}), which publicly reveals the \
             amount; consolidate the payout wallet's funds into one pool first",
            many.iter()
                .map(|p| pool_name(*p))
                .collect::<Vec<_>>()
                .join(" → ")
        )),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VerifiedOutput {
    pub recipient_fingerprint: String,
    pub amount_zats: i64,
    pub memo_matches: bool,
    pub pool: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TxVerification {
    pub source: String,
    pub fully_shielded: bool,
    pub single_pool: bool,
    pub fee_zats: Option<i64>,
    pub outputs: Vec<VerifiedOutput>,
}

fn memo_text(m: &MemoBytes) -> Option<String> {
    match Memo::try_from(m.clone()).ok()? {
        Memo::Text(t) => Some(t.to_string()),
        _ => None,
    }
}

/// Verifies `tx` as a payout of `payment_id` to `address`. `source` says
/// where the bytes came from ("chain" = served by the node).
#[allow(clippy::too_many_arguments)]
pub fn verify_tx(
    tx: &Transaction,
    raw_len: usize,
    mined: Option<BlockHeight>,
    tip: BlockHeight,
    ufvks: &HashMap<AccountUuid, UnifiedFullViewingKey>,
    payment_id: &str,
    address: &str,
    source: &str,
) -> TxVerification {
    let view = observer::observe(tx, raw_len, mined.map(u32::from));
    let bundles = [
        view.sapling_spends + view.sapling_outputs > 0,
        view.orchard_actions > 0,
        view.ironwood_actions > 0,
    ];
    let single_pool = bundles.iter().filter(|b| **b).count() == 1;

    let recipient = payout_recipient(address).ok();
    let expected_memo = memo_for(payment_id);
    let fingerprint_of = |raw: &[u8]| -> String {
        let ours = recipient.as_ref().is_some_and(|r| {
            raw == r.orchard.as_slice() || r.sapling.is_some_and(|s| raw == s.as_slice())
        });
        if ours {
            recipient_fingerprint(payment_id, address)
        } else {
            // Someone else: a fingerprint that cannot match the expected one.
            recipient_fingerprint(payment_id, &format!("raw:{}", hex::encode(raw)))
        }
    };

    let d = decrypt_transaction(&NETWORK, mined, Some(tip), tx, ufvks);
    let mut outputs = vec![];
    for o in d.sapling_outputs() {
        if o.transfer_type() == TransferType::Outgoing {
            outputs.push(VerifiedOutput {
                recipient_fingerprint: fingerprint_of(&o.note().recipient().to_bytes()),
                amount_zats: o.note().value().inner() as i64,
                memo_matches: memo_text(o.memo()).as_deref() == Some(expected_memo.as_str()),
                pool: "sapling".into(),
            });
        }
    }
    for (pool, outs) in [
        ("orchard", d.orchard_outputs()),
        ("ironwood", d.ironwood_outputs()),
    ] {
        for o in outs {
            if o.transfer_type() == TransferType::Outgoing {
                let note = &o.note().0;
                outputs.push(VerifiedOutput {
                    recipient_fingerprint: fingerprint_of(&note.recipient().to_raw_address_bytes()),
                    amount_zats: note.value().inner() as i64,
                    memo_matches: memo_text(o.memo()).as_deref() == Some(expected_memo.as_str()),
                    pool: pool.into(),
                });
            }
        }
    }
    TxVerification {
        source: source.into(),
        fully_shielded: view.fully_shielded,
        single_pool,
        fee_zats: view.fee_zats,
        outputs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zcash_address::{ToAddress as _, ZcashAddress};
    use zcash_keys::keys::{ReceiverRequirement, UnifiedAddressRequest, UnifiedSpendingKey};
    use zcash_protocol::consensus::NetworkType;

    fn ufvk(seed: u8) -> UnifiedFullViewingKey {
        UnifiedSpendingKey::from_seed(&NETWORK, &[seed; 32], zip32::AccountId::ZERO)
            .unwrap()
            .to_unified_full_viewing_key()
    }

    fn ua(req: UnifiedAddressRequest) -> String {
        ufvk(3).default_address(req).unwrap().0.encode(&NETWORK)
    }

    #[test]
    fn accepts_unified_with_orchard() {
        let shielded_only = UnifiedAddressRequest::unsafe_custom(
            ReceiverRequirement::Require,
            ReceiverRequirement::Require,
            ReceiverRequirement::Omit,
        );
        let r = payout_recipient(&ua(shielded_only)).unwrap();
        assert!(r.sapling.is_some());
        let orchard_only = UnifiedAddressRequest::unsafe_custom(
            ReceiverRequirement::Require,
            ReceiverRequirement::Omit,
            ReceiverRequirement::Omit,
        );
        let r = payout_recipient(&format!("  {}  ", ua(orchard_only))).unwrap();
        assert!(r.sapling.is_none());
    }

    #[test]
    fn refuses_sapling_only_and_unified_without_orchard() {
        let k = ufvk(3);
        let full = k
            .default_address(hushush_wallet_proof::wallet::SHIELDED_ONLY_UA)
            .unwrap()
            .0;
        let sapling = Address::Sapling(Box::new(*full.sapling().unwrap())).encode(&NETWORK);
        assert_eq!(payout_recipient(&sapling).unwrap_err(), SAPLING_ONLY);
        let sapling_ua = UnifiedAddressRequest::unsafe_custom(
            ReceiverRequirement::Omit,
            ReceiverRequirement::Require,
            ReceiverRequirement::Omit,
        );
        assert!(
            payout_recipient(&ua(sapling_ua))
                .unwrap_err()
                .contains("no Orchard receiver")
        );
    }

    #[test]
    fn refuses_transparent_tex_mainnet_and_garbage() {
        let t = ZcashAddress::from_transparent_p2pkh(NetworkType::Test, [1; 20]).encode();
        assert!(payout_recipient(&t).unwrap_err().contains("transparent"));
        let tex = ZcashAddress::from_tex(NetworkType::Test, [1; 20]).encode();
        assert!(payout_recipient(&tex).unwrap_err().contains("TEX"));
        let main = UnifiedSpendingKey::from_seed(
            &zcash_protocol::consensus::MainNetwork,
            &[3; 32],
            zip32::AccountId::ZERO,
        )
        .unwrap()
        .to_unified_full_viewing_key()
        .default_address(UnifiedAddressRequest::SHIELDED)
        .unwrap()
        .0
        .encode(&zcash_protocol::consensus::MainNetwork);
        assert!(payout_recipient(&main).is_err());
        assert!(payout_recipient("utest1nope").is_err());
        assert!(payout_recipient(&"u".repeat(2_000)).is_err());
    }
}

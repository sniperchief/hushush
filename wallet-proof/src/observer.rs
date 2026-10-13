//! What an independent chain observer — someone holding NO keys — can learn
//! from a transaction. Works purely from the serialized transaction bytes.

use serde::Serialize;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::value::ZatBalance;

#[derive(Debug, Serialize)]
pub struct ObserverView {
    pub txid: String,
    pub mined_height: Option<u32>,
    pub size_bytes: usize,
    pub tx_version: String,
    pub consensus_branch_id: String,
    pub lock_time: u32,
    pub expiry_height: u32,

    pub transparent_inputs: usize,
    pub transparent_outputs: Vec<TransparentOutput>,
    pub sprout_joinsplits: usize,

    pub sapling_spends: usize,
    pub sapling_outputs: usize,
    pub sapling_value_balance_zats: i64,

    pub orchard_actions: usize,
    pub orchard_value_balance_zats: i64,
    pub orchard_anchor_hex: Option<String>,

    pub ironwood_actions: usize,
    pub ironwood_value_balance_zats: i64,
    pub ironwood_anchor_hex: Option<String>,

    /// Public fee, derivable whenever there are no transparent inputs.
    pub fee_zats: Option<i64>,
    pub fully_shielded: bool,
    /// Human-readable list of everything this transaction reveals.
    pub disclosures: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TransparentOutput {
    pub value_zats: u64,
    pub script_pubkey_hex: String,
}

fn zb(v: ZatBalance) -> i64 {
    i64::from(v)
}

pub fn observe(tx: &Transaction, raw_len: usize, mined_height: Option<u32>) -> ObserverView {
    let t = tx.transparent_bundle();
    let t_in = t.map_or(0, |b| b.vin.len());
    let t_out: Vec<TransparentOutput> = t.map_or(vec![], |b| {
        b.vout
            .iter()
            .map(|o| TransparentOutput {
                value_zats: o.value().into_u64(),
                script_pubkey_hex: hex::encode(&o.script_pubkey().0.0),
            })
            .collect()
    });
    let sprout = tx.sprout_bundle().map_or(0, |b| b.joinsplits.len());

    let (s_sp, s_out, s_vb) = tx.sapling_bundle().map_or((0, 0, 0), |b| {
        (b.shielded_spends().len(), b.shielded_outputs().len(), zb(*b.value_balance()))
    });
    let (o_act, o_vb, o_anchor) = tx.orchard_bundle().map_or((0, 0, None), |b| {
        (b.actions().len(), zb(*b.value_balance()), Some(hex::encode(b.anchor().to_bytes())))
    });
    let (i_act, i_vb, i_anchor) = tx.ironwood_bundle().map_or((0, 0, None), |b| {
        (b.actions().len(), zb(*b.value_balance()), Some(hex::encode(b.anchor().to_bytes())))
    });

    let t_out_sum: i64 = t_out.iter().map(|o| o.value_zats as i64).sum();
    let fee = (t_in == 0).then(|| s_vb + o_vb + i_vb - t_out_sum);
    let fully_shielded = t_in == 0 && t_out.is_empty() && sprout == 0;

    let mut d = vec![];
    if fully_shielded {
        d.push("No transparent inputs or outputs: no addresses or payment amounts are visible.".into());
    } else {
        if t_in > 0 {
            d.push(format!("LEAK: {t_in} transparent input(s) reveal the funding addresses and amounts."));
        }
        if !t_out.is_empty() {
            d.push(format!(
                "LEAK: {} transparent output(s) reveal recipient address(es) and amount(s).",
                t_out.len()
            ));
        }
    }
    if let Some(f) = fee {
        d.push(format!("Fee is public: {f} zatoshis (implies the ZIP 317 logical-action count)."));
    }
    d.push(format!(
        "Transaction shape is public: {s_sp} Sapling spend(s), {s_out} Sapling output(s), \
         {o_act} Orchard action(s), {i_act} Ironwood action(s). Orchard/Ironwood actions are \
         padded (min 2), so counts bound but do not reveal the number of recipients."
    ));
    let nonzero = [s_vb, o_vb, i_vb].iter().filter(|v| **v != 0).count();
    if nonzero > 1 {
        d.push(format!(
            "Pool crossing: value moves between shielded pools (Sapling {s_vb}, Orchard {o_vb}, \
             Ironwood {i_vb}). The amount crossing a pool boundary is public."
        ));
    }
    d.push(format!(
        "Expiry height {} and lock time {} are public; mined height/time are public once mined.",
        u32::from(tx.expiry_height()),
        tx.lock_time()
    ));
    d.push(
        "Anchors (commitment-tree roots) are public and indicate roughly which chain height the \
         spent notes were proven against."
            .into(),
    );
    d.push(
        "Nullifiers and note commitments are public but are not linkable to addresses or values \
         without a viewing key."
            .into(),
    );

    ObserverView {
        txid: tx.txid().to_string(),
        mined_height,
        size_bytes: raw_len,
        tx_version: format!("{:?}", tx.version()),
        consensus_branch_id: format!("{:?}", tx.consensus_branch_id()),
        lock_time: tx.lock_time(),
        expiry_height: tx.expiry_height().into(),
        transparent_inputs: t_in,
        transparent_outputs: t_out,
        sprout_joinsplits: sprout,
        sapling_spends: s_sp,
        sapling_outputs: s_out,
        sapling_value_balance_zats: s_vb,
        orchard_actions: o_act,
        orchard_value_balance_zats: o_vb,
        orchard_anchor_hex: o_anchor,
        ironwood_actions: i_act,
        ironwood_value_balance_zats: i_vb,
        ironwood_anchor_hex: i_anchor,
        fee_zats: fee,
        fully_shielded,
        disclosures: d,
    }
}

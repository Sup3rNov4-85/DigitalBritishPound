//! Balance and history queries shared by CLI, API, and UI helpers.

use serde::Serialize;

use crate::{
    crypto::wallet::{format_units_as_dbc, Address},
    node::chain::Chain,
};

#[derive(Debug, Clone, Serialize)]
pub struct BalanceSummary {
    pub address: String,
    pub confirmed_pence: u64,
    pub spendable_pence: u64,
    pub confirmed_dbc: String,
    pub spendable_dbc: String,
}

pub fn balance_for_address(
    chain: &Chain,
    addr: &Address,
    _include_immature_in_shown: bool,
) -> Result<BalanceSummary, crate::node::chain::ChainError> {
    let current_height = chain.tip()?.map(|t| t.height).unwrap_or(0);
    let mut total = 0u64;
    let mut spendable = 0u64;
    chain.utxos().for_each(|_op, utxo| {
        if utxo.output.script_pubkey.as_bytes().len() == 21
            && utxo.output.script_pubkey.as_bytes()[0] == 0x14
            && &utxo.output.script_pubkey.as_bytes()[1..21] == addr.as_bytes()
        {
            total = total.saturating_add(utxo.value());
            if utxo.is_mature(current_height) {
                spendable = spendable.saturating_add(utxo.value());
            }
        }
        Ok(())
    })?;
    Ok(BalanceSummary {
        address: addr.to_bech32m().map_err(|e| {
            crate::node::chain::ChainError::Ser(e.to_string())
        })?,
        confirmed_pence: total,
        spendable_pence: spendable,
        confirmed_dbc: format_units_as_dbc(total),
        spendable_dbc: format_units_as_dbc(spendable),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub height: u64,
    pub kind: String,
    pub amount_pence: u64,
    pub amount_dbc: String,
}

/// Scan the local chain for coinbase and outputs paying `addr` (newest first).
pub fn history_for_address(
    chain: &Chain,
    addr: &Address,
    limit: usize,
) -> Result<Vec<HistoryEntry>, crate::node::chain::ChainError> {
    let tip = chain.tip()?.map(|t| t.height).unwrap_or(0);
    let mut out = Vec::new();
    for h in (0..=tip).rev() {
        if out.len() >= limit {
            break;
        }
        let Some(block) = chain.db().get_block_at_height(h)? else {
            continue;
        };
        if block.transactions.is_empty() {
            continue;
        }
        let coinbase = &block.transactions[0];
        for (idx, output) in coinbase.outputs.iter().enumerate() {
            if script_pays_address(&output.script_pubkey, addr) {
                out.push(HistoryEntry {
                    height: h,
                    kind: if idx == 0 {
                        "coinbase".to_string()
                    } else {
                        "output".to_string()
                    },
                    amount_pence: output.value,
                    amount_dbc: format_units_as_dbc(output.value),
                });
            }
        }
        for tx in block.transactions.iter().skip(1) {
            for output in &tx.outputs {
                if script_pays_address(&output.script_pubkey, addr) {
                    out.push(HistoryEntry {
                        height: h,
                        kind: "received".to_string(),
                        amount_pence: output.value,
                        amount_dbc: format_units_as_dbc(output.value),
                    });
                }
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

fn script_pays_address(script: &crate::Script, addr: &Address) -> bool {
    script.as_bytes().len() == 21
        && script.as_bytes()[0] == 0x14
        && &script.as_bytes()[1..21] == addr.as_bytes()
}

pub fn format_balance_display(summary: &BalanceSummary, include_immature: bool) -> String {
    let shown_dbc = if include_immature {
        &summary.confirmed_dbc
    } else {
        &summary.spendable_dbc
    };
    format!(
        "Address: {}\nConfirmed: {} DBC\nSpendable: {} DBC\nShown: {} DBC\n\nCoinbase rewards need 100 blocks (~25 h) to mature.",
        summary.address, summary.confirmed_dbc, summary.spendable_dbc, shown_dbc
    )
}

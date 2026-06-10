use std::collections::HashMap;

use crate::{OutPoint, Transaction};

/// In-memory mempool with replace-by-fee (BIP125-style `sequence < 0xfffffffe`).
#[derive(Debug, Default)]
pub struct Mempool {
    entries: HashMap<[u8; 32], (Transaction, u64)>,
}

impl Mempool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn snapshot(&self) -> Vec<Transaction> {
        self.entries.values().map(|(tx, _)| tx.clone()).collect()
    }

    /// Transactions with their fees, for fee-aware block assembly.
    pub fn snapshot_with_fees(&self) -> Vec<(Transaction, u64)> {
        self.entries.values().cloned().collect()
    }

    pub fn insert(&mut self, tx: Transaction, fee: u64) -> bool {
        let txid = match tx.compute_hash() {
            Ok(h) => *h.as_bytes(),
            Err(_) => return false,
        };

        let conflicts = self.find_conflicts(&tx);
        if !conflicts.is_empty() {
            let opts_in = tx.inputs.iter().any(|i| i.sequence < 0xffff_fffe);
            if !opts_in {
                return false;
            }
            let old_fee: u64 = conflicts
                .iter()
                .filter_map(|id| self.entries.get(id).map(|(_, f)| *f))
                .sum();
            if fee <= old_fee {
                return false;
            }
            for id in conflicts {
                self.entries.remove(&id);
            }
        }

        self.entries.insert(txid, (tx, fee));
        true
    }

    pub fn remove_confirmed(&mut self, confirmed: &[[u8; 32]]) {
        for id in confirmed {
            self.entries.remove(id);
        }
    }

    fn find_conflicts(&self, tx: &Transaction) -> Vec<[u8; 32]> {
        let spent: Vec<&OutPoint> = tx.inputs.iter().map(|i| &i.previous_output).collect();
        let mut conflicts = Vec::new();
        for (id, (other, _)) in &self.entries {
            for inp in &other.inputs {
                if spent.iter().any(|op| *op == &inp.previous_output) {
                    conflicts.push(*id);
                    break;
                }
            }
        }
        conflicts
    }
}

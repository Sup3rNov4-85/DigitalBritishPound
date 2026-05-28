#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::{
        consensus::UNITS_PER_DBC,
        crypto::wallet::Wallet,
        node::{
            miner::Miner,
            validation::{sighash, validate_block, validate_transaction},
        },
        storage::chaindb::ChainDb,
        types::utxo::UTXOSet,
        Hash,
    };

    static N: AtomicU64 = AtomicU64::new(0);
    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("dbc_{tag}_{n}_{}", std::process::id()))
    }

    #[test]
    fn mine_then_send_spends_utxo() {
        let root = tmpdir("mine_send");
        let chain = ChainDb::open(root.join("chain")).unwrap();
        let utxos = UTXOSet::new(root.join("utxo")).unwrap();

        let sender_m = Wallet::generate_mnemonic();
        let sender = Wallet::from_mnemonic(&sender_m).unwrap();
        let receiver_m = Wallet::generate_mnemonic();
        let receiver = Wallet::from_mnemonic(&receiver_m).unwrap();

        let difficulty_target: u32 = 0x1f00_ffff;
        let msg = b"genesis";

        // Mine a few blocks (coinbase maturity = 2 blocks in test builds).
        let mut prev = Hash::ZERO;
        for h in 0..5u64 {
            let block = Miner::mine_next_block(
                prev,
                h,
                difficulty_target,
                sender.address(),
                msg,
                vec![],
                0,
                vec![],
            )
            .unwrap();
            validate_block(&chain, &utxos, &block).unwrap();
            utxos.apply_block(&block).unwrap();
            prev = chain.put_block(&block).unwrap();
        }

        // Build payment tx by scanning UTXOs (same logic as CLI but shorter).
        let mut chosen: Option<(crate::OutPoint, crate::types::utxo::Utxo)> = None;
        utxos
            .for_each(|op, utxo| {
                if utxo.output.script_pubkey.as_bytes().len() == 21
                    && utxo.output.script_pubkey.as_bytes()[0] == 0x14
                    && &utxo.output.script_pubkey.as_bytes()[1..21] == sender.address().as_bytes()
                {
                    // Keep the *oldest* UTXO so it is coinbase-mature.
                    match &chosen {
                        None => chosen = Some((op, utxo)),
                        Some((_best_op, best_utxo)) => {
                            if utxo.height < best_utxo.height {
                                chosen = Some((op, utxo));
                            }
                        }
                    }
                }
                Ok(())
            })
            .unwrap();
        let (op, utxo) = chosen.expect("must have sender utxo");

        let amount = 1 * UNITS_PER_DBC;
        let change = utxo.value() - amount;
        let mut tx = crate::Transaction {
            version: 1,
            inputs: vec![crate::TxInput::new(op, crate::Script::new(vec![]), u32::MAX)],
            outputs: vec![
                crate::TxOutput::new(amount, crate::node::validation::build_p2addr_script_pubkey(receiver.address())),
                crate::TxOutput::new(change, crate::node::validation::build_p2addr_script_pubkey(sender.address())),
            ],
            locktime: 0,
        };

        let msg32 = sighash(&tx, 0).unwrap();
        let sig = sender.sign(&msg32);
        tx.inputs[0].script_sig =
            crate::node::validation::build_script_sig(&sig, sender.pubkey32());

        validate_transaction(&utxos, &tx, 4).unwrap();

        let b = Miner::mine_next_block(
            prev,
            5,
            difficulty_target,
            sender.address(),
            msg,
            vec![tx],
            0,
            vec![],
        )
        .unwrap();
        validate_block(&chain, &utxos, &b).unwrap();
        utxos.apply_block(&b).unwrap();
    }
}


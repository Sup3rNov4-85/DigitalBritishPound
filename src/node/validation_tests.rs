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
            vec![(tx, 0)],
            vec![],
        )
        .unwrap();
        validate_block(&chain, &utxos, &b).unwrap();
        utxos.apply_block(&b).unwrap();
    }

    /// Mine `n` canonical blocks into fresh chain/utxo databases.
    fn mine_chain(
        tag: &str,
        n: u64,
        payout: crate::crypto::wallet::Address,
    ) -> (ChainDb, UTXOSet, Vec<Hash>) {
        let root = tmpdir(tag);
        let chain = ChainDb::open(root.join("chain")).unwrap();
        let utxos = UTXOSet::new(root.join("utxo")).unwrap();
        let mut hashes = Vec::new();
        let mut prev = Hash::ZERO;
        for h in 0..n {
            let block = Miner::mine_next_block(
                prev,
                h,
                0x1f00_ffff,
                payout,
                b"genesis",
                vec![],
                vec![],
            )
            .unwrap();
            validate_block(&chain, &utxos, &block).unwrap();
            utxos.apply_block(&block).unwrap();
            prev = chain.put_block(&block).unwrap();
            hashes.push(prev);
        }
        (chain, utxos, hashes)
    }

    #[test]
    fn wrong_difficulty_target_rejected() {
        let root = tmpdir("bad_diff");
        let chain = ChainDb::open(root.join("chain")).unwrap();
        let utxos = UTXOSet::new(root.join("utxo")).unwrap();
        let w = Wallet::from_mnemonic(&Wallet::generate_mnemonic()).unwrap();

        // Genesis must use the consensus genesis target; this one is different.
        let block = Miner::mine_next_block(
            Hash::ZERO,
            0,
            0x1f00_fffe,
            w.address(),
            b"genesis",
            vec![],
            vec![],
        )
        .unwrap();
        let err = validate_block(&chain, &utxos, &block).unwrap_err();
        assert!(matches!(
            err,
            crate::node::validation::ValidationError::BadDifficultyTarget { .. }
        ));
    }

    #[test]
    fn far_future_timestamp_rejected() {
        let root = tmpdir("future_ts");
        let chain = ChainDb::open(root.join("chain")).unwrap();
        let utxos = UTXOSet::new(root.join("utxo")).unwrap();
        let w = Wallet::from_mnemonic(&Wallet::generate_mnemonic()).unwrap();

        let mut block = Miner::mine_next_block(
            Hash::ZERO,
            0,
            0x1f00_ffff,
            w.address(),
            b"genesis",
            vec![],
            vec![],
        )
        .unwrap();
        block.header.timestamp = block.header.timestamp.saturating_add(3 * 60 * 60);
        let err = validate_block(&chain, &utxos, &block).unwrap_err();
        assert!(matches!(
            err,
            crate::node::validation::ValidationError::TimestampTooFarInFuture
        ));
    }

    /// Build a stale (non-canonical) block at height 1 forking off genesis.
    fn stale_block_at_1(genesis_hash: Hash) -> crate::Block {
        let other = Wallet::from_mnemonic(&Wallet::generate_mnemonic()).unwrap();
        Miner::mine_next_block(
            genesis_hash,
            1,
            0x1f00_ffff,
            other.address(),
            b"stale",
            vec![],
            vec![],
        )
        .unwrap()
    }

    #[test]
    fn valid_uncle_accepted() {
        let w = Wallet::from_mnemonic(&Wallet::generate_mnemonic()).unwrap();
        let (chain, utxos, hashes) = mine_chain("uncle_ok", 4, w.address());
        let stale = stale_block_at_1(hashes[0]);

        let nephew = Miner::mine_next_block(
            hashes[3],
            4,
            0x1f00_ffff,
            w.address(),
            b"genesis",
            vec![],
            vec![stale.header.clone()],
        )
        .unwrap();
        validate_block(&chain, &utxos, &nephew).unwrap();
    }

    #[test]
    fn duplicate_uncles_in_block_rejected() {
        let w = Wallet::from_mnemonic(&Wallet::generate_mnemonic()).unwrap();
        let (chain, utxos, hashes) = mine_chain("uncle_dup", 4, w.address());
        let stale = stale_block_at_1(hashes[0]);

        let nephew = Miner::mine_next_block(
            hashes[3],
            4,
            0x1f00_ffff,
            w.address(),
            b"genesis",
            vec![],
            vec![stale.header.clone(), stale.header.clone()],
        )
        .unwrap();
        let err = validate_block(&chain, &utxos, &nephew).unwrap_err();
        assert!(matches!(
            err,
            crate::node::validation::ValidationError::UncleDuplicate
        ));
    }

    #[test]
    fn canonical_block_as_uncle_rejected() {
        let w = Wallet::from_mnemonic(&Wallet::generate_mnemonic()).unwrap();
        let (chain, utxos, hashes) = mine_chain("uncle_canon", 4, w.address());
        let canonical_1 = chain.get_block_at_height(1).unwrap().unwrap();

        let nephew = Miner::mine_next_block(
            hashes[3],
            4,
            0x1f00_ffff,
            w.address(),
            b"genesis",
            vec![],
            vec![canonical_1.header.clone()],
        )
        .unwrap();
        let err = validate_block(&chain, &utxos, &nephew).unwrap_err();
        assert!(matches!(
            err,
            crate::node::validation::ValidationError::UncleIsCanonical
        ));
    }
}


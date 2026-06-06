//! Integration-style tests for wallet queries (no network).

use dbc_node::{
    crypto::wallet::Wallet,
    node::{
        chain::Chain,
        genesis::import_genesis_json,
        wallet_query::{balance_for_address, history_for_address},
    },
};

#[test]
fn balance_after_imported_genesis() {
    let dir = tempfile::tempdir().unwrap();
    let chain = Chain::open(dir.path()).unwrap();
    let m = Wallet::generate_mnemonic();
    let w = Wallet::from_mnemonic(&m).unwrap();

    let genesis_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("genesis.json");
    if !genesis_path.exists() {
        return; // skip if genesis.json not in repo root during dev
    }
    let block = import_genesis_json(&genesis_path).unwrap();
    chain.accept_block(&block).unwrap().unwrap();

    let summary = balance_for_address(&chain, &w.address(), false).unwrap();
    assert_eq!(summary.confirmed_pence, 0);

    let hist = history_for_address(&chain, &w.address(), 5).unwrap();
    assert!(hist.is_empty() || hist[0].height == 0);
}

mod common;

use common::*;
use lianad::miniscript::bitcoin::{Amount, Network};
use lianad::DaemonHandle;
use serde_json::json;
use serial_test::serial;

#[test]
#[serial]
fn unconfirmed_deposit() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let miner = setup_miner(&bitcoind);
    mine_blocks(&bitcoind.client, 101);

    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    let addr = match &handle {
        DaemonHandle::Controller { control, .. } => control.get_new_address().address.to_string(),
        _ => unreachable!(),
    };
    let _txid: String = bitcoind
        .client
        .call("sendtoaddress", &[json!(addr), json!(0.42)])
        .expect("sendtoaddress");

    // Use longer timeout for electrum backend due to mempool indexing lag
    let timeout = match node_kind() {
        NodeKind::Bitcoind => 15_000,
        NodeKind::Electrs => 30_000,
    };
    wait_for_coins_len(&handle, 1, timeout);

    match &handle {
        DaemonHandle::Controller { control, .. } => {
            let coins = control.list_coins(&[], &[]).coins;
            assert_eq!(coins.len(), 1);
            let c = &coins[0];
            assert_eq!(c.amount, Amount::from_btc(0.42).unwrap());
            assert!(c.block_height.is_none());
            assert!(!c.is_change);
            assert!(!c.is_from_self);
        }
        _ => unreachable!(),
    }

    handle.stop().unwrap();
}

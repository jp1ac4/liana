mod common;

use common::*;
// Using corepc-client v29 via electrsd re-exports
use lianad::commands::LabelItem;
use lianad::miniscript::bitcoin::Network;
use lianad::DaemonHandle;
use serde_json::json;
use serial_test::serial;
use std::collections::{HashMap, HashSet};

#[test]
#[serial]
fn labels_roundtrip() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let miner = setup_miner(&bitcoind);
    mine_blocks(&bitcoind.client, 101);

    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    let (addr, outpoint) = match &handle {
        DaemonHandle::Controller { control, .. } => {
            let address = control.get_new_address().address;
            let _txid: String = bitcoind
                .client
                .call("sendtoaddress", &[json!(address.to_string()), json!(0.123)])
                .expect("sendtoaddress");
            mine_blocks(&bitcoind.client, 1);
            wait_for_height_match(&handle, &bitcoind.client, 10_000);
            wait_for_coins_len(&handle, 1, 15_000);
            let c = control.list_coins(&[], &[]).coins[0].clone();
            (address, c.outpoint)
        }
        _ => unreachable!(),
    };

    let addr_key = addr.to_string();
    let outpoint_key = outpoint.to_string();
    let txid_key = outpoint.txid.to_string();

    let addr_item = LabelItem::from(addr.clone());
    let op_item = LabelItem::from(outpoint.clone());
    let txid_item = LabelItem::from(outpoint.txid);

    let mut updates = HashMap::new();
    updates.insert(addr_item.clone(), Some("Deposit A".to_string()));
    updates.insert(op_item.clone(), Some("UTXO #1".to_string()));
    updates.insert(txid_item.clone(), Some("Tx for UTXO #1".to_string()));

    match &handle {
        DaemonHandle::Controller { control, .. } => {
            control.update_labels(&updates);
            let mut query = HashSet::new();
            query.insert(addr_item);
            query.insert(op_item);
            query.insert(txid_item);
            let res = control.get_labels(&query);
            assert_eq!(res.labels.get(&addr_key).unwrap(), "Deposit A");
            assert_eq!(res.labels.get(&outpoint_key).unwrap(), "UTXO #1");
            assert_eq!(res.labels.get(&txid_key).unwrap(), "Tx for UTXO #1");
        }
        _ => unreachable!(),
    }

    handle.stop().unwrap();
}

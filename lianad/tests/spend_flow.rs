mod common;

use common::*;
// corepc-client v29 is used via helper rpc clients
use liana::signer::HotSigner;
use lianad::miniscript::bitcoin::{Network, Txid};
use lianad::{commands::CreateSpendResult, DaemonHandle};
use miniscript::bitcoin::secp256k1;
use serde_json::json;
use serial_test::serial;
use std::collections::HashMap;

#[test]
#[serial]
fn spend_create_and_store() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let miner = setup_miner(&bitcoind);
    mine_blocks(&bitcoind.client, 101);

    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    let (_recv_addr, coin_op) = match &handle {
        DaemonHandle::Controller { control, .. } => {
            let recv = control.get_new_address().address;
            let _ = bitcoind
                .client
                .call::<String>("sendtoaddress", &[json!(recv.to_string()), json!(0.01)])
                .expect("sendtoaddress");
            mine_blocks(&bitcoind.client, 1);
            wait_for_height_match(&handle, &bitcoind.client, 10_000);
            wait_for_coins_len(&handle, 1, 15_000);
            let c = control.list_coins(&[], &[]).coins[0].clone();
            (recv, c.outpoint)
        }
        _ => unreachable!(),
    };

    // Create a spend paying to an external address at ~2 sat/vb.
    match &handle {
        DaemonHandle::Controller { control, .. } => {
            let dest_addr: String = bitcoind.client.call("getnewaddress", &[]).unwrap();
            let dest_addr = dest_addr.parse().unwrap();
            let mut destinations = HashMap::new();
            destinations.insert(dest_addr, 100_000u64); // 0.001 BTC
            let res = control.create_spend(&destinations, &[coin_op], 2, None);
            // Sign, store, and broadcast the PSBT.
            match res.expect("create_spend") {
                CreateSpendResult::Success { psbt, warnings } => {
                    assert!(warnings.len() <= 1);
                    let secp = secp256k1::Secp256k1::new();
                    let signer = HotSigner::from_str(Network::Regtest, "burger ball theme dog light account produce chest warrior swarm flip equip").unwrap();
                    let signed = signer.sign_psbt(psbt, &secp).expect("sign psbt");
                    let txid: Txid = signed.unsigned_tx.compute_txid();
                    control
                        .update_spend(signed)
                        .expect("update_spend stores PSBT");
                    control.broadcast_spend(&txid).expect("broadcast spend");
                    // Confirm and check for a confirmed change coin from self.
                    mine_blocks(&bitcoind.client, 1);
                    wait_for_height_match(&handle, &bitcoind.client, 10_000);
                    let coins = control.list_coins(&[], &[]).coins;
                    assert!(coins
                        .iter()
                        .any(|c| c.is_change && c.is_from_self && c.block_height.is_some()));
                }
                CreateSpendResult::InsufficientFunds { .. } => {
                    panic!("unexpected insufficient funds")
                }
            }

            // Create a self-send (sweep) using the newly created change coin and no destinations;
            // must require outpoint and produce 1 output.
            let coins = control.list_coins(&[], &[]).coins;
            let change_op = coins
                .iter()
                .find(|c| c.is_change && c.is_from_self)
                .expect("expected a change coin after broadcast")
                .outpoint;
            let res = control.create_spend(&HashMap::new(), &[change_op], 2, None);
            let psbt = match res.expect("create_spend self-send") {
                CreateSpendResult::Success { psbt, warnings } => {
                    assert!(warnings.is_empty());
                    psbt
                }
                _ => panic!("self-send should succeed with provided outpoint"),
            };
            assert_eq!(psbt.unsigned_tx.output.len(), 1);

            // Verify indices moved: change index must increment after self-send creation when address reserved.
            let info = control.get_info();
            assert!(info.change_index >= 1);

            // Basic coin selection: with no coins filtered, auto-select should work for small amount after confirmation exists.
            let dest_addr: String = bitcoind.client.call("getnewaddress", &[]).unwrap();
            let dest_addr = dest_addr.parse().unwrap();
            let mut destinations = HashMap::new();
            destinations.insert(dest_addr, 50_000u64);
            let res = control.create_spend(&destinations, &[], 2, None);
            match res.expect("create_spend autoselect") {
                CreateSpendResult::Success { psbt, warnings } => {
                    // Should have at least 1 output for dest and possibly change
                    assert!(psbt.unsigned_tx.output.len() >= 1);
                    assert!(warnings.len() <= 1);
                }
                _ => panic!("autoselect should succeed with confirmed coin"),
            }
        }
        _ => unreachable!(),
    }

    handle.stop().unwrap();
}

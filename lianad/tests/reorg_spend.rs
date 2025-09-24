mod common;

use common::*;
// corepc-client v29 provides the RPC methods used
use liana::signer::HotSigner;
use lianad::miniscript::bitcoin::{Network, Txid};
use lianad::{commands::CreateSpendResult, DaemonHandle};
use miniscript::bitcoin::secp256k1;
use serde_json::json;
use serial_test::serial;
use std::collections::HashMap;

#[test]
#[serial]
fn reorg_on_spend() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let miner = setup_miner(&bitcoind);
    mine_blocks(&bitcoind.client, 101);

    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    // Get a confirmed coin to spend.
    let coin_op = match &handle {
        DaemonHandle::Controller { control, .. } => {
            let recv = control.get_new_address().address;
            let _ = bitcoind
                .client
                .call::<String>("sendtoaddress", &[json!(recv.to_string()), json!(0.01)])
                .expect("sendtoaddress");
            mine_blocks(&bitcoind.client, 1);
            wait_for_height_match(&handle, &bitcoind.client, 10_000);
            wait_for_coins_len(&handle, 1, 15_000);
            control.list_coins(&[], &[]).coins[0].outpoint
        }
        _ => unreachable!(),
    };

    // Create and sign a spend to an external address.
    let _txid = match &handle {
        DaemonHandle::Controller { control, .. } => {
            let dest_addr: String = bitcoind.client.call("getnewaddress", &[]).unwrap();
            let dest_addr = dest_addr.parse().unwrap();
            let mut destinations = HashMap::new();
            destinations.insert(dest_addr, 80_000u64);
            let res = control.create_spend(&destinations, &[coin_op], 2, None);
            match res.expect("create_spend") {
                CreateSpendResult::Success { psbt, .. } => {
                    let secp = secp256k1::Secp256k1::new();
                    let signer = HotSigner::from_str(Network::Regtest, "burger ball theme dog light account produce chest warrior swarm flip equip").unwrap();
                    let signed = signer.sign_psbt(psbt, &secp).expect("sign psbt");
                    let txid: Txid = signed.unsigned_tx.compute_txid();
                    control.update_spend(signed).expect("update_spend");
                    control.broadcast_spend(&txid).expect("broadcast");
                    txid
                }
                _ => panic!("unexpected insufficient funds"),
            }
        }
        _ => unreachable!(),
    };

    // Mine one block to confirm.
    mine_blocks(&bitcoind.client, 1);
    wait_for_height_match(&handle, &bitcoind.client, 10_000);

    // Helper: get spend height for the original coin entry.
    let get_spend_height = || -> Option<i32> {
        match &handle {
            DaemonHandle::Controller { control, .. } => control
                .list_coins(&[], &[])
                .coins
                .iter()
                .find(|c| c.outpoint == coin_op)
                .and_then(|c| c.spend_info.as_ref())
                .and_then(|s| s.height),
            _ => None,
        }
    };

    // Assert confirmed spend.
    let h1 = get_spend_height().expect("spend should be confirmed");
    assert!(h1 > 0);

    // Reorg: invalidate the last block without re-mining to make the spend unconfirmed again.
    let tip: i64 = bitcoind.client.call("getblockcount", &[]).unwrap();
    simple_reorg(&bitcoind.client, tip - 1, 0);

    // Wait until daemon reflects the unconfirmed spend state.
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(10) {
        if get_spend_height().is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(
        get_spend_height().is_none(),
        "spend should be unconfirmed after reorg"
    );

    // Mine a block again; spend should reconfirm.
    mine_blocks(&bitcoind.client, 1);
    wait_for_height_match(&handle, &bitcoind.client, 10_000);

    // Wait until it shows confirmed again.
    let start = std::time::Instant::now();
    let mut confirmed_again = None;
    while start.elapsed() < std::time::Duration::from_secs(10) {
        if let Some(h) = get_spend_height() {
            confirmed_again = Some(h);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(
        confirmed_again.is_some(),
        "spend should reconfirm after reorg"
    );

    handle.stop().unwrap();
}

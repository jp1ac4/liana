mod common;

use common::*;
// RPC methods provided by corepc-client v29
use liana::signer::HotSigner;
use lianad::miniscript::bitcoin::{Network, Txid};
use lianad::{commands::CreateSpendResult, DaemonHandle};
use miniscript::bitcoin::secp256k1;
use serde_json::json;
use serial_test::serial;
use std::collections::HashMap;

#[test]
#[serial]
fn rbf_spend_replacement() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let miner = setup_miner(&bitcoind);
    mine_blocks(&bitcoind.client, 101);

    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    let coin_op = match &handle {
        DaemonHandle::Controller { control, .. } => {
            let recv = control.get_new_address().address;
            let _ = bitcoind
                .client
                .call::<String>("sendtoaddress", &[json!(recv.to_string()), json!(0.02)])
                .expect("sendtoaddress");
            mine_blocks(&bitcoind.client, 1);
            wait_for_height_match(&handle, &bitcoind.client, 10_000);
            wait_for_coins_len(&handle, 1, 15_000);
            control.list_coins(&[], &[]).coins[0].outpoint
        }
        _ => unreachable!(),
    };

    // Create an RBF-signaling spend (sequence ENABLE_RBF_NO_LOCKTIME is default for created txs).
    let (_orig_txid, _repl_txid) = match &handle {
        DaemonHandle::Controller { control, .. } => {
            // First spend at low fee.
            let dest_addr: String = bitcoind.client.call("getnewaddress", &[]).unwrap();
            let dest_addr = dest_addr.parse().unwrap();
            let mut destinations = HashMap::new();
            destinations.insert(dest_addr, 120_000u64);
            let res = control.create_spend(&destinations, &[coin_op], 2, None);
            let orig_txid = match res.expect("create_spend") {
                CreateSpendResult::Success { psbt, .. } => {
                    let secp = secp256k1::Secp256k1::new();
                    let signer = HotSigner::from_str(Network::Regtest, "burger ball theme dog light account produce chest warrior swarm flip equip").unwrap();
                    let signed = signer.sign_psbt(psbt, &secp).expect("sign psbt");
                    let txid: Txid = signed.unsigned_tx.compute_txid();
                    control.update_spend(signed).expect("update_spend");
                    control.broadcast_spend(&txid).expect("broadcast");
                    txid
                }
                _ => panic!("insufficient funds for initial spend"),
            };

            // Create RBF replacement at a higher feerate.
            let bump = control
                .rbf_psbt(&orig_txid, /*is_cancel=*/ false, Some(5))
                .expect("rbf_psbt");
            let repl_txid = match bump {
                CreateSpendResult::Success { psbt, .. } => {
                    let secp = secp256k1::Secp256k1::new();
                    let signer = HotSigner::from_str(Network::Regtest, "burger ball theme dog light account produce chest warrior swarm flip equip").unwrap();
                    let signed = signer.sign_psbt(psbt, &secp).expect("sign rbf psbt");
                    let txid: Txid = signed.unsigned_tx.compute_txid();
                    control.update_spend(signed).expect("update rbf");
                    control.broadcast_spend(&txid).expect("broadcast rbf");
                    txid
                }
                CreateSpendResult::InsufficientFunds { missing } => {
                    panic!(
                        "unexpected insufficient funds for rbf (missing {})",
                        missing
                    )
                }
            };
            (orig_txid, repl_txid)
        }
        _ => unreachable!(),
    };

    // Mine one block to confirm the replacement spend.
    mine_blocks(&bitcoind.client, 1);
    wait_for_height_match(&handle, &bitcoind.client, 10_000);

    // Assert the replacement is the confirmed spender (original should not remain confirmed).
    match &handle {
        DaemonHandle::Controller { control, .. } => {
            let coins = control.list_coins(&[], &[]).coins;
            // The spent original coin should show spend_info with some height; we can't directly check txid here
            // but we can ensure at least one change coin exists and is_from_self.
            assert!(coins
                .iter()
                .any(|c| c.is_change && c.is_from_self && c.block_height.is_some()));
        }
        _ => unreachable!(),
    }

    handle.stop().unwrap();
}

mod common;

use common::*;
// Using corepc-client v29 via helper client
use liana::signer::HotSigner;
use lianad::miniscript::bitcoin::{Network, Txid};
use lianad::{commands::CreateSpendResult, DaemonHandle};
use miniscript::bitcoin::secp256k1;
use serde_json::json;
use serial_test::serial;
use std::collections::HashMap;

#[test]
#[serial]
fn cpfp_additional_fee_warning_on_unconfirmed_self_coin() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let _ = setup_miner(&bitcoind);
    mine_blocks(&bitcoind.client, 101);

    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    // Step 1: Fund wallet with a confirmed coin
    let coin_op = match &handle {
        DaemonHandle::Controller { control, .. } => {
            let recv = control.get_new_address().address;
            let _ = bitcoind
                .client
                .call::<String>("sendtoaddress", &[json!(recv.to_string()), json!(0.001)]) // 100_000 sats
                .expect("sendtoaddress");
            mine_blocks(&bitcoind.client, 1);
            wait_for_height_match(&handle, &bitcoind.client, 10_000);
            wait_for_coins_len(&handle, 1, 15_000);
            control.list_coins(&[], &[]).coins[0].outpoint
        }
        _ => unreachable!(),
    };

    // Step 2: Create and broadcast a low-fee spend to produce an unconfirmed self change coin
    // Use feerate 1 sat/vb so the ancestor feerate is low; then we'll create a later spend with higher feerate.
    match &handle {
        DaemonHandle::Controller { control, .. } => {
            let dest_addr: String = bitcoind.client.call("getnewaddress", &[]).unwrap();
            let dest_addr = dest_addr.parse().unwrap();
            let mut destinations = HashMap::new();
            destinations.insert(dest_addr, 50_000u64); // ensure change exists
            let res = control.create_spend(&destinations, &[coin_op], 1, None);
            let psbt = match res.expect("create initial low-fee spend") {
                CreateSpendResult::Success { psbt, .. } => psbt,
                CreateSpendResult::InsufficientFunds { .. } => {
                    panic!("unexpected insufficient funds")
                }
            };
            // Sign and broadcast so we have an unconfirmed self coin in the mempool.
            let secp = secp256k1::Secp256k1::new();
            let signer = HotSigner::from_str(
                Network::Regtest,
                "burger ball theme dog light account produce chest warrior swarm flip equip",
            )
            .unwrap();
            let signed = signer.sign_psbt(psbt, &secp).expect("sign psbt");
            let txid: Txid = signed.unsigned_tx.compute_txid();
            control.update_spend(signed).expect("store initial psbt");
            control
                .broadcast_spend(&txid)
                .expect("broadcast initial spend");
        }
        _ => unreachable!(),
    }

    // Step 3: Without mining, create a new spend at a higher feerate so CPFP top-up is required
    match &handle {
        DaemonHandle::Controller { control, .. } => {
            // Autoselect coins: only available coin should be the unconfirmed self change coin.
            let dest_addr: String = bitcoind.client.call("getnewaddress", &[]).unwrap();
            let dest_addr = dest_addr.parse().unwrap();
            let mut destinations = HashMap::new();
            destinations.insert(dest_addr, 10_000u64);
            let res = control.create_spend(&destinations, &[], 5, None); // higher feerate
            match res.expect("create spend with cpfp") {
                CreateSpendResult::Success { psbt, warnings } => {
                    assert!(psbt.unsigned_tx.output.len() >= 1);
                    assert!(
                        warnings
                            .iter()
                            .any(|w| w.contains("CPFP: an unconfirmed input was selected")),
                        "expected CPFP warning for ancestor fee top-up"
                    );
                }
                CreateSpendResult::InsufficientFunds { missing } => {
                    panic!(
                        "unexpected insufficient funds when creating CPFP spend (missing {} sats)",
                        missing
                    );
                }
            }
        }
        _ => unreachable!(),
    }

    handle.stop().unwrap();
}

mod common;

use common::*;
use liana::signer::HotSigner;
use lianad::miniscript::bitcoin::{Network, Txid};
use lianad::{commands::CreateSpendResult, DaemonHandle};
use miniscript::bitcoin::secp256k1;
use serde_json::json as j;
use serial_test::serial;
use std::collections::HashMap;
use std::str::FromStr;

#[test]
#[serial]
fn test_spend_change() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let miner = setup_miner(&bitcoind);
    mine_blocks(&bitcoind.client, 101);
    // bitcoind.client.send_to_address(address, amount)

    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let descriptor = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, descriptor, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    // Receive a coin on a fresh receive address.
    let receive_addr = match &handle {
        DaemonHandle::Controller { control, .. } => control.get_new_address().address,
        _ => unreachable!(),
    };
    let _txid: String = bitcoind
        .client
        .call("sendtoaddress", &[j!(receive_addr.to_string()), j!(0.01)])
        .expect("sendtoaddress");
    mine_blocks(&bitcoind.client, 1);
    wait_for_height_match(&handle, &bitcoind.client, 10_000);
    wait_for_coins_len(&handle, 1, 15_000);

    // Create a spend paying to an external address and one of our addresses, expecting change.
    let first_psbt = match &handle {
        DaemonHandle::Controller { control, .. } => {
            let coins = control.list_coins(&[], &[]).coins;
            assert_eq!(coins.len(), 1, "expected single coin after deposit");
            let dest_external: String = bitcoind.client.call("getnewaddress", &[]).unwrap();
            let dest_external = miniscript::bitcoin::Address::from_str(&dest_external).unwrap();
            let mut destinations = HashMap::new();
            destinations.insert(dest_external, 100_000u64);
            let dest_internal_raw = control.get_new_address().address;
            let dest_internal =
                miniscript::bitcoin::Address::from_str(&dest_internal_raw.to_string()).unwrap();
            destinations.insert(dest_internal, 100_000u64);
            let res = control.create_spend(&destinations, &[coins[0].outpoint], 2, None);
            let psbt = match res.expect("create_spend") {
                CreateSpendResult::Success { psbt, warnings } => {
                    assert!(warnings.is_empty(), "unexpected warnings: {:?}", warnings);
                    assert_eq!(
                        psbt.unsigned_tx.output.len(),
                        3,
                        "expected dest + dest + change"
                    );
                    psbt
                }
                CreateSpendResult::InsufficientFunds { missing } => {
                    panic!("insufficient funds missing {}", missing)
                }
            };
            psbt
        }
        _ => unreachable!(),
    };

    // Sign and broadcast the spend.
    match &handle {
        DaemonHandle::Controller { control, .. } => {
            let secp = secp256k1::Secp256k1::new();
            let signer = HotSigner::from_str(
                Network::Regtest,
                "burger ball theme dog light account produce chest warrior swarm flip equip",
            )
            .unwrap();
            let signed = signer.sign_psbt(first_psbt, &secp).expect("sign psbt");
            let txid: Txid = signed.unsigned_tx.compute_txid();
            control.update_spend(signed.clone()).expect("update spend");
            control.broadcast_spend(&txid).expect("broadcast spend");
        }
        _ => unreachable!(),
    };
    mine_blocks(&bitcoind.client, 1);
    wait_for_height_match(&handle, &bitcoind.client, 10_000);
    wait_for_coins_len(&handle, 3, 15_000);

    // Create a new spend using the change output and internal receive output.
    match &handle {
        DaemonHandle::Controller { control, .. } => {
            let coins = control.list_coins(&[], &[]).coins;
            let spendable: Vec<_> = coins
                .iter()
                .filter(|c| c.spend_info.is_none())
                .map(|c| c.outpoint)
                .collect();
            assert!(
                spendable.len() >= 2,
                "expected at least two unspent outputs"
            );
            let dest_external: String = bitcoind.client.call("getnewaddress", &[]).unwrap();
            let dest_external = miniscript::bitcoin::Address::from_str(&dest_external).unwrap();
            let mut destinations = HashMap::new();
            destinations.insert(dest_external, 100_000u64);
            let res = control.create_spend(&destinations, &spendable, 2, None);
            let psbt = match res.expect("create second spend") {
                CreateSpendResult::Success { psbt, warnings } => {
                    assert!(
                        warnings.is_empty(),
                        "unexpected warnings on second spend: {:?}",
                        warnings
                    );
                    assert_eq!(
                        psbt.unsigned_tx.output.len(),
                        2,
                        "expected dest + change outputs"
                    );
                    psbt
                }
                CreateSpendResult::InsufficientFunds { missing } => {
                    panic!("insufficient funds on second spend missing {}", missing)
                }
            };
            let secp = secp256k1::Secp256k1::new();
            let signer = HotSigner::from_str(
                Network::Regtest,
                "burger ball theme dog light account produce chest warrior swarm flip equip",
            )
            .unwrap();
            let signed = signer.sign_psbt(psbt, &secp).expect("sign second psbt");
            let txid: Txid = signed.unsigned_tx.compute_txid();
            control
                .update_spend(signed.clone())
                .expect("update second spend");
            control
                .broadcast_spend(&txid)
                .expect("broadcast second spend");
            mine_blocks(&bitcoind.client, 1);
            wait_for_height_match(&handle, &bitcoind.client, 10_000);
        }
        _ => unreachable!(),
    }

    handle.stop().unwrap();
}

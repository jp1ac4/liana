mod common;

use common::*;
// Using corepc-client v29 via helper client
use lianad::miniscript::bitcoin::Network;
use lianad::{commands::CreateSpendResult, DaemonHandle};
use serde_json::json;
use serial_test::serial;
use std::collections::HashMap;

#[test]
#[serial]
fn change_below_dust_adds_warning() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let miner = setup_miner(&bitcoind);
    mine_blocks(&bitcoind.client, 101);

    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    let (coin_amount_sat, coin_op) = match &handle {
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
            (c.amount.to_sat(), c.outpoint)
        }
        _ => unreachable!(),
    };

    match &handle {
        DaemonHandle::Controller { control, .. } => {
            let dest_addr: String = bitcoind
                .client
                .call::<String>("getnewaddress", &[])
                .unwrap();
            let dest_addr = dest_addr.parse().unwrap();

            let send_value = coin_amount_sat.saturating_sub(3_000);
            assert!(send_value > 5_000, "send_value should be well above dust");

            let mut destinations = HashMap::new();
            destinations.insert(dest_addr, send_value);
            let res = control.create_spend(&destinations, &[coin_op], 2, None);

            match res.expect("create_spend") {
                CreateSpendResult::Success { psbt, warnings } => {
                    assert_eq!(
                        psbt.unsigned_tx.output.len(),
                        1,
                        "no change output expected"
                    );
                    assert!(
                        !warnings.is_empty(),
                        "expected dust/change warning to be present"
                    );
                    assert!(warnings.iter().any(|w| w.contains("minimal change output") || w.contains("Dust UTXO")), "expected warning to mention dust/minimal change");
                }
                CreateSpendResult::InsufficientFunds { missing } => {
                    panic!("unexpected insufficient funds (missing {} sats) when crafting dust-change spend", missing);
                }
            }
        }
        _ => unreachable!(),
    }

    handle.stop().unwrap();
}

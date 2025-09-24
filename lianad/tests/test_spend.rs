mod common;

use lianad::commands::CreateSpendResult;
use lianad::miniscript::bitcoin::Txid;
use miniscript::bitcoin::address::NetworkUnchecked;
use miniscript::bitcoin::{secp256k1, Address, Amount};
use serial_test::serial;
use std::collections::HashMap;
use std::str::FromStr;

use crate::common::bitcoind::{mine_blocks, new_bitcoind_address, setup_bitcoind};
use crate::common::{get_daemon_control, setup_lianad, test_signer, utils};

#[ignore]
#[test]
#[serial]
fn test_spend_change() {
    let bitcoind = setup_bitcoind().unwrap();
    let handle = setup_lianad(&bitcoind).unwrap();
    let control = get_daemon_control(&handle);

    // Receive a coin on a fresh receive address.
    let receive_addr = control.get_new_address().address;
    let _ = bitcoind
        .client
        .send_to_address(&receive_addr, Amount::from_btc(0.01).unwrap())
        .unwrap();

    mine_blocks(&bitcoind.client, 1);

    utils::wait_for(|| control.list_coins(&[], &[]).coins.len() == 1);

    // Create a spend paying to an external address and one of our addresses, expecting change.
    let coins = control.list_coins(&[], &[]).coins;
    assert_eq!(coins.len(), 1);
    let dest_external = new_bitcoind_address(&bitcoind.client);
    let mut destinations = HashMap::<Address<NetworkUnchecked>, u64>::new();
    destinations.insert(dest_external, 100_000);
    let dest_internal = control.get_new_address().address;
    destinations.insert(dest_internal.as_unchecked().clone(), 100_000);
    let res = control
        .create_spend(&destinations, &[coins[0].outpoint], 2, None)
        .unwrap();
    let CreateSpendResult::Success {
        psbt: first_psbt,
        warnings,
    } = res
    else {
        panic!("expected successful spend creation, got {:?}", res);
    };
    assert!(warnings.is_empty(), "unexpected warnings: {:?}", warnings);
    assert_eq!(first_psbt.unsigned_tx.output.len(), 3);

    let secp = secp256k1::Secp256k1::new();
    let signer = test_signer();
    let signed = signer.sign_psbt(first_psbt, &secp).unwrap();
    let txid: Txid = signed.unsigned_tx.compute_txid();
    control.update_spend(signed.clone()).unwrap();
    control.broadcast_spend(&txid).unwrap();

    mine_blocks(&bitcoind.client, 1);

    let coins = control.list_coins(&[], &[]).coins;
    let spendable: Vec<_> = coins
        .iter()
        .filter(|c| c.spend_info.is_none())
        .map(|c| c.outpoint)
        .collect();
    assert!(spendable.len() >= 2);
    let dest_external: String = bitcoind.client.call("getnewaddress", &[]).unwrap();
    let dest_external = miniscript::bitcoin::Address::from_str(&dest_external).unwrap();
    let mut destinations = HashMap::new();
    destinations.insert(dest_external, 100_000u64);
    let res = control
        .create_spend(&destinations, &spendable, 2, None)
        .unwrap();
    let CreateSpendResult::Success { psbt, warnings } = res else {
        panic!("expected successful second spend creation, got {:?}", res);
    };
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

    let signed = signer.sign_psbt(psbt, &secp).expect("sign second psbt");
    let txid: Txid = signed.unsigned_tx.compute_txid();
    control
        .update_spend(signed.clone())
        .expect("update second spend");
    control
        .broadcast_spend(&txid)
        .expect("broadcast second spend");
    mine_blocks(&bitcoind.client, 1);

    handle.stop().unwrap();
}

mod common;

use common::*;
use electrsd::corepc_client::client_sync::v29 as rpc;
use electrsd::corepc_client::client_sync::Auth as RpcAuth;
use lianad::miniscript::bitcoin::{Amount, Network};
use lianad::DaemonHandle;
use serde_json::json;
use serial_test::serial;

fn setup_miner(bitcoind: &electrsd::corepc_node::Node) -> rpc::Client {
    let url = format!("http://{}", bitcoind.params.rpc_socket);
    let auth = RpcAuth::CookieFile(bitcoind.params.cookie_file.clone());
    let node_client = rpc::Client::new_with_auth(&url, auth.clone()).expect("rpc client");
    // Create and load a wallet for mining/sending. If it already exists, ignore error.
    let _: serde_json::Value = node_client
        .call(
            "createwallet",
            &[
                json!("miner"),
                json!(false),
                json!(false),
                json!(""),
                json!(false),
                json!(true),
                json!(true),
            ],
        )
        .unwrap_or_else(|_| serde_json::Value::Null);
    // Load the wallet explicitly (no-op if already loaded).
    let _: serde_json::Value = node_client
        .call("loadwallet", &[json!("miner")])
        .unwrap_or_else(|_| serde_json::Value::Null);
    // Return a client scoped to the wallet RPC endpoint.
    let wallet_url = format!("{}/wallet/{}", url, "miner");
    rpc::Client::new_with_auth(&wallet_url, auth).expect("wallet rpc client")
}

fn mine_blocks(client: &rpc::Client, n: u64) {
    // Generate blocks to a fresh address from miner wallet.
    let addr: String = client.call("getnewaddress", &[]).expect("getnewaddress");
    let _: serde_json::Value = client
        .call("generatetoaddress", &[json!(n), json!(addr)])
        .expect("generatetoaddress");
}

fn wait_for_height_match(handle: &DaemonHandle, client: &rpc::Client, timeout_ms: u64) {
    let start = std::time::Instant::now();
    loop {
        let bitcoind_height: i64 = client.call("getblockcount", &[]).expect("getblockcount");
        let got = match handle {
            DaemonHandle::Controller { control, .. } => control.get_info().block_height,
            _ => unreachable!("rpc server not used"),
        };
        if got as i64 >= bitcoind_height {
            break;
        }
        if start.elapsed() > std::time::Duration::from_millis(timeout_ms) {
            panic!(
                "timeout waiting for height: lianad {} < bitcoind {}",
                got, bitcoind_height
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

fn wait_for_coins_len(handle: &DaemonHandle, len: usize, timeout_ms: u64) {
    let start = std::time::Instant::now();
    loop {
        let got = match handle {
            DaemonHandle::Controller { control, .. } => control.list_coins(&[], &[]).coins.len(),
            _ => unreachable!("rpc server not used"),
        };
        if got >= len {
            break;
        }
        if start.elapsed() > std::time::Duration::from_millis(timeout_ms) {
            panic!("timeout waiting for coins (got {}, want {}).", got, len);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

// #[test]
// #[serial]
// fn fund_and_mine_backend() {
//     let bitcoind = start_bitcoind().expect("spawn bitcoind");
//     let miner = setup_miner(&bitcoind);
//     mine_blocks(&miner, 101);

//     let datadir = tempfile::TempDir::new().unwrap();
//     let use_electrum = std::env::var("LIANA_TEST_BACKEND")
//         .map(|v| {
//             matches!(
//                 v.to_ascii_lowercase().as_str(),
//                 "electrum" | "electrs" | "electrumd"
//             )
//         })
//         .unwrap_or(false)
//         || std::env::var("LIANA_TEST_ELECTRUM")
//             .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
//             .unwrap_or(false);

//     let (cfg, wait_height, wait_coins) = if use_electrum {
//         let electrs = start_electrs(&bitcoind).expect("spawn electrs");
//         (
//             make_config_for_electrum(Network::Regtest, &datadir, &electrs),
//             20_000,
//             30_000,
//         )
//     } else {
//         (
//             make_config_for_bitcoind(Network::Regtest, &datadir, &bitcoind),
//             10_000,
//             15_000,
//         )
//     };
//     let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
//     assert!(wait_alive(&handle, 500));

//     let addr = match &handle {
//         DaemonHandle::Controller { control, .. } => control.get_new_address().address.to_string(),
//         _ => unreachable!(),
//     };
//     let _txid: String = miner
//         .call("sendtoaddress", &[json!(addr), json!(1.0)])
//         .expect("sendtoaddress");
//     mine_blocks(&miner, 1);

//     wait_for_height_match(&handle, &miner, wait_height);
//     wait_for_coins_len(&handle, 1, wait_coins);

//     match &handle {
//         DaemonHandle::Controller { control, .. } => {
//             let coins = control.list_coins(&[], &[]).coins;
//             assert_eq!(coins.len(), 1);
//             let c = &coins[0];
//             assert_eq!(c.amount, Amount::from_btc(1.0).unwrap());
//             assert!(c.block_height.is_some());
//             assert!(!c.is_change);
//             assert!(!c.is_from_self);
//             assert!(!c.is_immature);
//         }
//         _ => unreachable!(),
//     }

//     handle.stop().unwrap();
// }

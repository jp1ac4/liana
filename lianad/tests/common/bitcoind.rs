use std::thread::sleep;
use std::time::Duration;

use electrsd::corepc_client::client_sync::v29 as rpc;
use electrsd::corepc_node::{Conf, Node, P2P};
use miniscript::bitcoin::{address::NetworkUnchecked, Address};
use serde_json::json;

pub fn start_bitcoind() -> anyhow::Result<Node> {
    let mut conf = Conf::default();
    // TODO: check if we can remove some args here
    conf.args.push("-printtoconsole");
    conf.args.push("-server");
    conf.args.push("-debug");
    conf.args.push("-debugexclude=libevent");
    conf.args.push("-debugexclude=tor");
    conf.args.push("-txindex=1"); // enable txindex for electrs
    conf.args.push("-peertimeout=172800"); // = 2 days (2 * 24 * 60 * 60)
    conf.args.push("-rpcthreads=32");

    conf.p2p = P2P::Yes; // electrs requires p2p port open

    // Check for custom bitcoind binary path
    let exe = match std::env::var("BITCOIND_PATH") {
        Ok(path) if !path.is_empty() => {
            println!("Using custom bitcoind binary");
            path
        }
        _ => {
            println!("Using downloaded bitcoind binary");
            electrsd::corepc_node::downloaded_exe_path()?
        }
    };
    println!("bitcoind binary: {}", exe);
    let bitcoind = Node::with_conf(&exe, &conf)?;
    Ok(bitcoind)
}

pub fn new_bitcoind_address(client: &rpc::Client) -> Address<NetworkUnchecked> {
    client
        .get_new_address(None, None)
        .unwrap()
        .address()
        .unwrap()
}

/// Create bitcoind process and set up wallet.
pub fn setup_bitcoind() -> anyhow::Result<Node> {
    let bitcoind = start_bitcoind().expect("start_bitcoind");

    bitcoind
        .client
        .call::<serde_json::Value>(
            "createwallet",
            &[
                json!("lianad-tests"),
                json!(false),
                json!(false),
                json!(""),
                json!(false),
                json!(true),
                json!(true),
            ],
        )
        .unwrap();

    bitcoind
        .client
        .generate_to_address(101, &bitcoind.client.new_address().unwrap())
        .unwrap();

    while bitcoind.client.get_balance().unwrap().0 < 50.0 {
        sleep(Duration::from_millis(100));
    }

    Ok(bitcoind)
}

/// Generate `num_blocks` blocks to a fresh address using the given wallet RPC client.
pub fn mine_blocks(client: &rpc::Client, num_blocks: usize) {
    let addr = new_bitcoind_address(client);
    client
        .generate_to_address(num_blocks, addr.assume_checked_ref())
        .expect("generate_to_address");
}

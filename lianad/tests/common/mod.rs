pub mod utils;

use std::thread::sleep;
use std::time::Duration;

use electrsd::corepc_client::client_sync::v29 as rpc;
use electrsd::corepc_node::{Conf as BtcConf, Node as BitcoinD, P2P};
use electrsd::{self, ElectrsD};
use liana::descriptors::LianaDescriptor;
use lianad::config::{
    BitcoinBackend, BitcoinConfig, BitcoindConfig, BitcoindRpcAuth, Config as LianadConfig,
    ElectrumConfig,
};
use lianad::datadir::DataDirectory;
use lianad::miniscript::bitcoin::Network;
use serde_json::json;

/// Node selection for tests: bitcoind or electrum, chosen via env var.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Bitcoind,
    Electrs,
}

pub enum Node<'a> {
    Bitcoind(&'a BitcoinD),
    Electrs(ElectrsD),
}

impl<'a> Node<'a> {
    pub fn new(kind: NodeKind, bitcoind: &'a BitcoinD) -> anyhow::Result<Self> {
        match kind {
            NodeKind::Bitcoind => Ok(Node::Bitcoind(bitcoind)),
            NodeKind::Electrs => start_electrs(bitcoind).map(Node::Electrs),
        }
    }

    pub fn backend_config(&self) -> BitcoinBackend {
        match self {
            Node::Bitcoind(d) => BitcoinBackend::Bitcoind(BitcoindConfig {
                rpc_auth: BitcoindRpcAuth::CookieFile(d.params.cookie_file.clone()),
                addr: std::net::SocketAddr::V4(d.params.rpc_socket),
            }),
            Node::Electrs(e) => BitcoinBackend::Electrum(ElectrumConfig {
                addr: format!("tcp://{}", e.electrum_url.clone()),
                validate_domain: true,
            }),
        }
    }
}

/// Returns the node to use for tests.
pub fn node_kind() -> NodeKind {
    match std::env::var("LIANAD_TESTS_NODE_KIND") {
        Ok(v) if v.eq_ignore_ascii_case("electrs") => NodeKind::Electrs,
        _ => NodeKind::Bitcoind,
    }
}

/// Create a config for the current test backend, keeping electrs alive if needed.
pub fn lianad_config(
    network: Network,
    datadir: &tempfile::TempDir,
    liana_desc: LianaDescriptor,
    node: &Node<'_>,
) -> LianadConfig {
    let mut dd = datadir.path().to_path_buf();
    dd.push(network.to_string());
    let data_directory = DataDirectory::new(dd);

    let bitcoin_config = BitcoinConfig {
        network,
        poll_interval_secs: std::time::Duration::from_secs(1),
    };

    LianadConfig::new(
        bitcoin_config,
        Some(node.backend_config()),
        log::LevelFilter::Debug,
        liana_desc,
        data_directory,
    )
}

pub fn start_bitcoind() -> anyhow::Result<BitcoinD> {
    let mut conf = BtcConf::default();
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
    let exe = match std::env::var("BITCOIND_BINARY_PATH") {
        Ok(path) if !path.is_empty() => {
            println!("Using custom bitcoind binary: {}", path);
            path
        }
        _ => {
            println!("Using downloaded bitcoind binary");
            electrsd::corepc_node::downloaded_exe_path()?
        }
    };
    println!("Using bitcoind binary: {}", exe);
    let bitcoind = BitcoinD::with_conf(&exe, &conf)?;
    // bitcoind.client.generate_to_address(nblocks, address)
    Ok(bitcoind)
}

pub fn start_electrs(bitcoind: &BitcoinD) -> anyhow::Result<ElectrsD> {
    let mut conf = electrsd::Conf::default();
    conf.view_stderr = true;
    let exe = electrsd::downloaded_exe_path()
        .ok_or_else(|| anyhow::anyhow!("electrs binary not available"))?;
    let electrs = ElectrsD::with_conf(exe, bitcoind, &conf)?;
    Ok(electrs)
}

/// Whether test runs should use Taproot descriptors instead of legacy (P2WSH).
///
/// Controls:
/// - LIANA_TEST_TAPROOT=1 or LIANA_TEST_TAPROOT=true (case insensitive)
pub fn use_taproot() -> bool {
    match std::env::var("LIANAD_TESTS_USE_TAPROOT") {
        Ok(v) if v == "1" || v.eq_ignore_ascii_case("true") => return true,
        _ => false,
    }
}

pub fn test_descriptor(network: Network, use_taproot: bool) -> liana::descriptors::LianaDescriptor {
    use liana::descriptors::{LianaDescriptor, LianaPolicy, PathInfo};
    use miniscript::bitcoin::{bip32, secp256k1};
    use miniscript::descriptor::{DerivPaths, DescriptorMultiXKey, DescriptorPublicKey, Wildcard};
    use std::str::FromStr;

    // Deterministic single-signer policy we can use to sign spends in tests.
    let secp = secp256k1::Secp256k1::signing_only();
    let signer = liana::signer::HotSigner::from_str(
        network,
        "burger ball theme dog light account produce chest warrior swarm flip equip",
    )
    .expect("valid mnemonic");

    let fg = signer.fingerprint(&secp);
    let xkey = signer.xpub_at(&bip32::DerivationPath::master(), &secp);
    let primary_key = DescriptorPublicKey::MultiXPub(DescriptorMultiXKey {
        origin: Some((fg, bip32::DerivationPath::master())),
        xkey,
        derivation_paths: DerivPaths::new(vec![
            bip32::DerivationPath::from_str("m/0").unwrap(),
            bip32::DerivationPath::from_str("m/1").unwrap(),
        ])
        .expect("valid deriv paths"),
        wildcard: Wildcard::Unhardened,
    });
    // Use the same xpub but different indices to avoid duplicate key across paths.
    let recov_key = DescriptorPublicKey::MultiXPub(DescriptorMultiXKey {
        origin: Some((fg, bip32::DerivationPath::master())),
        xkey,
        derivation_paths: DerivPaths::new(vec![
            bip32::DerivationPath::from_str("m/2").unwrap(),
            bip32::DerivationPath::from_str("m/3").unwrap(),
        ])
        .expect("valid deriv paths"),
        wildcard: Wildcard::Unhardened,
    });

    let mut recov = std::collections::BTreeMap::new();
    recov.insert(42u16, PathInfo::Single(recov_key));

    // Choose descriptor type based on env toggles (default: legacy P2WSH).
    let policy = if use_taproot {
        LianaPolicy::new(PathInfo::Single(primary_key), recov).expect("taproot policy")
    } else {
        LianaPolicy::new_legacy(PathInfo::Single(primary_key), recov).expect("legacy policy")
    };
    LianaDescriptor::new(policy)
}

// pub fn wait_alive(handle: &DaemonHandle, timeout_ms: u64) -> bool {
//     let start = std::time::Instant::now();
//     while start.elapsed() < std::time::Duration::from_millis(timeout_ms) {
//         if handle.is_alive() {
//             return true;
//         }
//         std::thread::sleep(std::time::Duration::from_millis(50));
//     }
//     false
// }

/// Create wallet.
pub fn run_setup(bitcoind: &BitcoinD) {
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
}

/// Generate n blocks to a fresh address using the given wallet RPC client.
pub fn mine_blocks(client: &rpc::Client, n: u64) {
    let addr: String = client.call("getnewaddress", &[]).expect("getnewaddress");
    let _: serde_json::Value = client
        .call("generatetoaddress", &[json!(n), json!(addr)])
        .expect("generatetoaddress");
}

// /// Wait until lianad's reported height catches up with bitcoind's.
// pub fn wait_for_height_match(handle: &DaemonHandle, client: &rpc::Client, timeout_ms: u64) {
//     let start = std::time::Instant::now();
//     loop {
//         let bitcoind_height: i64 = client.call("getblockcount", &[]).expect("getblockcount");
//         let got = match handle {
//             DaemonHandle::Controller { control, .. } => control.get_info().block_height,
//             _ => unreachable!("rpc server not used"),
//         };
//         if got as i64 >= bitcoind_height {
//             break;
//         }
//         if start.elapsed() > std::time::Duration::from_millis(timeout_ms) {
//             panic!(
//                 "timeout waiting for height: lianad {} < bitcoind {}",
//                 got, bitcoind_height
//             );
//         }
//         std::thread::sleep(std::time::Duration::from_millis(200));
//     }
// }

// /// Wait until list_coins returns at least `len` entries.
// pub fn wait_for_coins_len(handle: &DaemonHandle, len: usize, timeout_ms: u64) {
//     let start = std::time::Instant::now();
//     loop {
//         let got = match handle {
//             DaemonHandle::Controller { control, .. } => control.list_coins(&[], &[]).coins.len(),
//             _ => unreachable!("rpc server not used"),
//         };
//         if got >= len {
//             break;
//         }
//         if start.elapsed() > std::time::Duration::from_millis(timeout_ms) {
//             panic!("timeout waiting for coins (got {}, want {}).", got, len);
//         }
//         std::thread::sleep(std::time::Duration::from_millis(200));
//     }
// }

pub mod bitcoind;
pub mod utils;

use electrsd::corepc_node::Node as BitcoinD;
use electrsd::{self, ElectrsD};
use liana::descriptors::{LianaDescriptor, LianaPolicy, PathInfo};
use lianad::config::{
    BitcoinBackend, BitcoinConfig, BitcoindConfig, BitcoindRpcAuth, Config as LianadConfig,
    ElectrumConfig,
};
use lianad::datadir::DataDirectory;
use lianad::miniscript::bitcoin::Network;
use lianad::{DaemonControl, DaemonHandle};
use miniscript::bitcoin::{bip32, secp256k1};
use miniscript::descriptor::{DerivPaths, DescriptorMultiXKey, DescriptorPublicKey, Wildcard};
use std::str::FromStr;

/// Node kind used by lianad process.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Bitcoind,
    Electrs, // we may also want to test against other electrum servers in the future
}

/// Node instance used by lianad process.
pub enum Node<'a> {
    Bitcoind(&'a BitcoinD),
    Electrs(Box<ElectrsD>),
}

impl<'a> Node<'a> {
    pub fn new(kind: NodeKind, bitcoind: &'a BitcoinD) -> anyhow::Result<Self> {
        match kind {
            NodeKind::Bitcoind => Ok(Node::Bitcoind(bitcoind)),
            NodeKind::Electrs => start_electrs(bitcoind).map(|e| Node::Electrs(Box::new(e))),
        }
    }

    pub fn lianad_backend_config(&self) -> BitcoinBackend {
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
    match std::env::var("BITCOIN_BACKEND_TYPE") {
        Ok(v) if v.eq_ignore_ascii_case("electrs") => NodeKind::Electrs,
        _ => NodeKind::Bitcoind,
    }
}

/// Create config file for lianad process.
pub fn lianad_config(
    network: Network,
    datadir: &tempfile::TempDir,
    liana_desc: LianaDescriptor,
    node: &Node<'_>,
) -> LianadConfig {
    let bitcoin_config = BitcoinConfig {
        network,
        poll_interval_secs: std::time::Duration::from_secs(1),
    };

    LianadConfig::new(
        bitcoin_config,
        Some(node.lianad_backend_config()),
        log::LevelFilter::Debug,
        liana_desc,
        DataDirectory::new(datadir.path().to_path_buf()),
    )
}

pub fn start_electrs(bitcoind: &BitcoinD) -> anyhow::Result<ElectrsD> {
    let mut conf = electrsd::Conf::default();
    conf.view_stderr = true;
    let exe_path = match std::env::var("ELECTRS_PATH") {
        Ok(path) if !path.is_empty() => {
            println!("Using custom electrs binary");
            path
        }
        _ => {
            println!("Using downloaded electrs binary");
            electrsd::downloaded_exe_path()
                .ok_or_else(|| anyhow::anyhow!("electrs binary not available"))?
        }
    };
    println!("electrs binary: {}", exe_path);
    let electrs = ElectrsD::with_conf(exe_path, bitcoind, &conf)?;
    Ok(electrs)
}

/// Whether test runs should use Taproot descriptors.
pub fn use_taproot() -> bool {
    matches!(std::env::var("USE_TAPROOT"), Ok(v) if v == "1" || v.eq_ignore_ascii_case("true"))
}

pub fn test_signer() -> liana::signer::HotSigner {
    liana::signer::HotSigner::from_str(
        Network::Regtest,
        "burger ball theme dog light account produce chest warrior swarm flip equip",
    )
    .expect("valid mnemonic")
}

pub fn test_descriptor(use_taproot: bool) -> liana::descriptors::LianaDescriptor {
    // Deterministic single-signer policy we can use to sign spends in tests.
    let secp = secp256k1::Secp256k1::signing_only();
    let signer = test_signer();

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

    let policy = if use_taproot {
        LianaPolicy::new(PathInfo::Single(primary_key), recov).expect("taproot policy")
    } else {
        LianaPolicy::new_legacy(PathInfo::Single(primary_key), recov).expect("legacy policy")
    };
    LianaDescriptor::new(policy)
}

pub fn setup_lianad(bitcoind: &BitcoinD) -> anyhow::Result<DaemonHandle> {
    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), bitcoind).unwrap();
    let descriptor = test_descriptor(use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, descriptor, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    Ok(handle)
}

pub fn get_daemon_control(handle: &DaemonHandle) -> &DaemonControl {
    match handle {
        DaemonHandle::Controller { control, .. } => control,
        _ => panic!("expected Controller handle in integration tests"),
    }
}

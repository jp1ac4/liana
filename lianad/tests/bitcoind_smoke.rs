mod common;

use common::*;
use lianad::miniscript::bitcoin::Network;
use lianad::DaemonHandle;
use serial_test::serial;

#[test]
#[serial]
fn daemon_starts() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::Bitcoind(&bitcoind);
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);

    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");

    assert!(wait_alive(&handle, 500));

    handle.stop().unwrap();
}

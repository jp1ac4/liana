mod common;

use common::*;
use lianad::miniscript::bitcoin::Network;
use lianad::DaemonHandle;
use serial_test::serial;

#[test]
#[serial]
fn reorg_detected() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let miner = setup_miner(&bitcoind);
    mine_blocks(&bitcoind.client, 5);

    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);
    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    // Use longer timeout for electrum backend
    let timeout = match node_kind() {
        NodeKind::Bitcoind => 10_000,
        NodeKind::Electrs => 30_000,
    };

    wait_for_height_match(&handle, &bitcoind.client, timeout);
    let initial = get_blockcount(&bitcoind.client);

    invalidate_and_remine(&bitcoind.client, initial);
    wait_for_height_match(&handle, &bitcoind.client, timeout);

    handle.stop().unwrap();
}

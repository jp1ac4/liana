mod common;

use common::*;
use lianad::commands::ListCoinsResult;
use lianad::miniscript::bitcoin::Network;
use lianad::DaemonHandle;
use serial_test::serial;

/// Helper to get a reference to the DaemonControl from the handle.
fn with_control<R>(handle: &DaemonHandle, f: impl FnOnce(&lianad::DaemonControl) -> R) -> R {
    match handle {
        DaemonHandle::Controller { control, .. } => f(control),
        DaemonHandle::Server { .. } => unreachable!("RPC server not used in tests"),
    }
}

#[test]
#[serial]
fn getnewaddress_and_listcoins_empty() {
    let bitcoind = start_bitcoind().expect("spawn bitcoind");
    let datadir = tempfile::TempDir::new().unwrap();
    let node = Node::new(node_kind(), &bitcoind).unwrap();
    let desc = test_descriptor(Network::Regtest, use_taproot());
    let cfg = lianad_config(Network::Regtest, &datadir, desc, &node);

    let handle = DaemonHandle::start_default(cfg, false).expect("start daemon");
    assert!(wait_alive(&handle, 500));

    with_control(&handle, |control| {
        let addr_res = control.get_new_address();
        assert!(addr_res.address.to_string().len() > 0);
        let ListCoinsResult { coins } = control.list_coins(&[], &[]);
        assert!(coins.is_empty(), "expected no coins at startup");
    });

    handle.stop().unwrap();
}

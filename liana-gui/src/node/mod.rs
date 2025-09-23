use lianad::config::BitcoinBackend;

pub mod bitcoind;
pub mod electrum;

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum NodeType {
    Bitcoind,
    Electrum,
}

impl From<&BitcoinBackend> for NodeType {
    fn from(bitcoin_backend: &BitcoinBackend) -> Self {
        match bitcoin_backend {
            BitcoinBackend::Bitcoind(_) => Self::Bitcoind,
            BitcoinBackend::Electrum(_) => Self::Electrum,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DefineNode<E> {
    NodeTypeSelected(NodeType),
    DefineBitcoind(bitcoind::DefineBitcoind),
    DefineElectrum(electrum::DefineElectrum),
    PingResult((NodeType, Result<(), E>)),
    Ping,
}

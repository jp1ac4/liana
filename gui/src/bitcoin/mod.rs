pub mod bitcoind;
pub mod electrum;

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum BackendType {
    Bitcoind,
    Electrum,
}

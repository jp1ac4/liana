use crate::{
    daemon::{
        model::{Coin, ListCoinsResult},
        Daemon, DaemonError,
    },
    dir::LianaDirectory,
    services::fiat::{
        api::{GetPriceResult, PriceApi, PriceApiError},
        client::PriceClient,
        Currency, PriceSource,
    },
};
use liana::miniscript::bitcoin::Network;
use lianad::commands::CoinStatus;
use std::sync::Arc;

pub const FIAT_PRICE_UPDATE_INTERVAL_SECS: u64 = 30; //FIXME

#[derive(Debug, Clone)]
pub struct Cache {
    pub datadir_path: LianaDirectory,
    pub network: Network,
    /// The `last_poll_timestamp` when starting the application.
    pub last_poll_at_startup: Option<u32>,
    pub daemon_cache: DaemonCache,
    pub fiat_price: Option<FiatPrice>,
}

/// only used for tests.
impl std::default::Default for Cache {
    fn default() -> Self {
        Self {
            datadir_path: LianaDirectory::new(std::path::PathBuf::new()),
            network: Network::Bitcoin,
            last_poll_at_startup: None,
            daemon_cache: DaemonCache::default(),
            fiat_price: None,
        }
    }
}

impl Cache {
    pub fn blockheight(&self) -> i32 {
        self.daemon_cache.blockheight
    }

    pub fn coins(&self) -> &[Coin] {
        &self.daemon_cache.coins
    }

    pub fn rescan_progress(&self) -> Option<f64> {
        self.daemon_cache.rescan_progress
    }

    pub fn sync_progress(&self) -> f64 {
        self.daemon_cache.sync_progress
    }

    pub fn last_poll_timestamp(&self) -> Option<u32> {
        self.daemon_cache.last_poll_timestamp
    }
}

/// The cache for dynamic daemon data.
#[derive(Debug, Clone)]
pub struct DaemonCache {
    pub blockheight: i32,
    pub coins: Vec<Coin>,
    pub rescan_progress: Option<f64>,
    pub sync_progress: f64,
    /// The most recent `last_poll_timestamp`.
    pub last_poll_timestamp: Option<u32>,
}

/// only used for tests.
impl std::default::Default for DaemonCache {
    fn default() -> Self {
        Self {
            blockheight: 0,
            coins: Vec::new(),
            rescan_progress: None,
            sync_progress: 1.0,
            last_poll_timestamp: None,
        }
    }
}

/// Get the coins that should be cached.
pub async fn coins_to_cache(
    daemon: Arc<dyn Daemon + Sync + Send>,
) -> Result<ListCoinsResult, DaemonError> {
    daemon
        .list_coins(&[CoinStatus::Unconfirmed, CoinStatus::Confirmed], &[])
        .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiatPrice {
    pub price_res: Result<GetPriceResult, PriceApiError>,
    pub currency: Currency,
    pub source: PriceSource,
    pub requested_at: u64,
}

pub async fn get_fiat_price(source: PriceSource, currency: Currency) -> FiatPrice {
    let client = PriceClient::default_from_source(source);
    FiatPrice {
        price_res: client.get_price(currency).await,
        currency,
        source: client.source,
        requested_at: crate::utils::now().as_secs(),
    }
}

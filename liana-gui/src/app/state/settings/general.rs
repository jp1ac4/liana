use std::collections::HashMap;
use std::sync::Arc;

use iced::Task;
use liana::miniscript::bitcoin::Network;
use liana_ui::widget::Element;

use crate::app::cache::Cache;
use crate::app::error::Error;
use crate::app::message::{FiatMessage, Message};
use crate::app::settings::fiat::PriceSetting;
use crate::app::settings::update_settings_file;
use crate::app::state::State;
use crate::app::view;
use crate::app::wallet::Wallet;
use crate::daemon::Daemon;
use crate::dir::LianaDirectory;
use crate::services::fiat::api::PriceApi;
use crate::services::fiat::client::PriceClient;
use crate::services::fiat::currency::Currency;
use crate::services::fiat::source::PriceSource;
use crate::utils::now;

/// Time to live of the list of available currencies for a given `PriceSource`.
const CURRENCIES_LIST_TTL_SECS: u64 = 3_600; // 1 hour

async fn update_price_setting(
    data_dir: LianaDirectory,
    network: Network,
    wallet: Arc<Wallet>,
    new_price_setting: PriceSetting,
) -> Result<Arc<Wallet>, Error> {
    let mut wallet = wallet.as_ref().clone();
    wallet = wallet.with_fiat_price_setting(Some(new_price_setting.clone()));
    let network_dir = data_dir.network_directory(network);
    let wallet_id = wallet.id();
    update_settings_file(&network_dir, |mut settings| {
        if let Some(wallet_setting) = settings
            .wallets
            .iter_mut()
            .find(|w| w.wallet_id() == wallet_id)
        {
            wallet_setting.fiat_price = Some(new_price_setting);
        }
        settings
    })
    .await?;
    Ok(Arc::new(wallet))
}

pub struct GeneralSettingsState {
    wallet: Arc<Wallet>,
    fiat_is_enabled: bool,
    source: PriceSource,
    currency: Option<Currency>, // there may be no currency selected yet
    currencies_list: HashMap<PriceSource, (/* timestamp */ u64, Vec<Currency>)>,
    error: Option<Error>,
}

impl From<GeneralSettingsState> for Box<dyn State> {
    fn from(s: GeneralSettingsState) -> Box<dyn State> {
        Box::new(s)
    }
}

impl GeneralSettingsState {
    pub fn new(wallet: Arc<Wallet>) -> Self {
        let price_setting = wallet.fiat_price_setting.clone();
        // If no fiat price setting, initialize as disabled with the default source but no currency.
        Self {
            wallet,
            fiat_is_enabled: price_setting
                .as_ref()
                .map(|s| s.is_enabled)
                .unwrap_or_default(),
            source: price_setting.as_ref().map(|s| s.source).unwrap_or_default(),
            currency: price_setting.map(|s| s.currency),
            currencies_list: HashMap::new(),
            error: None,
        }
    }

    fn new_price_setting(&self) -> Option<PriceSetting> {
        // We don't do any validation of the currency here.
        self.currency.map(|currency| PriceSetting {
            is_enabled: self.fiat_is_enabled,
            source: self.source,
            currency,
        })
    }
}

impl State for GeneralSettingsState {
    fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, view::Message> {
        view::settings::general::general_section(
            cache,
            self.fiat_is_enabled,
            self.source,
            self.currency,
            self.currencies_list
                .get(&self.source)
                .map(|(_, list)| &list[..])
                .unwrap_or(&[]),
            self.error.as_ref(),
        )
    }
    fn reload(
        &mut self,
        _daemon: Arc<dyn Daemon + Sync + Send>,
        _wallet: Arc<Wallet>,
    ) -> iced::Task<Message> {
        // This method is called after initialization of the state so no need to update fields.
        if self.fiat_is_enabled {
            // Update the currencies list for the source.
            let source = self.source;
            return Task::perform(async move { source }, |source| {
                Message::Fiat(FiatMessage::ListCurrencies(source))
            });
        }
        Task::none()
    }

    fn update(
        &mut self,
        _daemon: Arc<dyn Daemon + Sync + Send>,
        cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        match message {
            Message::WalletUpdated(res) => {
                match res {
                    Ok(wallet) => {
                        self.error = None;
                        // Get the fiat price if the setting is enabled and has changed.
                        // This check should be done before updating self.wallet.
                        let get_price = self.new_price_setting().is_some_and(|new| {
                            new.is_enabled && Some(new) != self.wallet.fiat_price_setting
                        });
                        self.wallet = wallet;
                        if get_price {
                            return Task::perform(async move {}, |_| {
                                Message::Fiat(FiatMessage::GetPrice)
                            });
                        }
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
                Task::none()
            }
            Message::Fiat(FiatMessage::SaveChanges) => {
                // Only save if there is a new price setting and no error.
                match (self.new_price_setting().as_ref(), self.error.as_ref()) {
                    (Some(new), None) if Some(new) != self.wallet.fiat_price_setting.as_ref() => {
                        tracing::info!(
                            "Saving fiat price setting for wallet '{}': {:?}",
                            self.wallet.id(),
                            new
                        );
                        let wallet = self.wallet.clone();
                        let price_setting = new.clone();
                        let network = cache.network;
                        let datadir_path = cache.datadir_path.clone();
                        Task::perform(
                            async move {
                                update_price_setting(datadir_path, network, wallet, price_setting)
                                    .await
                            },
                            Message::WalletUpdated,
                        )
                    }
                    _ => Task::none(),
                }
            }
            Message::Fiat(FiatMessage::ValidateCurrencySetting) => {
                if let Some(currency) = self.currency {
                    if let Some((_, list)) = self.currencies_list.get(&self.source) {
                        self.error = None;
                        // If the currently selected currency is not in the list of available currencies,
                        // set it to the default currency if eligible or otherwise the first available currency.
                        if !list.contains(&currency) {
                            if list.contains(&Currency::default()) {
                                self.currency = Some(Currency::default());
                            } else if let Some(curr) = list.first() {
                                self.currency = Some(*curr);
                            } else {
                                self.currency = None;
                                return Task::none();
                            }
                        }
                        return Task::perform(async move {}, |_| {
                            Message::Fiat(FiatMessage::SaveChanges)
                        });
                    }
                }
                Task::none()
            }
            Message::Fiat(FiatMessage::ListCurrenciesResult(source, requested_at, res)) => {
                match res {
                    Ok(list) => {
                        self.error = None;
                        // Update the currencies list only if the requested_at is newer than the existing one.
                        if !self
                            .currencies_list
                            .get(&source)
                            .is_some_and(|(old, _)| *old > requested_at)
                        {
                            tracing::debug!(
                                "Updating currencies list for source '{}' as requested at {}.",
                                source,
                                requested_at,
                            );
                            self.currencies_list
                                .insert(source, (requested_at, list.currencies));
                        }
                        return Task::perform(async move {}, |_| {
                            Message::Fiat(FiatMessage::ValidateCurrencySetting)
                        });
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
                Task::none()
            }
            Message::Fiat(FiatMessage::ListCurrencies(source)) => {
                if self.fiat_is_enabled {
                    // Update the currencies list if the cached list is stale.
                    let now = now().as_secs();
                    match self.currencies_list.get(&source) {
                        Some((old, _)) if now.saturating_sub(*old) <= CURRENCIES_LIST_TTL_SECS => {
                            return Task::perform(async move {}, |_| {
                                Message::Fiat(FiatMessage::ValidateCurrencySetting)
                            });
                        }
                        _ => {
                            return Task::perform(
                                async move {
                                    let client = PriceClient::default_from_source(source);
                                    (
                                        source,
                                        now,
                                        client.list_currencies().await.map_err(Error::FiatPrice),
                                    )
                                },
                                |(source, now, res)| {
                                    Message::Fiat(FiatMessage::ListCurrenciesResult(
                                        source, now, res,
                                    ))
                                },
                            );
                        }
                    }
                }
                Task::none()
            }
            Message::View(view::Message::Settings(view::SettingsMessage::Fiat(msg))) => {
                match msg {
                    view::FiatMessage::Enable(is_enabled) => {
                        self.fiat_is_enabled = is_enabled;
                        if self.fiat_is_enabled {
                            let source = self.source;
                            return Task::perform(async move { source }, |source| {
                                Message::Fiat(FiatMessage::ListCurrencies(source))
                            });
                        } else {
                            return Task::perform(async move {}, |_| {
                                Message::Fiat(FiatMessage::SaveChanges)
                            });
                        }
                    }
                    view::FiatMessage::SourceEdited(source) => {
                        self.source = source;
                        if self.fiat_is_enabled {
                            let source = self.source;
                            return Task::perform(async move { source }, |source| {
                                Message::Fiat(FiatMessage::ListCurrencies(source))
                            });
                        }
                    }
                    view::FiatMessage::CurrencyEdited(currency) => {
                        self.currency = Some(currency);
                        return Task::perform(async move {}, |_| {
                            Message::Fiat(FiatMessage::ValidateCurrencySetting)
                        });
                    }
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }
}

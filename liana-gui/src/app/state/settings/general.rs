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

pub struct GeneralSettingsState {
    fiat_state: FiatPriceSettingsState,
}

impl GeneralSettingsState {
    pub fn new(wallet: Arc<Wallet>) -> Self {
        Self {
            fiat_state: FiatPriceSettingsState::new(wallet),
        }
    }

    pub fn warning(&self) -> Option<&Error> {
        self.fiat_state.error.as_ref()
    }
}

impl From<GeneralSettingsState> for Box<dyn State> {
    fn from(s: GeneralSettingsState) -> Box<dyn State> {
        Box::new(s)
    }
}

impl State for GeneralSettingsState {
    fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, view::Message> {
        view::settings::general::general_section(
            cache,
            vec![self.fiat_state.view()],
            self.warning(),
        )
    }

    fn reload(
        &mut self,
        _daemon: Arc<dyn Daemon + Sync + Send>,
        wallet: Arc<Wallet>,
    ) -> iced::Task<Message> {
        Task::batch(vec![self.fiat_state.reload(wallet)])
    }

    fn update(
        &mut self,
        _daemon: Arc<dyn Daemon + Sync + Send>,
        cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        Task::batch(vec![self.fiat_state.update(cache, message)])
    }
}

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

fn wallet_price_setting_or_default(wallet: &Wallet) -> PriceSetting {
    wallet
        .fiat_price_setting
        .as_ref()
        .cloned()
        .unwrap_or_default()
}

pub struct FiatPriceSettingsState {
    wallet: Arc<Wallet>,
    new_price_setting: PriceSetting,
    currencies_list: HashMap<PriceSource, (u64, Vec<Currency>)>,
    error: Option<Error>,
}

impl FiatPriceSettingsState {
    pub fn new(wallet: Arc<Wallet>) -> Self {
        let new_price_setting = wallet_price_setting_or_default(&wallet);
        Self {
            wallet,
            new_price_setting,
            currencies_list: HashMap::new(),
            error: None,
        }
    }
}

impl FiatPriceSettingsState {
    fn view(&self) -> Element<view::Message> {
        view::settings::general::fiat_price(
            &self.new_price_setting,
            self.currencies_list
                .get(&self.new_price_setting.source)
                .map(|(_, list)| &list[..])
                .unwrap_or(&[]),
        )
    }

    fn reload(&mut self, wallet: Arc<Wallet>) -> iced::Task<Message> {
        self.new_price_setting = wallet_price_setting_or_default(&wallet);
        self.wallet = wallet.clone();
        if self.new_price_setting.is_enabled {
            let source = self.new_price_setting.source;
            return Task::perform(async move { source }, |source| {
                Message::Fiat(FiatMessage::ListCurrencies(source))
            });
        } else if self.wallet.fiat_price_setting.is_none() {
            // If the wallet does not have a fiat price setting, save the default disabled setting
            // to indicate that the user has seen the setting option (and a notification is no longer required).
            tracing::info!(
                "Fiat price setting is missing for wallet '{}'. Saving default setting.",
                self.wallet.id()
            );
            return Task::perform(async move {}, |_| Message::Fiat(FiatMessage::SaveChanges));
        }
        Task::none()
    }

    fn update(&mut self, cache: &Cache, message: Message) -> Task<Message> {
        match message {
            Message::WalletUpdated(res) => {
                match res {
                    Ok(wallet) => {
                        self.new_price_setting = wallet_price_setting_or_default(&wallet);
                        self.wallet = wallet;
                        self.error = None;
                        println!("Fiat price setting updated: {}", now().as_secs());
                        return Task::perform(async move {}, |_| {
                            Message::Fiat(FiatMessage::GetPrice)
                        });
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
                Task::none()
            }
            Message::Fiat(FiatMessage::SaveChanges) => {
                if Some(&self.new_price_setting) != self.wallet.fiat_price_setting.as_ref() {
                    let wallet = self.wallet.clone();
                    let price_setting = self.new_price_setting.clone();
                    let network = cache.network;
                    let datadir_path = cache.datadir_path.clone();
                    return Task::perform(
                        async move {
                            update_price_setting(datadir_path, network, wallet, price_setting).await
                        },
                        Message::WalletUpdated,
                    );
                }
                Task::none()
            }
            Message::Fiat(FiatMessage::ValidateCurrencySetting) => {
                if let Some((_, list)) = self.currencies_list.get(&self.new_price_setting.source) {
                    if !list.contains(&self.new_price_setting.currency) {
                        if list.contains(&Currency::default()) {
                            self.new_price_setting.currency = Currency::default();
                        } else if let Some(curr) = list.first() {
                            self.new_price_setting.currency = *curr;
                        } else {
                            return Task::none();
                        }
                    }
                    return Task::perform(async move {}, |_| {
                        Message::Fiat(FiatMessage::SaveChanges)
                    });
                }
                Task::none()
            }
            Message::Fiat(FiatMessage::ListCurrenciesResult(source, requested_at, res)) => {
                match res {
                    Ok(list) => {
                        if !self
                            .currencies_list
                            .get(&source)
                            .is_some_and(|(old, _)| *old > requested_at)
                        {
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
                if self.new_price_setting.is_enabled {
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
                        self.new_price_setting.is_enabled = is_enabled;
                        if self.new_price_setting.is_enabled {
                            let source = self.new_price_setting.source;
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
                        self.new_price_setting.source = source;
                        if self.new_price_setting.is_enabled {
                            let source = self.new_price_setting.source;
                            return Task::perform(async move { source }, |source| {
                                Message::Fiat(FiatMessage::ListCurrencies(source))
                            });
                        }
                    }
                    view::FiatMessage::CurrencyEdited(currency) => {
                        self.new_price_setting.currency = currency;
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

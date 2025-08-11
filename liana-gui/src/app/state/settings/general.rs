use std::{collections::HashMap, sync::Arc};

use iced::Task;
use liana::miniscript::bitcoin::Network;

use crate::app::cache::Cache;
use crate::app::error::Error;
use crate::app::message::{FiatMessage, Message};
use crate::app::settings::{fiat::PriceSetting, update_settings_file};
use crate::app::state::State;
use crate::app::view;
use crate::app::wallet::Wallet;
use crate::daemon::Daemon;
use crate::dir::LianaDirectory;
use crate::fiat::api::PriceApi;
use crate::fiat::{Currency, PriceClient, PriceSource, ALL_PRICE_SOURCES};
use crate::utils::now;

fn price_setting_from_wallet(wallet: &Wallet) -> PriceSetting {
    wallet
        .fiat_price_setting
        .as_ref()
        .cloned()
        .unwrap_or_default()
}

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

pub struct FiatPriceSettingsState {
    wallet: Arc<Wallet>,
    new_price_setting: PriceSetting,
    // is_enabled: bool,
    // source: PriceSource,
    // currency: Currency,
    currencies_list: HashMap<PriceSource, (u64, Vec<Currency>)>,
    // prices_cache: HashMap<PriceSource, HashMap<Currency, u64>>,
    error: Option<Error>,
}

impl FiatPriceSettingsState {
    pub fn new(wallet: Arc<Wallet>) -> Self {
        // let (is_enabled, source, currency) = if let Some(price_setting) = &wallet.fiat_price_setting
        // {
        //     (
        //         price_setting.is_enabled,
        //         price_setting.source,
        //         price_setting.currency,
        //     )
        // } else {
        //     (false, PriceSource::default(), Currency::default())
        // };
        // assert!(
        //     ALL_PRICE_SOURCES.contains(&source),
        //     "Source {} is not in the list of available sources",
        //     source,
        // );
        let new_price_setting = price_setting_from_wallet(&wallet);
        let currencies_list = ALL_PRICE_SOURCES
            .iter()
            .map(|s| {
                (
                    *s,
                    (
                        0,
                        if s == &new_price_setting.source {
                            vec![new_price_setting.currency]
                        } else {
                            vec![]
                        },
                    ),
                )
            })
            .collect();
        // let prices_cache = ALL_PRICE_SOURCES
        //     .iter()
        //     .map(|s| (*s, HashMap::new()))
        //     .collect();
        FiatPriceSettingsState {
            wallet,
            // is_enabled,
            // source,
            // currency,
            new_price_setting,
            currencies_list,

            // prices_cache,
            error: None,
        }
    }
}

impl State for FiatPriceSettingsState {
    fn view<'a>(&'a self, _cache: &'a Cache) -> liana_ui::widget::Element<'a, view::Message> {
        todo!()
    }

    fn reload(
        &mut self,
        _daemon: Arc<dyn Daemon + Sync + Send>,
        wallet: Arc<Wallet>,
    ) -> iced::Task<Message> {
        self.new_price_setting = price_setting_from_wallet(&wallet);
        self.wallet = wallet.clone();
        if self.new_price_setting.is_enabled {
            let source = self.new_price_setting.source;
            return Task::perform(async move { source }, |source| {
                Message::Fiat(FiatMessage::UpdateCurrencies(source))
            });
        } else if self.wallet.fiat_price_setting.is_none() {
            // If the wallet does not have a fiat price setting, save the default disabled setting
            // to indicate that the user has seen the setting option (and a notification is no longer required).
            return Task::perform(async move {}, |_| Message::Fiat(FiatMessage::SaveChanges));
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
                        self.wallet = wallet;
                        self.new_price_setting = price_setting_from_wallet(&self.wallet);
                        self.error = None;
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
            Message::Fiat(FiatMessage::ListCurrenciesResult(source, requested_at, res)) => {
                match res {
                    Ok(list) => {
                        if !self
                            .currencies_list
                            .get(&source)
                            .is_some_and(|(old, _)| *old > requested_at)
                        {
                            if !list.currencies.contains(&self.new_price_setting.currency) {
                                if let Some(curr) = list.currencies.first() {
                                    self.new_price_setting.currency = *curr;
                                }
                            }
                            self.currencies_list
                                .insert(source, (requested_at, list.currencies));
                        }
                        return Task::perform(async move {}, |_| {
                            Message::Fiat(FiatMessage::SaveChanges)
                        });

                        // TODO: update settings file & wallet with new price setting.

                        // if self.new_price_setting.is_enabled {
                        //     let now = now().as_secs();
                        //     if !self
                        //         .prices_cache
                        //         .get(&source)
                        //         .and_then(|curr_map| curr_map.get(&self.new_price_setting.currency))
                        //         .is_some_and(|timestamp| now.saturating_sub(*timestamp) < 100)
                        //     {
                        //         let source = self.new_price_setting.source;
                        //         let currency = self.new_price_setting.currency;
                        //         return Task::perform(
                        //             async move { (source, currency) },
                        //             |(source, currency)| {
                        //                 Message::Fiat(FiatMessage::GetPrice(source, currency))
                        //             },
                        //         );
                        //     }
                        // }
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
                Task::none()
            }
            // Message::Fiat(FiatMessage::GetPrice(source, currency)) => {
            //     if self.new_price_setting.is_enabled {
            //         return Task::perform(
            //             async move { cache::get_fiat_price(source, currency).await },
            //             move |res| {
            //                 Message::Fiat(FiatMessage::GetPriceResult(res))
            //             },
            //         );
            //     }
            //     Task::none()
            // }
            Message::Fiat(FiatMessage::UpdateCurrencies(source)) => {
                if self.new_price_setting.is_enabled {
                    // Do not get currencies list if it has already been set for this source recently.
                    const CURRENCIES_LIST_TTL_SECS: u64 = 3_600; // 1 hour
                    let now = now().as_secs();
                    if self
                        .currencies_list
                        .get(&source)
                        .is_some_and(|(timestamp, _)| {
                            now.saturating_sub(*timestamp) > CURRENCIES_LIST_TTL_SECS
                        })
                    {
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
                                Message::Fiat(FiatMessage::ListCurrenciesResult(source, now, res))
                            },
                        );
                    }
                }
                Task::none()
            }
            Message::View(view::Message::Settings(view::SettingsMessage::Fiat(msg))) => {
                match msg {
                    view::FiatMessage::Enable(is_enabled) => {
                        self.new_price_setting.is_enabled = is_enabled;
                        if self.new_price_setting.is_enabled {
                            return Task::perform(
                                async move { PriceSource::default() },
                                |source| Message::Fiat(FiatMessage::UpdateCurrencies(source)),
                            );
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
                                Message::Fiat(FiatMessage::UpdateCurrencies(source))
                            });
                        }
                    }
                    view::FiatMessage::CurrencyEdited(currency) => {
                        self.new_price_setting.currency = currency;
                        return Task::perform(async move {}, |_| {
                            Message::Fiat(FiatMessage::SaveChanges)
                        });
                    }
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }
}

impl From<FiatPriceSettingsState> for Box<dyn State> {
    fn from(s: FiatPriceSettingsState) -> Box<dyn State> {
        Box::new(s)
    }
}

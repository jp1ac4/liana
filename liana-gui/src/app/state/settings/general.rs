use std::{collections::HashMap, sync::Arc};

// use iced::{Subscription, Task};

// use liana::{
//     descriptors::LianaDescriptor,
//     miniscript::bitcoin::{bip32::Fingerprint, Network},
// };

// use liana_ui::{
//     component::{form, modal},
//     widget::Element,
// };

use iced::Task;

use crate::{
    app::{
        cache::Cache,
        error::Error,
        message::{FiatMessage, Message},
        settings::{self, fiat::PriceSetting, update_settings_file},
        state::{export::ExportModal, State},
        view,
        wallet::Wallet,
        Config,
    },
    daemon::{Daemon, DaemonBackend},
    dir::LianaDirectory,
    export::{ImportExportMessage, ImportExportType},
    fiat::{
        api::PriceApi,
        source::{self, ALL_PRICE_SOURCES},
        Currency, PriceClient, PriceSource,
    },
    hw::{HardwareWallet, HardwareWalletConfig, HardwareWallets},
    services::connect::client::backend::api::WALLET_ALIAS_MAXIMUM_LENGTH,
    utils::now,
};

pub struct FiatPriceSettingsState {
    wallet: Arc<Wallet>,
    new_price_setting: PriceSetting,
    // is_enabled: bool,
    // source: PriceSource,
    // currency: Currency,
    currencies_list: HashMap<PriceSource, (u64, Vec<Currency>)>,
    prices_cache: HashMap<PriceSource, HashMap<Currency, u64>>,
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
        let new_price_setting = wallet
            .fiat_price_setting
            .as_ref()
            .cloned()
            .unwrap_or_default();
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
        let prices_cache = ALL_PRICE_SOURCES
            .iter()
            .map(|s| (*s, HashMap::new()))
            .collect();
        FiatPriceSettingsState {
            wallet,
            // is_enabled,
            // source,
            // currency,
            new_price_setting,
            currencies_list,

            prices_cache,
            error: None,
        }
    }
}

impl State for FiatPriceSettingsState {
    fn view<'a>(&'a self, cache: &'a Cache) -> liana_ui::widget::Element<'a, view::Message> {
        todo!()
    }

    fn reload(
        &mut self,
        _daemon: Arc<dyn Daemon + Sync + Send>,
        wallet: Arc<Wallet>,
    ) -> iced::Task<Message> {
        let mut new = FiatPriceSettingsState::new(wallet);
        new.currencies_list = self.currencies_list.clone();
        *self = new;
        if self.new_price_setting.is_enabled {
            let source = self.new_price_setting.source;
            return Task::perform(async move { source }, |source| {
                Message::Fiat(FiatMessage::UpdateCurrencies(source))
            });
        }
        Task::none()
    }

    fn update(
        &mut self,
        _daemon: Arc<dyn Daemon + Sync + Send>,
        _cache: &Cache,
        _message: Message,
    ) -> Task<Message> {
        match _message {
            Message::Fiat(FiatMessage::ListCurrenciesResult(source, requested_at, res)) => {
                match res {
                    Ok(list) => {
                        if self
                            .currencies_list
                            .get(&source)
                            .is_some_and(|(timestamp, _)| *timestamp < requested_at)
                        {
                            if !list.currencies.contains(&self.new_price_setting.currency) {
                                if let Some(curr) = list.currencies.first() {
                                    self.new_price_setting.currency = *curr;
                                }
                            }
                            self.currencies_list
                                .insert(source, (requested_at, list.currencies));
                        }
                        if self.new_price_setting.is_enabled {
                            return Task::perform(async move {}, |_| {
                                Message::Fiat(FiatMessage::PriceTick)
                            });
                        }
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
                Task::none()
            }
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
                        // TODO: need to make sure wallet has been updated before tick...
                        // and need to update settings file.
                        if self.new_price_setting.is_enabled {
                            return Task::perform(async move {}, |_| {
                                Message::Fiat(FiatMessage::PriceTick)
                            });
                        }
                    }
                }
                Task::none()
            }
            // Message::Fiat(FiatMessage::UpdatePrice(source, currency, res)) => {
            //     if let Ok(price) = res {
            //         self.source = source;
            //         self.currency = currency;
            //         self.currencies_list = price.currencies;
            //     } else {
            //         self.error = Some(Error::FiatPrice(res.unwrap_err()));
            //     }
            // }
            // Message::View(view::Message::Settings(view::SettingsMessage::FiatPriceEnabled(
            //     is_enabled,
            // ))) => {
            //     self.is_enabled = is_enabled;
            // }
            // Message::View(view::Message::Settings(view::SettingsMessage::FiatPriceSource(
            //     source,
            // ))) => {
            //     self.source = source;
            //     self.currencies_list.clear();
            //     self.currencies_list.push(Currency::default());
            // }
            // Message::View(view::Message::Settings(view::SettingsMessage::FiatPriceCurrency(
            //     currency,
            // ))) => {
            //     self.currency = currency;
            // }
            _ => Task::none(),
        }
    }
}

// impl State for FiatPriceSettingsState {
//     fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, view::Message> {
//         let content = view::settings::general: wallet_settings(
//             cache,
//             self.warning.as_ref(),
//             &self.descriptor,
//             &self.wallet_alias,
//             &self.keys_aliases,
//             &self.wallet.provider_keys,
//             self.processing,
//             self.updated,
//         );

//         match &self.modal {
//             Modal::None => content,
//             Modal::RegisterWallet(m) => modal::Modal::new(content, m.view())
//                 .on_blur(Some(view::Message::Close))
//                 .into(),
//             Modal::ImportExport(m) => m.view(content),
//         }
//     }

//     fn subscription(&self) -> Subscription<Message> {
//         match &self.modal {
//             Modal::None => Subscription::none(),
//             Modal::RegisterWallet(modal) => modal.subscription(),
//             Modal::ImportExport(modal) => {
//                 if let Some(sub) = modal.subscription() {
//                     sub.map(|m| {
//                         Message::View(view::Message::Settings(
//                             view::SettingsMessage::ImportExport(ImportExportMessage::Progress(m)),
//                         ))
//                     })
//                 } else {
//                     Subscription::none()
//                 }
//             }
//         }
//     }

//     fn update(
//         &mut self,
//         daemon: Arc<dyn Daemon + Sync + Send>,
//         cache: &Cache,
//         message: Message,
//     ) -> Task<Message> {
//         match message {
//             Message::WalletUpdated(res) => {
//                 self.processing = false;
//                 if let Modal::RegisterWallet(modal) = &mut self.modal {
//                     modal.update(daemon, cache, Message::WalletUpdated(res))
//                 } else {
//                     match res {
//                         Ok(wallet) => {
//                             self.keys_aliases = Self::keys_aliases(&wallet);
//                             self.wallet = wallet;
//                             self.updated = true;
//                         }
//                         Err(e) => self.warning = Some(e),
//                     };
//                     Task::none()
//                 }
//             }
//             Message::View(view::Message::Settings(view::SettingsMessage::WalletAliasEdited(
//                 alias,
//             ))) => {
//                 self.wallet_alias.valid = alias.len() < WALLET_ALIAS_MAXIMUM_LENGTH;
//                 self.wallet_alias.value = alias;
//                 Task::none()
//             }
//             Message::View(view::Message::Settings(
//                 view::SettingsMessage::FingerprintAliasEdited(fg, value),
//             )) => {
//                 if let Some((_, name)) = self
//                     .keys_aliases
//                     .iter_mut()
//                     .find(|(fingerprint, _)| fg == *fingerprint)
//                 {
//                     name.value = value;
//                 }
//                 Task::none()
//             }
//             Message::View(view::Message::Settings(view::SettingsMessage::Save)) => {
//                 self.modal = Modal::None;
//                 self.processing = true;
//                 self.updated = false;
//                 Task::perform(
//                     update_aliases(
//                         self.data_dir.clone(),
//                         cache.network,
//                         self.wallet.clone(),
//                         match self
//                             .wallet
//                             .alias
//                             .as_ref()
//                             .map(|a| *a == self.wallet_alias.value)
//                         {
//                             Some(true) => None,
//                             Some(false) => Some(self.wallet_alias.value.clone()),
//                             None => {
//                                 if self.wallet_alias.value.is_empty() {
//                                     None
//                                 } else {
//                                     Some(self.wallet_alias.value.clone())
//                                 }
//                             }
//                         },
//                         self.keys_aliases
//                             .iter()
//                             .map(|(fg, name)| (*fg, name.value.to_owned()))
//                             .collect(),
//                         daemon,
//                     ),
//                     Message::WalletUpdated,
//                 )
//             }
//             Message::View(view::Message::Close) => {
//                 self.modal = Modal::None;
//                 Task::none()
//             }
//             Message::View(view::Message::Settings(view::SettingsMessage::RegisterWallet)) => {
//                 self.modal = Modal::RegisterWallet(RegisterWalletModal::new(
//                     self.data_dir.clone(),
//                     self.wallet.clone(),
//                     cache.network,
//                 ));
//                 Task::none()
//             }

//             Message::View(view::Message::ImportExport(ImportExportMessage::UpdateAliases(
//                 aliases,
//             ))) => {
//                 self.processing = true;
//                 self.updated = false;
//                 Task::perform(
//                     update_aliases(
//                         self.data_dir.clone(),
//                         cache.network,
//                         self.wallet.clone(),
//                         None,
//                         aliases.into_iter().map(|(fg, ks)| (fg, ks.name)).collect(),
//                         daemon,
//                     ),
//                     Message::WalletUpdated,
//                 )
//             }
//             Message::View(view::Message::ImportExport(ImportExportMessage::Close)) => {
//                 if let Modal::ImportExport(_) = &self.modal {
//                     self.modal = Modal::None;
//                 }
//                 Task::none()
//             }
//             Message::View(view::Message::ImportExport(m)) => {
//                 if let Modal::ImportExport(modal) = &mut self.modal {
//                     modal.update(m)
//                 } else {
//                     Task::none()
//                 }
//             }
//             Message::View(view::Message::Settings(view::SettingsMessage::ImportExport(m))) => {
//                 if let Modal::ImportExport(modal) = &mut self.modal {
//                     modal.update(m)
//                 } else {
//                     Task::none()
//                 }
//             }
//             Message::View(view::Message::Settings(view::SettingsMessage::ExportWallet)) => {
//                 if self.modal.is_none() {
//                     let datadir = cache.datadir_path.clone();
//                     let network = cache.network;
//                     let config = self.config.clone();
//                     let wallet = self.wallet.clone();
//                     let daemon = daemon.clone();
//                     let modal = ExportModal::new(
//                         Some(daemon),
//                         ImportExportType::ExportProcessBackup(datadir, network, config, wallet),
//                     );
//                     let launch = modal.launch(true);
//                     self.modal = Modal::ImportExport(modal);
//                     launch
//                 } else {
//                     Task::none()
//                 }
//             }
//             Message::View(view::Message::Settings(view::SettingsMessage::ImportWallet)) => {
//                 if self.modal.is_none() {
//                     let modal = ExportModal::new(
//                         Some(daemon),
//                         ImportExportType::ImportBackup {
//                             network_dir: cache.datadir_path.network_directory(cache.network),
//                             wallet: self.wallet.clone(),
//                             overwrite_labels: None,
//                             overwrite_aliases: None,
//                         },
//                     );
//                     let launch = modal.launch(false);
//                     self.modal = Modal::ImportExport(modal);
//                     launch
//                 } else {
//                     Task::none()
//                 }
//             }
//             _ => match &mut self.modal {
//                 Modal::RegisterWallet(m) => m.update(daemon, cache, message),
//                 _ => Task::none(),
//             },
//         }
//     }

//     fn reload(
//         &mut self,
//         daemon: Arc<dyn Daemon + Sync + Send>,
//         wallet: Arc<Wallet>,
//     ) -> Task<Message> {
//         self.descriptor = wallet.main_descriptor.clone();
//         self.keys_aliases = Self::keys_aliases(&wallet);
//         self.wallet = wallet;
//         Task::perform(
//             async move { daemon.get_info().await.map_err(|e| e.into()) },
//             Message::Info,
//         )
//     }
// }

// impl From<WalletSettingsState> for Box<dyn State> {
//     fn from(s: WalletSettingsState) -> Box<dyn State> {
//         Box::new(s)
//     }
// }

use std::convert::{From, TryInto};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{NaiveDate, Utc};
use iced::{clipboard, Task};
use tracing::info;

use liana::miniscript::bitcoin::Network;
use lianad::config::{BitcoinBackend, BitcoindConfig, BitcoindRpcAuth, Config, ElectrumConfig};

use liana_ui::{component::form, widget::Element};

use crate::{
    app::{
        cache::Cache,
        error::Error,
        message::Message,
        state::settings::State,
        view::{self, DefineNode, SettingsEditMessage},
    },
    daemon::Daemon,
    help,
    node::{
        bitcoind::{self, DefineBitcoind, RpcAuthType, RpcAuthValues},
        electrum::{self, DefineElectrum},
    },
};

#[derive(Debug)]
pub struct NodeSettingsState {
    warning: Option<Error>,
    config_updated: bool,

    can_edit_node_settings: bool,
    editing_node_settings: bool,
    backend_config: Option<BitcoinBackend>,
    bitcoind_settings: Option<BitcoindSettings>,
    electrum_settings: Option<ElectrumSettings>,
    rescan_settings: RescanSetting,
}

impl NodeSettingsState {
    pub fn new(config: Option<Config>, cache: &Cache, bitcoind_is_internal: bool) -> Self {
        let backend_config = config.clone().and_then(|c| c.bitcoin_backend);
        let (bitcoind_config, electrum_config) = match backend_config.as_ref() {
            Some(BitcoinBackend::Bitcoind(bitcoind_config)) => {
                (Some(bitcoind_config.clone()), None)
            }
            Some(BitcoinBackend::Electrum(electrum_config)) => {
                (None, Some(electrum_config.clone()))
            }
            _ => (None, None),
        };
        NodeSettingsState {
            warning: None,
            config_updated: false,
            can_edit_node_settings: backend_config.is_some() && !bitcoind_is_internal,
            editing_node_settings: false,
            backend_config,
            bitcoind_settings: bitcoind_config.map(BitcoindSettings::new),
            electrum_settings: electrum_config.map(ElectrumSettings::new),
            rescan_settings: RescanSetting::new(cache.rescan_progress()),
        }
    }

    fn any_node_processing(&self) -> bool {
        self.bitcoind_settings
            .as_ref()
            .map(|s| s.processing)
            .unwrap_or_default()
            || self
                .electrum_settings
                .as_ref()
                .map(|s| s.processing)
                .unwrap_or_default()
    }
}

impl State for NodeSettingsState {
    fn update(
        &mut self,
        daemon: Arc<dyn Daemon + Sync + Send>,
        cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        match message {
            Message::DaemonConfigLoaded(res) => match res {
                Ok(()) => {
                    self.config_updated = true;
                    self.warning = None;
                    if let Some(settings) = &mut self.bitcoind_settings {
                        settings.edited();
                        return Task::perform(async {}, |_| {
                            Message::View(view::Message::Settings(
                                view::SettingsMessage::EditNodeSettings,
                            ))
                        });
                    }
                    if let Some(settings) = &mut self.electrum_settings {
                        settings.edited();
                        return Task::perform(async {}, |_| {
                            Message::View(view::Message::Settings(
                                view::SettingsMessage::EditNodeSettings,
                            ))
                        });
                    }
                }
                Err(e) => {
                    self.config_updated = false;
                    self.warning = Some(e);
                    if let Some(settings) = &mut self.bitcoind_settings {
                        settings.edited();
                    }
                    if let Some(settings) = &mut self.electrum_settings {
                        settings.edited();
                    }
                }
            },
            Message::Info(res) => match res {
                Err(e) => self.warning = Some(e),
                Ok(info) => {
                    if info.rescan_progress == Some(1.0) {
                        self.rescan_settings.edited(true);
                    }
                }
            },
            Message::StartRescan(Err(_)) => {
                self.rescan_settings.past_possible_height = true;
                self.rescan_settings.processing = false;
            }
            Message::UpdatePanelCache(_) => {
                self.rescan_settings.processing = cache.rescan_progress().is_some_and(|p| p < 1.0);
            }
            Message::View(view::Message::Settings(view::SettingsMessage::NodeSettings(msg))) => {
                match msg {
                    SettingsEditMessage::Select => {
                        if !self.any_node_processing() {
                            self.editing_node_settings = true;
                        }
                    }
                    SettingsEditMessage::Cancel => {
                        if !self.any_node_processing() {
                            self.editing_node_settings = false;
                        }
                    }
                    _ => {
                        if let Some(settings) = &mut self.bitcoind_settings {
                            return settings.update(daemon, cache, msg);
                        }
                        if let Some(settings) = &mut self.electrum_settings {
                            return settings.update(daemon, cache, msg);
                        }
                    }
                }
            }
            Message::View(view::Message::Settings(view::SettingsMessage::RescanSettings(msg))) => {
                return self.rescan_settings.update(daemon, cache, msg);
            }
            _ => {}
        };
        Task::none()
    }

    fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, view::Message> {
        let can_edit = self.can_edit_node_settings && !self.rescan_settings.processing;
        let can_do_rescan = !self.rescan_settings.processing && !self.editing_node_settings;
        let mut setting_panels = Vec::new();
        if let Some(backend_config) = &self.backend_config {
            if !self.editing_node_settings {
                setting_panels.push(
                    view::settings::node(
                        cache.network,
                        backend_config,
                        cache.blockheight(),
                        Some(cache.blockheight() != 0),
                        can_edit,
                    )
                    .map(move |msg| {
                        view::Message::Settings(view::SettingsMessage::NodeSettings(msg))
                    }),
                );
            } else {
                if let Some(settings) = self.bitcoind_settings.as_ref() {
                    setting_panels.push(settings.view(cache).map(move |msg| {
                        view::Message::Settings(view::SettingsMessage::NodeSettings(msg))
                    }))
                }
                if let Some(settings) = self.electrum_settings.as_ref() {
                    setting_panels.push(settings.view(cache).map(move |msg| {
                        view::Message::Settings(view::SettingsMessage::NodeSettings(msg))
                    }))
                }
            }
            setting_panels.push(view::settings::link(
                help::CHANGE_BACKEND_OR_NODE_URL,
                "I want to change node type or use Liana Connect",
            ));
        }
        setting_panels.push(self.rescan_settings.view(cache, can_do_rescan).map(
            move |msg: view::SettingsEditMessage| {
                view::Message::Settings(view::SettingsMessage::RescanSettings(msg))
            },
        ));
        view::settings::node_settings(cache, self.warning.as_ref(), setting_panels)
    }
}

impl From<NodeSettingsState> for Box<dyn State> {
    fn from(s: NodeSettingsState) -> Box<dyn State> {
        Box::new(s)
    }
}

#[derive(Debug)]
pub struct BitcoindSettings {
    processing: bool,
    rpc_auth_vals: RpcAuthValues,
    selected_auth_type: RpcAuthType,
    addr: form::Value<String>,
}

impl BitcoindSettings {
    fn new(bitcoind_config: BitcoindConfig) -> BitcoindSettings {
        let (rpc_auth_vals, selected_auth_type) = match &bitcoind_config.rpc_auth {
            BitcoindRpcAuth::CookieFile(path) => (
                RpcAuthValues {
                    cookie_path: form::Value {
                        valid: true,
                        warning: None,
                        value: path.to_str().unwrap().to_string(),
                    },
                    user: form::Value::default(),
                    password: form::Value::default(),
                },
                RpcAuthType::CookieFile,
            ),
            BitcoindRpcAuth::UserPass(user, password) => (
                RpcAuthValues {
                    cookie_path: form::Value::default(),
                    user: form::Value {
                        valid: true,
                        warning: None,
                        value: user.clone(),
                    },
                    password: form::Value {
                        valid: true,
                        warning: None,
                        value: password.clone(),
                    },
                },
                RpcAuthType::UserPass,
            ),
        };
        let addr = bitcoind_config.addr.to_string();
        BitcoindSettings {
            processing: false,
            rpc_auth_vals,
            selected_auth_type,
            addr: form::Value {
                valid: true,
                warning: None,
                value: addr,
            },
        }
    }
}

impl BitcoindSettings {
    fn edited(&mut self) {
        self.processing = false;
    }

    fn update(
        &mut self,
        daemon: Arc<dyn Daemon + Sync + Send>,
        _cache: &Cache,
        message: view::SettingsEditMessage,
    ) -> Task<Message> {
        match message {
            view::SettingsEditMessage::Node(DefineNode::DefineBitcoind(msg)) => {
                if !self.processing {
                    match msg {
                        DefineBitcoind::ConfigFieldEdited(field, value) => match field {
                            bitcoind::ConfigField::Address => {
                                self.addr.value = value;
                            }
                            bitcoind::ConfigField::CookieFilePath => {
                                self.rpc_auth_vals.cookie_path.value = value;
                            }
                            bitcoind::ConfigField::User => {
                                self.rpc_auth_vals.user.value = value;
                            }
                            bitcoind::ConfigField::Password => {
                                self.rpc_auth_vals.password.value = value;
                            }
                        },
                        DefineBitcoind::RpcAuthTypeSelected(auth_type) => {
                            self.selected_auth_type = auth_type;
                        }
                    }
                }
            }
            view::SettingsEditMessage::Confirm => {
                let new_addr = SocketAddr::from_str(&self.addr.value);
                self.addr.valid = new_addr.is_ok();
                let rpc_auth = match self.selected_auth_type {
                    RpcAuthType::CookieFile => {
                        let new_path = PathBuf::from_str(&self.rpc_auth_vals.cookie_path.value);
                        match new_path {
                            Ok(path) => {
                                self.rpc_auth_vals.cookie_path.valid = true;
                                Some(BitcoindRpcAuth::CookieFile(path))
                            }
                            Err(_) => None,
                        }
                    }
                    RpcAuthType::UserPass => Some(BitcoindRpcAuth::UserPass(
                        self.rpc_auth_vals.user.value.clone(),
                        self.rpc_auth_vals.password.value.clone(),
                    )),
                };

                if let (true, Some(rpc_auth)) = (self.addr.valid, rpc_auth) {
                    let mut daemon_config = daemon.config().cloned().unwrap();
                    daemon_config.bitcoin_backend =
                        Some(lianad::config::BitcoinBackend::Bitcoind(BitcoindConfig {
                            rpc_auth,
                            addr: new_addr.unwrap(),
                        }));
                    self.processing = true;
                    return Task::perform(async move { daemon_config }, |cfg| {
                        Message::LoadDaemonConfig(Box::new(cfg))
                    });
                }
            }
            view::SettingsEditMessage::Clipboard(text) => return clipboard::write(text),
            _ => {}
        };
        Task::none()
    }

    fn view<'a>(&self, cache: &'a Cache) -> Element<'a, view::SettingsEditMessage> {
        view::settings::bitcoind_edit(
            cache.network,
            cache.blockheight(),
            &self.addr,
            &self.rpc_auth_vals,
            &self.selected_auth_type,
            self.processing,
        )
    }
}

#[derive(Debug)]
pub struct ElectrumSettings {
    processing: bool,
    addr: form::Value<String>,
    validate_domain: bool,
}

impl ElectrumSettings {
    fn new(electrum_config: ElectrumConfig) -> ElectrumSettings {
        let addr = electrum_config.addr.to_string();
        ElectrumSettings {
            processing: false,
            addr: form::Value {
                valid: true,
                warning: None,
                value: addr,
            },
            validate_domain: electrum_config.validate_domain,
        }
    }
}

impl ElectrumSettings {
    fn edited(&mut self) {
        self.processing = false;
    }

    fn update(
        &mut self,
        daemon: Arc<dyn Daemon + Sync + Send>,
        _cache: &Cache,
        message: view::SettingsEditMessage,
    ) -> Task<Message> {
        match message {
            view::SettingsEditMessage::Node(DefineNode::DefineElectrum(msg)) => {
                if !self.processing {
                    match msg {
                        DefineElectrum::ConfigFieldEdited(field, value) => match field {
                            electrum::ConfigField::Address => {
                                self.addr.valid = electrum::is_electrum_address_valid(&value);
                                self.addr.value = value;
                            }
                        },
                        DefineElectrum::ValidDomainChanged(b) => {
                            self.validate_domain = b;
                        }
                    }
                }
            }
            view::SettingsEditMessage::Confirm => {
                if self.addr.valid {
                    let mut daemon_config = daemon.config().cloned().unwrap();
                    daemon_config.bitcoin_backend =
                        Some(lianad::config::BitcoinBackend::Electrum(ElectrumConfig {
                            addr: self.addr.value.clone(),
                            validate_domain: self.validate_domain,
                        }));
                    self.processing = true;
                    return Task::perform(async move { daemon_config }, |cfg| {
                        Message::LoadDaemonConfig(Box::new(cfg))
                    });
                }
            }
            view::SettingsEditMessage::Clipboard(text) => return clipboard::write(text),
            _ => {}
        };
        Task::none()
    }

    fn view<'a>(&self, cache: &'a Cache) -> Element<'a, view::SettingsEditMessage> {
        view::settings::electrum_edit(
            cache.network,
            cache.blockheight(),
            &self.addr,
            self.processing,
            self.validate_domain,
        )
    }
}

#[derive(Debug, Default)]
pub struct RescanSetting {
    processing: bool,
    success: bool,
    year: form::Value<String>,
    month: form::Value<String>,
    day: form::Value<String>,
    invalid_date: bool,
    future_date: bool,
    past_possible_height: bool,
}

impl RescanSetting {
    pub fn new(rescan_progress: Option<f64>) -> Self {
        Self {
            processing: if let Some(progress) = rescan_progress {
                progress < 1.0
            } else {
                false
            },
            ..Default::default()
        }
    }
}

impl RescanSetting {
    fn edited(&mut self, success: bool) {
        self.processing = false;
        self.success = success;
    }

    fn update(
        &mut self,
        daemon: Arc<dyn Daemon + Sync + Send>,
        cache: &Cache,
        message: view::SettingsEditMessage,
    ) -> Task<Message> {
        match message {
            view::SettingsEditMessage::FieldEdited(field, value) => {
                self.invalid_date = false;
                self.future_date = false;
                self.past_possible_height = false;
                if !self.processing && (value.is_empty() || u32::from_str(&value).is_ok()) {
                    match field {
                        "rescan_year" => self.year.value = value,
                        "rescan_month" => self.month.value = value,
                        "rescan_day" => self.day.value = value,
                        _ => {}
                    }
                }
            }
            view::SettingsEditMessage::Confirm => {
                let t = if let Some(date) = NaiveDate::from_ymd_opt(
                    i32::from_str(&self.year.value).unwrap_or(1),
                    u32::from_str(&self.month.value).unwrap_or(1),
                    u32::from_str(&self.day.value).unwrap_or(1),
                )
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|d| d.and_utc().timestamp())
                {
                    match cache.network {
                        Network::Bitcoin => {
                            if date < MAINNET_GENESIS_BLOCK_TIMESTAMP {
                                info!("Date {} prior to genesis block, using genesis block timestamp {}", date, MAINNET_GENESIS_BLOCK_TIMESTAMP);

                                MAINNET_GENESIS_BLOCK_TIMESTAMP
                            } else {
                                date
                            }
                        }
                        Network::Testnet => {
                            if date < TESTNET3_GENESIS_BLOCK_TIMESTAMP {
                                info!("Date {} prior to genesis block, using genesis block timestamp {}", date, TESTNET3_GENESIS_BLOCK_TIMESTAMP);
                                TESTNET3_GENESIS_BLOCK_TIMESTAMP
                            } else {
                                date
                            }
                        }
                        Network::Testnet4 => {
                            if date < TESTNET4_GENESIS_BLOCK_TIMESTAMP {
                                info!("Date {} prior to genesis block, using genesis block timestamp {}", date, TESTNET4_GENESIS_BLOCK_TIMESTAMP);
                                TESTNET4_GENESIS_BLOCK_TIMESTAMP
                            } else {
                                date
                            }
                        }
                        Network::Signet => {
                            if date < SIGNET_GENESIS_BLOCK_TIMESTAMP {
                                info!("Date {} prior to genesis block, using genesis block timestamp {}", date, SIGNET_GENESIS_BLOCK_TIMESTAMP);
                                SIGNET_GENESIS_BLOCK_TIMESTAMP
                            } else {
                                date
                            }
                        }
                        // We expect regtest user to not use genesis block timestamp inferior to
                        // the mainnet one.
                        // Network is a non exhaustive enum, that is why the _.
                        _ => {
                            if date < MAINNET_GENESIS_BLOCK_TIMESTAMP {
                                info!("Date {} prior to genesis block, using genesis block timestamp {}", date, MAINNET_GENESIS_BLOCK_TIMESTAMP);
                                MAINNET_GENESIS_BLOCK_TIMESTAMP
                            } else {
                                date
                            }
                        }
                    }
                } else {
                    self.invalid_date = true;
                    return Task::none();
                };
                if t > Utc::now().timestamp() {
                    self.future_date = true;
                    return Task::none();
                }
                self.processing = true;
                info!("Asking daemon to rescan with timestamp: {}", t);
                return Task::perform(
                    async move {
                        daemon.start_rescan(t.try_into().expect("t cannot be inferior to 0 otherwise genesis block timestamp is chosen"))
                            .await
                            .map_err(|e| e.into())
                    },
                    Message::StartRescan,
                );
            }
            _ => {}
        };
        Task::none()
    }

    fn view<'a>(&self, cache: &'a Cache, can_edit: bool) -> Element<'a, view::SettingsEditMessage> {
        view::settings::rescan(
            &self.year,
            &self.month,
            &self.day,
            cache.rescan_progress(),
            self.success,
            self.processing,
            can_edit,
            self.invalid_date,
            self.past_possible_height,
            self.future_date,
        )
    }
}

/// Use bitcoin-cli getblock $(bitcoin-cli getblockhash 0) | jq .time
const MAINNET_GENESIS_BLOCK_TIMESTAMP: i64 = 1231006505;
const TESTNET3_GENESIS_BLOCK_TIMESTAMP: i64 = 1296688602;
const TESTNET4_GENESIS_BLOCK_TIMESTAMP: i64 = 1714777860;
const SIGNET_GENESIS_BLOCK_TIMESTAMP: i64 = 1598918400;

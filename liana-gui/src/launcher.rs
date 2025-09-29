use iced::{
    alignment::Horizontal,
    widget::{checkbox, pick_list, scrollable, Button, Space},
    Alignment, Length, Subscription, Task,
};

use liana::miniscript::bitcoin::Network;
use liana_ui::{
    component::{button, card, network_banner, notification, text::*},
    icon, image, theme,
    widget::{modal::Modal, Column, Container, Element, Row},
};
use lianad::config::ConfigError;
use tokio::runtime::Handle;

use crate::{
    app::{
        self,
        settings::{self, AuthConfig, WalletId, WalletSettings},
    },
    delete::{delete_wallet, DeleteError},
    dir::{LianaDirectory, NetworkDirectory},
    installer::UserFlow,
    services::connect::{
        client::{auth::AuthClient, backend::api::UserRole, get_service_config},
        login::{connect_with_credentials, BackendState},
    },
};

const NETWORKS: [Network; 5] = [
    Network::Bitcoin,
    Network::Testnet,
    Network::Testnet4,
    Network::Signet,
    Network::Regtest,
];

#[derive(Debug, Clone)]
pub enum State {
    Unchecked,
    Wallets {
        wallets: Vec<WalletSettings>,
        add_wallet: bool,
    },
    NoWallet,
}

pub struct Launcher {
    state: State,
    displayed_networks: Vec<Network>,
    network: Network,
    pub datadir_path: LianaDirectory,
    error: Option<String>,
    delete_wallet_modal: Option<DeleteWalletModal>,
}

impl Launcher {
    pub fn new(datadir_path: LianaDirectory, network: Option<Network>) -> (Self, Task<Message>) {
        let network = network.unwrap_or(
            NETWORKS
                .iter()
                .find(|net| has_existing_wallet(&datadir_path, **net))
                .cloned()
                .unwrap_or(Network::Bitcoin),
        );
        let network_dir = datadir_path.network_directory(network);
        (
            Self {
                state: State::Unchecked,
                displayed_networks: displayed_networks(&datadir_path),
                network,
                datadir_path: datadir_path.clone(),
                error: None,
                delete_wallet_modal: None,
            },
            Task::perform(check_network_datadir(network_dir), Message::Checked),
        )
    }

    pub fn reload(&self) -> Task<Message> {
        Task::perform(
            check_network_datadir(self.datadir_path.network_directory(self.network)),
            Message::Checked,
        )
    }

    pub fn stop(&mut self) {}

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::none()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::View(ViewMessage::ImportWallet) => {
                let datadir_path = self.datadir_path.clone();
                let network = self.network;
                Task::perform(async move { (datadir_path, network) }, |(d, n)| {
                    Message::Install(d, n, UserFlow::AddWallet)
                })
            }
            Message::View(ViewMessage::CreateWallet) => {
                let datadir_path = self.datadir_path.clone();
                let network = self.network;
                Task::perform(async move { (datadir_path, network) }, |(d, n)| {
                    Message::Install(d, n, UserFlow::CreateWallet)
                })
            }
            Message::View(ViewMessage::ShareXpubs) => {
                let datadir_path = self.datadir_path.clone();
                let network = self.network;
                Task::perform(async move { (datadir_path, network) }, |(d, n)| {
                    Message::Install(d, n, UserFlow::ShareXpubs)
                })
            }
            Message::View(ViewMessage::DeleteWallet(DeleteWalletMessage::ShowModal(i))) => {
                if let State::Wallets { wallets, .. } = &self.state {
                    let wallet_datadir = self.datadir_path.network_directory(self.network);
                    let config_path = wallet_datadir.path().join(app::config::DEFAULT_FILE_NAME);
                    let internal_bitcoind = if wallets[i].remote_backend_auth.is_some() {
                        Some(false)
                    } else if wallets[i].start_internal_bitcoind.is_some() {
                        wallets[i].start_internal_bitcoind
                    } else if let Ok(cfg) = app::Config::from_file(&config_path) {
                        Some(cfg.start_internal_bitcoind)
                    } else {
                        None
                    };
                    self.delete_wallet_modal = Some(DeleteWalletModal::new(
                        self.network,
                        wallet_datadir,
                        wallets[i].clone(),
                        internal_bitcoind,
                    ));
                }
                Task::none()
            }
            Message::View(ViewMessage::SelectNetwork(network)) => {
                self.network = network;
                let network_dir = self.datadir_path.network_directory(self.network);
                Task::perform(check_network_datadir(network_dir), Message::Checked)
            }
            Message::View(ViewMessage::DeleteWallet(DeleteWalletMessage::Deleted)) => {
                self.state = State::NoWallet;
                let network_dir = self.datadir_path.network_directory(self.network);
                Task::perform(check_network_datadir(network_dir), Message::Checked)
            }

            Message::View(ViewMessage::DeleteWallet(DeleteWalletMessage::CloseModal)) => {
                self.delete_wallet_modal = None;
                if self.network == Network::Testnet
                    && !has_existing_wallet(&self.datadir_path, Network::Testnet)
                {
                    self.network = Network::Testnet4;
                }
                Task::none()
            }
            Message::Checked(res) => match res {
                Err(e) => {
                    self.error = Some(e);
                    Task::none()
                }
                Ok(state) => {
                    self.state = state;
                    Task::none()
                }
            },
            Message::View(ViewMessage::AddWalletToList(add)) => {
                if let State::Wallets { add_wallet, .. } = &mut self.state {
                    *add_wallet = add;
                }
                Task::none()
            }
            Message::View(ViewMessage::Run(index)) => {
                if let State::Wallets { wallets, .. } = &self.state {
                    if let Some(settings) = wallets.get(index) {
                        let datadir_path = self.datadir_path.clone();
                        let mut path = self
                            .datadir_path
                            .network_directory(self.network)
                            .path()
                            .to_path_buf();
                        path.push(app::config::DEFAULT_FILE_NAME);
                        let cfg = app::Config::from_file(&path).expect("Already checked");
                        let network = self.network;
                        let settings = settings.clone();
                        Task::perform(
                            async move { (datadir_path.clone(), cfg, network, settings) },
                            |m| Message::Run(m.0, m.1, m.2, m.3),
                        )
                    } else {
                        Task::none()
                    }
                } else {
                    Task::none()
                }
            }
            _ => {
                if let Some(modal) = &mut self.delete_wallet_modal {
                    return modal.update(message);
                }
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<Message> {
        let content = Into::<Element<ViewMessage>>::into(scrollable(
            Column::new()
                .push(
                    Row::new()
                        .spacing(20)
                        .push(Container::new(
                            image::liana_brand_grey().width(Length::Fixed(200.0)),
                        ))
                        .push(
                            Container::new(image::retailer_logo().width(Length::Fixed(200.0)))
                                .width(Length::Fill),
                        )
                        .push_maybe(if let State::Wallets { add_wallet, .. } = &self.state {
                            if *add_wallet {
                                Some(
                                    button::secondary(
                                        Some(icon::previous_icon()),
                                        "Back to wallet list",
                                    )
                                    .on_press(ViewMessage::AddWalletToList(false)),
                                )
                            } else {
                                None
                            }
                        } else {
                            None
                        })
                        .push(
                            button::secondary(None, "Share Xpubs")
                                .on_press(ViewMessage::ShareXpubs),
                        )
                        .push(
                            pick_list(
                                self.displayed_networks.as_slice(),
                                Some(self.network),
                                ViewMessage::SelectNetwork,
                            )
                            .style(theme::pick_list::primary)
                            .padding(10),
                        )
                        .align_y(Alignment::Center)
                        .padding(100),
                )
                .push(
                    Container::new(
                        Column::new()
                            .align_x(Alignment::Center)
                            .spacing(30)
                            .push(if matches!(self.state, State::Wallets { .. }) {
                                text("Welcome back").size(50).bold()
                            } else {
                                text("Welcome").size(50).bold()
                            })
                            .push_maybe(self.error.as_ref().map(|e| card::simple(text(e))))
                            .push(match &self.state {
                                State::Unchecked => Column::new(),
                                State::Wallets {
                                    wallets,
                                    add_wallet,
                                } => {
                                    if *add_wallet {
                                        Column::new().push(add_wallet_menu())
                                    } else {
                                        let col = wallets.iter().enumerate().fold(
                                            Column::new().spacing(20),
                                            |col, (i, settings)| {
                                                col.push(wallets_list_item(
                                                    self.network,
                                                    settings,
                                                    i,
                                                ))
                                            },
                                        );
                                        col.push(
                                            Column::new().push(
                                                button::secondary(
                                                    Some(icon::plus_icon()),
                                                    "Add wallet",
                                                )
                                                .on_press(ViewMessage::AddWalletToList(true))
                                                .padding(10)
                                                .width(Length::Fixed(500.0)),
                                            ),
                                        )
                                    }
                                }
                                State::NoWallet => Column::new().push(add_wallet_menu()),
                            })
                            .align_x(Alignment::Center),
                    )
                    .center_x(Length::Fill),
                )
                .push(Space::with_height(Length::Fixed(100.0))),
        ))
        .map(Message::View);
        let content = if self.network != Network::Bitcoin {
            Column::with_children(vec![network_banner(self.network).into(), content]).into()
        } else {
            content
        };
        if let Some(modal) = &self.delete_wallet_modal {
            Modal::new(Container::new(content).height(Length::Fill), modal.view())
                .on_blur(Some(Message::View(ViewMessage::DeleteWallet(
                    DeleteWalletMessage::CloseModal,
                ))))
                .into()
        } else {
            content
        }
    }
}

fn add_wallet_menu<'a>() -> Element<'a, ViewMessage> {
    Row::new()
        .align_y(Alignment::End)
        .spacing(20)
        .push(
            Container::new(
                Column::new()
                    .spacing(20)
                    .align_x(Alignment::Center)
                    .push(image::create_new_wallet_icon().width(Length::Fixed(100.0)))
                    .push(p1_regular("Create a new Liana wallet").style(theme::text::secondary))
                    .push(
                        button::secondary(None, "Select")
                            .width(Length::Fixed(200.0))
                            .on_press(ViewMessage::CreateWallet),
                    )
                    .align_x(Alignment::Center),
            )
            .padding(20),
        )
        .push(
            Container::new(
                Column::new()
                    .spacing(20)
                    .align_x(Alignment::Center)
                    .push(image::restore_wallet_icon().width(Length::Fixed(100.0)))
                    .push(p1_regular("Add an existing Liana wallet").style(theme::text::secondary))
                    .push(
                        button::secondary(None, "Select")
                            .width(Length::Fixed(200.0))
                            .on_press(ViewMessage::ImportWallet),
                    )
                    .align_x(Alignment::Center),
            )
            .padding(20),
        )
        .into()
}

fn wallets_list_item(
    network: Network,
    settings: &WalletSettings,
    i: usize,
) -> Element<ViewMessage> {
    Container::new(
        Row::new()
            .align_y(Alignment::Center)
            .spacing(20)
            .push(
                Container::new(
                    Button::new(
                        Column::new()
                            .push(if let Some(alias) = &settings.alias {
                                p1_bold(alias)
                            } else {
                                p1_bold(format!(
                                    "My Liana {} wallet",
                                    match network {
                                        Network::Bitcoin => "Bitcoin",
                                        Network::Signet => "Signet",
                                        Network::Testnet => "Testnet",
                                        Network::Testnet4 => "Testnet4",
                                        Network::Regtest => "Regtest",
                                        _ => "",
                                    }
                                ))
                            })
                            .push(
                                p1_regular(format!("Liana-{}", settings.descriptor_checksum))
                                    .style(theme::text::secondary),
                            )
                            .push_maybe(settings.remote_backend_auth.as_ref().map(|auth| {
                                Row::new()
                                    .push(Space::with_width(Length::Fill))
                                    .push(p1_regular(&auth.email).style(theme::text::secondary))
                            })),
                    )
                    .on_press(ViewMessage::Run(i))
                    .padding(15)
                    .style(theme::button::container_border)
                    .width(Length::Fixed(500.0)),
                )
                .style(theme::card::simple),
            )
            .push(
                Button::new(icon::trash_icon())
                    .style(theme::button::secondary)
                    .padding(10)
                    .on_press(ViewMessage::DeleteWallet(DeleteWalletMessage::ShowModal(i))),
            ),
    )
    .into()
}

/// Returns the list of displayed networks.
///
/// `Testnet` is not displayed if no wallet already exists as `Testnet4` should be available.
fn displayed_networks(dir: &LianaDirectory) -> Vec<Network> {
    let mut networks = NETWORKS.to_vec();

    networks.retain(|&n| match n {
        Network::Testnet => has_existing_wallet(dir, Network::Testnet),
        _ => true,
    });

    networks
}

fn has_existing_wallet(data_dir: &LianaDirectory, network: Network) -> bool {
    data_dir
        .path()
        .join(network.to_string())
        .join(settings::SETTINGS_FILE_NAME)
        .exists()
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Message {
    View(ViewMessage),
    Install(LianaDirectory, Network, UserFlow),
    Checked(Result<State, String>),
    Run(LianaDirectory, app::config::Config, Network, WalletSettings),
}

#[derive(Debug, Clone)]
pub enum ViewMessage {
    ImportWallet,
    CreateWallet,
    AddWalletToList(bool),
    ShareXpubs,
    SelectNetwork(Network),
    StartInstall(Network),
    Check,
    Run(usize),
    DeleteWallet(DeleteWalletMessage),
}

#[derive(Debug, Clone)]
pub enum DeleteWalletMessage {
    ShowModal(usize),
    CloseModal,
    Confirm(WalletId),
    DeleteLianaConnect(bool),
    Deleted,
}

struct DeleteWalletModal {
    network: Network,
    network_directory: NetworkDirectory,
    wallet_settings: WalletSettings,
    warning: Option<DeleteError>,
    deleted: bool,
    delete_liana_connect: bool,
    user_role: Option<UserRole>,
    // `None` means we were not able to determine whether wallet uses internal bitcoind.
    internal_bitcoind: Option<bool>,
}

impl DeleteWalletModal {
    fn new(
        network: Network,
        network_directory: NetworkDirectory,
        wallet_settings: WalletSettings,
        internal_bitcoind: Option<bool>,
    ) -> Self {
        let mut modal = Self {
            network,
            wallet_settings,
            network_directory,
            warning: None,
            deleted: false,
            delete_liana_connect: false,
            internal_bitcoind,
            user_role: None,
        };
        if let Some(auth) = &modal.wallet_settings.remote_backend_auth {
            match Handle::current().block_on(check_membership(
                modal.network,
                &modal.network_directory,
                auth,
            )) {
                Err(e) => {
                    modal.warning = Some(e);
                }
                Ok(user_role) => {
                    modal.user_role = user_role;
                }
            }
        }
        modal
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::View(ViewMessage::DeleteWallet(DeleteWalletMessage::Confirm(wallet_id))) => {
                if wallet_id != self.wallet_settings.wallet_id() {
                    return Task::none();
                }
                self.warning = None;
                if let Err(e) = Handle::current().block_on(delete_wallet(
                    self.network,
                    &self.network_directory,
                    &self.wallet_settings,
                    self.delete_liana_connect,
                )) {
                    self.warning = Some(e);
                } else {
                    self.deleted = true;
                    return Task::perform(async {}, |_| {
                        Message::View(ViewMessage::DeleteWallet(DeleteWalletMessage::Deleted))
                    });
                };
            }
            Message::View(ViewMessage::DeleteWallet(DeleteWalletMessage::DeleteLianaConnect(
                delete,
            ))) => {
                self.delete_liana_connect = delete;
            }
            _ => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<Message> {
        let mut confirm_button = button::secondary(None, "Delete wallet")
            .width(Length::Fixed(200.0))
            .style(theme::button::destructive);
        if self.warning.is_none() {
            confirm_button = confirm_button.on_press(ViewMessage::DeleteWallet(
                DeleteWalletMessage::Confirm(self.wallet_settings.wallet_id()),
            ));
        }
        // Use separate `Row`s for help text in order to have better spacing.
        let help_text_1 = format!(
            "Are you sure you want to {} for the wallet {}",
            if self.wallet_settings.remote_backend_auth.is_some() {
                "delete locally the configuration"
            } else {
                "delete the configuration and all associated data"
            },
            if let Some(alias) = &self.wallet_settings.alias {
                format!(
                    "{} (Liana-{})?",
                    alias, self.wallet_settings.descriptor_checksum
                )
            } else {
                format!("Liana-{}?", &self.wallet_settings.descriptor_checksum)
            }
        );
        let help_text_2 = match self.internal_bitcoind {
            Some(true) => Some("(The Liana-managed Bitcoin node for this network will not be affected by this action.)"),
            Some(false) => None,
            None => Some("(If you are using a Liana-managed Bitcoin node, it will not be affected by this action.)"),
        };
        let help_text_3 = "WARNING: This cannot be undone.";

        Into::<Element<ViewMessage>>::into(
            card::simple(
                Column::new()
                    .spacing(10)
                    .push(Container::new(
                        h4_bold(if let Some(alias) = &self.wallet_settings.alias {
                            format!(
                                "Delete configuration for {} (Liana-{})",
                                alias, &self.wallet_settings.descriptor_checksum
                            )
                        } else {
                            format!(
                                "Delete configuration for Liana-{}",
                                &self.wallet_settings.descriptor_checksum
                            )
                        })
                        .style(theme::text::destructive)
                        .width(Length::Fill),
                    ))
                    .push(Row::new().push(text(help_text_1)))
                    .push_maybe(
                        help_text_2
                            .map(|t| Row::new().push(p1_regular(t).style(theme::text::secondary))),
                    )
                    .push(Row::new())
                    .push_maybe(self.wallet_settings.remote_backend_auth.as_ref().map(|a| {
                        checkbox(
                            match self.user_role {
                                Some(UserRole::Owner) | None => "Also permanently delete this wallet from Liana Connect (for all members).".to_string(),
                                Some(UserRole::Member) => format!("Also disassociate {} from this Liana Connect wallet.", a.email),
                            },
                            self.delete_liana_connect,
                        )
                        .on_toggle_maybe(if !self.deleted {
                                Some(|v| {
                                    ViewMessage::DeleteWallet(DeleteWalletMessage::DeleteLianaConnect(v))
                                })
                            } else {
                                None
                            })
                    }))
                    .push(Row::new().push(text(help_text_3)))
                    .push_maybe(self.warning.as_ref().map(|w| {
                        notification::warning(w.to_string(), w.to_string()).width(Length::Fill)
                    }))
                    .push(
                        Container::new(if !self.deleted {
                            Row::new().push(confirm_button)
                        } else {
                            Row::new()
                                .spacing(10)
                                .push(icon::circle_check_icon().style(theme::text::success))
                                .push(
                                    text("Wallet successfully deleted").style(theme::text::success),
                                )
                        })
                        .align_x(Horizontal::Center)
                        .width(Length::Fill),
                    ),
            )
            .width(Length::Fixed(700.0)),
        )
        .map(Message::View)
    }
}

pub async fn check_membership(
    network: Network,
    network_dir: &NetworkDirectory,
    auth: &AuthConfig,
) -> Result<Option<UserRole>, DeleteError> {
    let service_config = get_service_config(network)
        .await
        .map_err(|e| DeleteError::Connect(e.to_string()))?;

    if let BackendState::WalletExists(client, _, _) = connect_with_credentials(
        AuthClient::new(
            service_config.auth_api_url,
            service_config.auth_api_public_key,
            auth.email.to_string(),
        ),
        auth.wallet_id.clone(),
        service_config.backend_api_url,
        network,
        network_dir,
    )
    .await
    .map_err(|e| DeleteError::Connect(e.to_string()))?
    {
        Ok(Some(
            client
                .user_wallet_membership()
                .await
                .map_err(|e| DeleteError::Connect(e.to_string()))?,
        ))
    } else {
        Ok(None)
    }
}

async fn check_network_datadir(path: NetworkDirectory) -> Result<State, String> {
    let mut config_path = path.clone().path().to_path_buf();
    config_path.push(app::config::DEFAULT_FILE_NAME);

    if let Err(e) = app::Config::from_file(&config_path) {
        if e == app::config::ConfigError::NotFound {
            return Ok(State::NoWallet);
        } else {
            return Err(format!(
                "Failed to read GUI configuration file in the directory: {}",
                path.path().to_string_lossy()
            ));
        }
    };

    let mut daemon_config_path = path.clone().path().to_path_buf();
    daemon_config_path.push("daemon.toml");

    if daemon_config_path.exists() {
        lianad::config::Config::from_file(Some(daemon_config_path.clone())).map_err(|e| match e {
        ConfigError::FileNotFound
        | ConfigError::DatadirNotFound => {
            format!(
                "Failed to read daemon configuration file in the directory: {}",
                daemon_config_path.to_string_lossy()
            )
        }
        ConfigError::ReadingFile(e) => {
            if e.starts_with("Parsing configuration file: Error parsing descriptor") {
                "There is an issue with the configuration for this network. You most likely use a descriptor containing one or more public key(s) without origin. Liana v0.2 and later only support public keys with origins. Please migrate your funds using Liana v0.1.".to_string()
            } else {
                format!(
                    "Failed to read daemon configuration file in the directory: {}",
                    daemon_config_path.to_string_lossy()
                )
            }
        }
        ConfigError::UnexpectedDescriptor(_) => {
            "There is an issue with the configuration for this network. You most likely use a descriptor containing one or more public key(s) without origin. Liana v0.2 and later only support public keys with origins. Please migrate your funds using Liana v0.1.".to_string()
        }
        ConfigError::Unexpected(e) => {
            format!(
                "Unexpected {}",
                e,
            )
        }
    })?;
    }

    match settings::Settings::from_file(&path) {
        Ok(s) => {
            if s.wallets.is_empty() {
                Ok(State::NoWallet)
            } else {
                Ok(State::Wallets {
                    wallets: s.wallets,
                    add_wallet: false,
                })
            }
        }
        Err(settings::SettingsError::NotFound) => Ok(State::NoWallet),
        Err(e) => Err(e.to_string()),
    }
}

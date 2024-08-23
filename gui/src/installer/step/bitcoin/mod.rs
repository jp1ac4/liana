pub mod bitcoind;
pub mod electrum;

use crate::{
    bitcoin::{self, BackendType},
    hw::HardwareWallets,
    installer::{
        context::Context,
        message::{self, Message},
        step::{
            bitcoin::{bitcoind::DefineBitcoind, electrum::DefineElectrum},
            Step,
        },
        view, Error,
    },
};

use iced::Command;
use liana_ui::widget::Element;

#[derive(Clone)]
pub enum BitcoinBackendDefinition {
    Bitcoind(DefineBitcoind),
    Electrum(DefineElectrum),
}

pub struct BitcoinBackend {
    definition: BitcoinBackendDefinition,
    is_running: Option<Result<(), Error>>,
}

impl BitcoinBackend {
    fn new(backend_type: &bitcoin::BackendType) -> Self {
        let definition = match backend_type {
            bitcoin::BackendType::Bitcoind => {
                BitcoinBackendDefinition::Bitcoind(DefineBitcoind::new())
            }
            bitcoin::BackendType::Electrum => {
                BitcoinBackendDefinition::Electrum(DefineElectrum::new())
            }
        };
        BitcoinBackend {
            definition,
            is_running: None,
        }
    }
}

impl BitcoinBackendDefinition {
    fn apply(&mut self, ctx: &mut Context) -> bool {
        match *self {
            BitcoinBackendDefinition::Bitcoind(ref mut def) => def.apply(ctx),
            BitcoinBackendDefinition::Electrum(ref mut def) => def.apply(ctx),
        }
    }

    fn backend_type(&self) -> BackendType {
        match self {
            BitcoinBackendDefinition::Bitcoind(_) => BackendType::Bitcoind,
            BitcoinBackendDefinition::Electrum(_) => BackendType::Electrum,
        }
    }

    fn can_try_ping(&self) -> bool {
        match self {
            BitcoinBackendDefinition::Bitcoind(ref def) => def.can_try_ping(),
            BitcoinBackendDefinition::Electrum(ref def) => def.can_try_ping(),
        }
    }

    fn load_context(&mut self, ctx: &Context) {
        match *self {
            BitcoinBackendDefinition::Bitcoind(ref mut def) => def.load_context(ctx),
            BitcoinBackendDefinition::Electrum(ref mut def) => def.load_context(ctx),
        }
    }

    fn update(&mut self, message: message::DefineBitcoinBackend) -> Command<Message> {
        match *self {
            BitcoinBackendDefinition::Bitcoind(ref mut def) => def.update(message),
            BitcoinBackendDefinition::Electrum(ref mut def) => def.update(message),
        }
    }

    fn view(&self) -> Element<Message> {
        match self {
            BitcoinBackendDefinition::Bitcoind(ref def) => def.view(),
            BitcoinBackendDefinition::Electrum(ref def) => def.view(),
        }
    }

    fn ping(&self) -> Result<(), Error> {
        match self {
            BitcoinBackendDefinition::Bitcoind(ref def) => def.ping(),
            BitcoinBackendDefinition::Electrum(ref def) => def.ping(),
        }
    }
}

pub struct DefineBitcoinBackend {
    backends: Vec<BitcoinBackend>,
    selected_backend_type: bitcoin::BackendType,
}

impl From<DefineBitcoinBackend> for Box<dyn Step> {
    fn from(s: DefineBitcoinBackend) -> Box<dyn Step> {
        Box::new(s)
    }
}

impl DefineBitcoinBackend {
    pub fn new(selected_backend_type: bitcoin::BackendType) -> Self {
        let backends = [
            // This is the order in which the available backends will be shown to the user.
            bitcoin::BackendType::Bitcoind,
            bitcoin::BackendType::Electrum,
        ]
        .iter()
        .map(BitcoinBackend::new)
        .collect();

        Self {
            backends,
            selected_backend_type,
        }
    }

    pub fn selected_mut(&mut self) -> &mut BitcoinBackend {
        self.get_mut(&self.selected_backend_type.clone())
            .expect("selected type must be present")
    }

    pub fn selected(&self) -> &BitcoinBackend {
        self.get(&self.selected_backend_type)
            .expect("selected type must be present")
    }

    pub fn get_mut(&mut self, backend_type: &bitcoin::BackendType) -> Option<&mut BitcoinBackend> {
        self.backends
            .iter_mut()
            .find(|backend| backend.definition.backend_type() == *backend_type)
    }

    pub fn get(&self, backend_type: &bitcoin::BackendType) -> Option<&BitcoinBackend> {
        self.backends
            .iter()
            .find(|backend| backend.definition.backend_type() == *backend_type)
    }

    fn ping_selected(&self) -> Command<Message> {
        let selected = self.selected().definition.clone();
        let backend_type = selected.backend_type();
        Command::perform(async move { selected.ping() }, move |res| {
            Message::DefineBitcoinBackend(message::DefineBitcoinBackend::PingResult((
                backend_type,
                res,
            )))
        })
    }

    fn update_backend(
        &mut self,
        backend_type: BackendType,
        message: message::DefineBitcoinBackend,
    ) -> Command<Message> {
        if let Some(backend) = self.get_mut(&backend_type) {
            backend.is_running = None;
            return backend.definition.update(message);
        }
        Command::none()
    }
}

impl Step for DefineBitcoinBackend {
    fn load_context(&mut self, ctx: &Context) {
        for backend in self.backends.iter_mut() {
            backend.definition.load_context(ctx);
        }
    }
    fn update(&mut self, _hws: &mut HardwareWallets, message: Message) -> Command<Message> {
        if let Message::DefineBitcoinBackend(msg) = message {
            match msg {
                message::DefineBitcoinBackend::BackendTypeSelected(backend_type) => {
                    self.selected_backend_type = backend_type;
                }
                message::DefineBitcoinBackend::Ping => {
                    return self.ping_selected();
                }
                message::DefineBitcoinBackend::PingResult((backend_type, res)) => {
                    // Result may not be for the selected backend type.
                    if let Some(backend) = self.get_mut(&backend_type) {
                        backend.is_running = Some(res);
                    }
                }
                // We don't assume the backend message is for the selected backend,
                // e.g. in case user changes selection before message arrives.
                msg @ message::DefineBitcoinBackend::DefineBitcoind(_) => {
                    return self.update_backend(BackendType::Bitcoind, msg);
                }
                msg @ message::DefineBitcoinBackend::DefineElectrum(_) => {
                    return self.update_backend(BackendType::Electrum, msg);
                }
            }
        }
        Command::none()
    }

    fn apply(&mut self, ctx: &mut Context) -> bool {
        self.selected_mut().definition.apply(ctx)
    }

    fn view(
        &self,
        _hws: &HardwareWallets,
        progress: (usize, usize),
        _email: Option<&str>,
    ) -> Element<Message> {
        view::define_bitcoin_backend(
            progress,
            self.backends
                .iter()
                .map(|backend| backend.definition.backend_type()),
            &self.selected_backend_type,
            self.selected().definition.view(),
            self.selected().is_running.as_ref(),
            self.selected().definition.can_try_ping(),
        )
    }

    fn load(&self) -> Command<Message> {
        self.ping_selected()
    }

    fn skip(&self, ctx: &Context) -> bool {
        !ctx.bitcoind_is_external || ctx.remote_backend.is_some()
    }
}

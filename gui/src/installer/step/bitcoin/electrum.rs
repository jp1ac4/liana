use iced::Command;
use liana::{
    config::ElectrumConfig,
    electrum_client::{self, ElectrumApi},
};
use liana_ui::{component::form, widget::*};

use crate::{
    bitcoin::electrum::ConfigField,
    installer::{
        context::Context,
        message::{self, Message},
        view, Error,
    },
};

#[derive(Clone)]
pub struct DefineElectrum {
    address: form::Value<String>,
}

impl DefineElectrum {
    pub fn new() -> Self {
        Self {
            address: form::Value::default(),
        }
    }

    pub fn can_try_ping(&self) -> bool {
        !self.address.value.is_empty() && self.address.valid
    }

    pub fn load_context(&mut self, _ctx: &Context) {}

    pub fn update(&mut self, message: message::DefineBitcoinBackend) -> Command<Message> {
        if let message::DefineBitcoinBackend::DefineElectrum(msg) = message {
            match msg {
                message::DefineElectrum::ConfigFieldEdited(field, value) => match field {
                    ConfigField::Address => {
                        self.address.value.clone_from(&value);
                        self.address.valid = !self.address.value.is_empty();
                    }
                },
            };
        };
        Command::none()
    }

    pub fn apply(&mut self, ctx: &mut Context) -> bool {
        if self.can_try_ping() {
            ctx.bitcoin_backend = Some(liana::config::BitcoinBackend::Electrum(ElectrumConfig {
                addr: self.address.value.clone(),
            }));
            return true;
        }
        false
    }

    pub fn view(&self) -> Element<Message> {
        view::define_electrum(&self.address)
    }

    pub fn ping(&self) -> Result<(), Error> {
        let builder = electrum_client::Config::builder();
        let config = builder.timeout(Some(3)).build();
        let client = electrum_client::Client::from_config(&self.address.value, config)
            .map_err(|e| Error::Electrum(e.to_string()))?;
        client
            .raw_call("server.ping", [])
            .map_err(|e| Error::Electrum(e.to_string()))?;
        Ok(())
    }
}

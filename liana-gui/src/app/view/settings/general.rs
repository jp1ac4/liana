use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use iced::alignment::Vertical;
use iced::widget::{Column, Rule};
use iced::{
    alignment,
    widget::{radio, scrollable, tooltip as iced_tooltip, Space},
    Alignment, Length,
};

use liana::{
    descriptors::{LianaDescriptor, LianaPolicy},
    miniscript::bitcoin::{bip32::Fingerprint, Network},
};
use lianad::config::BitcoindRpcAuth;

use super::{dashboard, message::*};

use liana_ui::{
    component::{badge, button, card, form, separation, text::*, tooltip::tooltip},
    icon,
    theme::{self},
    widget::*,
};

use crate::{
    app::{
        cache::Cache,
        error::Error,
        menu::Menu,
        settings::ProviderKey,
        view::{hw, warning::warn},
    },
    help,
    hw::HardwareWallet,
    node::{
        bitcoind::{RpcAuthType, RpcAuthValues},
        electrum::{self, validate_domain_checkbox},
    },
};

pub fn section<'a>(
    cache: &'a Cache,
    email_form: &form::Value<String>,
    processing: bool,
    success: bool,
    warning: Option<&Error>,
) -> Element<'a, Message> {
    let header = header("General", SettingsMessage::EditRemoteBackendSettings);

    let content = card::simple(
        Column::new()
            .spacing(20)
            .push(text("Grant access to wallet to another user"))
            .push(
                form::Form::new_trimmed("User email", email_form, |email| {
                    Message::Settings(SettingsMessage::RemoteBackendSettings(
                        RemoteBackendSettingsMessage::EditInvitationEmail(email),
                    ))
                })
                .warning("Email is invalid")
                .size(P1_SIZE)
                .padding(10),
            )
            .push(
                Row::new()
                    .push_maybe(if success {
                        Some(text("Invitation was sent").style(theme::text::success))
                    } else {
                        None
                    })
                    .push(Space::with_width(Length::Fill))
                    .push(button::secondary(None, "Send invitation").on_press_maybe(
                        if !processing && email_form.valid {
                            Some(Message::Settings(SettingsMessage::RemoteBackendSettings(
                                RemoteBackendSettingsMessage::SendInvitation,
                            )))
                        } else {
                            None
                        },
                    )),
            ),
    )
    .width(Length::Fill);

    dashboard(
        &Menu::Settings,
        cache,
        warning,
        Column::new()
            .spacing(20)
            .push(header)
            .push(content)
            .push(link(
                help::CHANGE_BACKEND_OR_NODE_URL,
                "I want to connect to my own node",
            )),
    )
}

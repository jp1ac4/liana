use iced::widget::{pick_list, radio, Column};
use iced::Alignment;
use iced::{widget::Space, Length};

use super::header;

use liana_ui::{
    component::{button, card, form, text::*},
    theme,
    widget::*,
};

use crate::app::cache;
use crate::app::settings::fiat::PriceSetting;
use crate::app::{
    error::Error,
    menu::Menu,
    view::{dashboard, message::*},
};
use crate::services::fiat::{Currency, ALL_PRICE_SOURCES};

pub fn general_section<'a>(
    cache: &'a cache::Cache,
    new_price_setting: &'a PriceSetting,
    currencies_list: &'a [Currency],
    warning: Option<&Error>,
) -> Element<'a, Message> {
    let header = header("General", SettingsMessage::GeneralSection);

    let content = card::simple(
        Column::new()
            .spacing(20)
            .push(
                [true, false].iter().fold(
                    Row::new()
                        .push(text("Fiat price").small().bold())
                        .spacing(10),
                    |row, enable| {
                        row.push(radio(
                            match enable {
                                true => "On",
                                false => "Off",
                            },
                            enable,
                            Some(&new_price_setting.is_enabled),
                            |new_selection| {
                                Message::Settings(SettingsMessage::Fiat(FiatMessage::Enable(
                                    *new_selection,
                                )))
                            },
                        ))
                        .spacing(30)
                        .align_y(Alignment::Center)
                    },
                ),
            )
            .push_maybe(
                new_price_setting.is_enabled.then_some(
                    pick_list(
                        &ALL_PRICE_SOURCES[..],
                        Some(new_price_setting.source),
                        |source| {
                            Message::Settings(SettingsMessage::Fiat(FiatMessage::SourceEdited(
                                source,
                            )))
                        },
                    )
                    .style(theme::pick_list::primary)
                    .padding(10),
                ),
            )
            .push_maybe(
                new_price_setting.is_enabled.then_some(
                    pick_list(
                        currencies_list,
                        Some(new_price_setting.currency),
                        |currency| {
                            Message::Settings(SettingsMessage::Fiat(FiatMessage::CurrencyEdited(
                                currency,
                            )))
                        },
                    )
                    .style(theme::pick_list::primary)
                    .padding(10),
                ),
            ),
    )
    .width(Length::Fill);

    dashboard(
        &Menu::Settings,
        cache,
        warning,
        Column::new().spacing(20).push(header).push(content),
    )
}

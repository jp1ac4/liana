use iced::widget::{pick_list, radio, Column, Row, Space};
use iced::{Alignment, Length};

use super::header;

use liana_ui::component::card;
use liana_ui::component::text::*;
use liana_ui::theme;
use liana_ui::widget::*;

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
    settings: Vec<Element<'a, Message>>,
    warning: Option<&Error>,
) -> Element<'a, Message> {
    let header = header("General", SettingsMessage::GeneralSection);

    dashboard(
        &Menu::Settings,
        cache,
        warning,
        Column::new()
            .spacing(20)
            .push(header)
            .push(Column::with_children(settings).spacing(20)),
    )
}

pub fn fiat_price<'a>(
    new_price_setting: &'a PriceSetting,
    currencies_list: &'a [Currency],
) -> Element<'a, Message> {
    card::simple(
        Column::new()
            .spacing(20)
            .push(
                [true, false].iter().fold(
                    Row::new()
                        .push(text("Fiat price:").bold())
                        .push(Space::with_width(Length::Fill))
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
                    Row::new()
                        .spacing(20)
                        .align_y(Alignment::Center)
                        .push(text("Exchange rate source:").bold())
                        .push(Space::with_width(Length::Fill))
                        .push(
                            pick_list(
                                &ALL_PRICE_SOURCES[..],
                                Some(new_price_setting.source),
                                |source| {
                                    Message::Settings(SettingsMessage::Fiat(
                                        FiatMessage::SourceEdited(source),
                                    ))
                                },
                            )
                            .style(theme::pick_list::primary)
                            .padding(10),
                        ),
                ),
            )
            .push_maybe(
                new_price_setting.is_enabled.then_some(
                    Row::new()
                        .spacing(20)
                        .align_y(Alignment::Center)
                        .push(text("Currency:").bold())
                        .push(Space::with_width(Length::Fill))
                        .push(
                            pick_list(
                                currencies_list,
                                Some(new_price_setting.currency),
                                |currency| {
                                    Message::Settings(SettingsMessage::Fiat(
                                        FiatMessage::CurrencyEdited(currency),
                                    ))
                                },
                            )
                            .style(theme::pick_list::primary)
                            .padding(10),
                        ),
                ),
            ),
    )
    .width(Length::Fill)
    .into()
}

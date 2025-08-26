use iced::widget::{pick_list, Column, Row, Space, Toggler};
use iced::{Alignment, Length};

use super::header;

use liana_ui::component::card;
use liana_ui::component::text::*;
use liana_ui::theme;
use liana_ui::widget::*;

use crate::app::cache;
use crate::app::error::Error;
use crate::app::menu::Menu;
use crate::app::view::dashboard;
use crate::app::view::message::*;
use crate::app::view::settings::SettingsMessage;
use crate::services::fiat::{Currency, PriceSource, ALL_PRICE_SOURCES};

pub fn general_section<'a>(
    cache: &'a cache::Cache,
    fiat_is_enabled: bool,
    source: PriceSource,
    currency: Option<Currency>,
    currencies_list: &'a [Currency],
    warning: Option<&Error>,
) -> Element<'a, Message> {
    let header = header("General", SettingsMessage::GeneralSection);

    dashboard(
        &Menu::Settings,
        cache,
        warning,
        Column::new().spacing(20).push(header).push(fiat_price(
            fiat_is_enabled,
            source,
            currency,
            currencies_list,
        )),
    )
}

pub fn fiat_price(
    is_enabled: bool,
    source: PriceSource,
    currency: Option<Currency>,
    currencies_list: &[Currency],
) -> Element<'_, Message> {
    card::simple(
        Column::new()
            .spacing(20)
            .push(
                Row::new()
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .push(text("Fiat price:").bold())
                    .push(Space::with_width(Length::Fill))
                    .push(
                        Toggler::new(is_enabled)
                            .on_toggle(|new_selection| {
                                Message::Settings(SettingsMessage::Fiat(FiatMessage::Enable(
                                    new_selection,
                                )))
                            })
                            .style(theme::toggler::primary),
                    ),
            )
            .push_maybe(
                is_enabled.then_some(
                    Row::new()
                        .spacing(20)
                        .align_y(Alignment::Center)
                        .push(text("Exchange rate source:").bold())
                        .push(Space::with_width(Length::Fill))
                        .push(
                            pick_list(&ALL_PRICE_SOURCES[..], Some(source), |source| {
                                Message::Settings(SettingsMessage::Fiat(FiatMessage::SourceEdited(
                                    source,
                                )))
                            })
                            .style(theme::pick_list::primary)
                            .padding(10),
                        ),
                ),
            )
            .push_maybe(
                is_enabled.then_some(
                    Row::new()
                        .spacing(20)
                        .align_y(Alignment::Center)
                        .push(text("Currency:").bold())
                        .push(Space::with_width(Length::Fill))
                        .push(
                            pick_list(currencies_list, currency, |currency| {
                                Message::Settings(SettingsMessage::Fiat(
                                    FiatMessage::CurrencyEdited(currency),
                                ))
                            })
                            .style(theme::pick_list::primary)
                            .padding(10),
                        ),
                ),
            )
            .push_maybe(source.attribution().filter(|_| is_enabled).map(|s| {
                Row::new()
                    .spacing(20)
                    .align_y(Alignment::Center)
                    .push(Space::with_width(Length::Fill))
                    .push(text(s))
            })),
    )
    .width(Length::Fill)
    .into()
}

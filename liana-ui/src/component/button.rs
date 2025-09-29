use super::text::text;
use crate::font::{BOLD, MEDIUM};
use crate::{theme, widget::*};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{button, container, row};

pub fn menu<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Button<'a, T> {
    Button::new(content_menu(icon.map(|i| i.style(theme::text::secondary)), t).padding(10))
        .style(theme::button::menu)
}

pub fn menu_active<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Button<'a, T> {
    Button::new(content_menu(icon.map(|i| i.style(theme::text::secondary)), t).padding(10))
        .style(theme::button::menu_pressed)
}

pub fn menu_small<'a, T: 'a>(icon: Text<'a>) -> Button<'a, T> {
    Button::new(
        container(icon.style(theme::text::secondary))
            .padding(10)
            .align_x(Horizontal::Center),
    )
    .style(theme::button::menu)
}

pub fn menu_active_small<'a, T: 'a>(icon: Text<'a>) -> Button<'a, T> {
    Button::new(
        container(icon.style(theme::text::secondary))
            .padding(10)
            .align_x(Horizontal::Center),
    )
    .style(theme::button::menu_pressed)
}

fn content_menu<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Container<'a, T> {
    match icon {
        None => container(text(t)).padding(5),
        Some(i) => container(row![i, text(t)].spacing(10).align_y(Vertical::Center)).padding(5),
    }
}

pub fn alert<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Button<'a, T> {
    Button::new(content(icon, text(t))).style(theme::button::destructive)
}

pub fn primary<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Button<'a, T> {
    Button::new(content(icon, text(t).font(MEDIUM))).style(theme::button::primary)
}

pub fn transparent<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Button<'a, T> {
    Button::new(content(icon, text(t))).style(theme::button::container)
}

pub fn secondary<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Button<'a, T> {
    Button::new(content(icon, text(t))).style(theme::button::secondary)
}

pub fn retailer<'a, T: 'a>(t: &'static str) -> Button<'a, T> {
    button::Button::new(content_retailer(t)).style(theme::button::retailer)
}

pub fn border<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Button<'a, T> {
    Button::new(content(icon, text(t))).style(theme::button::secondary)
}

pub fn transparent_border<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Button<'a, T> {
    button(content(icon, text(t))).style(theme::button::container_border)
}

pub fn link<'a, T: 'a>(icon: Option<Text<'a>>, t: &'static str) -> Button<'a, T> {
    Button::new(content(icon, text(t))).style(theme::button::link)
}

fn content<'a, T: 'a>(icon: Option<Text<'a>>, text: Text<'a>) -> Container<'a, T> {
    match icon {
        None => container(text).align_x(Horizontal::Center).padding(5),
        Some(i) => container(row![i, text].spacing(10).align_y(Vertical::Center))
            .align_x(Horizontal::Center)
            .padding(5),
    }
}

fn content_retailer<'a, T: 'a>(t: &'static str) -> Container<'a, T> {
    container(text(t).font(BOLD))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .padding(5)
}

use iced::Task;
use std::collections::HashMap;

use crate::app;
use crate::app::cache::{self, FiatPriceRequest};
use crate::app::message::FiatMessage;
use crate::gui::pane;
use crate::gui::tab;
use crate::gui::Config;
use crate::gui::Message;
use crate::services::fiat::api::{ListCurrenciesResult, PriceApi};
use crate::services::fiat::{Currency, PriceClient, PriceSource};
use crate::utils::now;

use iced::widget::pane_grid;

use crate::app::cache::FIAT_PRICE_UPDATE_INTERVAL_SECS;

/// Time to live of the list of available currencies for a given `PriceSource`.
const CURRENCIES_LIST_TTL_SECS: u64 = 3_600; // 1 hour

#[derive(Default)]
pub struct FiatPricesCache {
    pub prices: HashMap<(PriceSource, Currency), app::cache::FiatPrice>,
    pub last_requests: HashMap<(PriceSource, Currency), FiatPriceRequest>,
    pub currencies: HashMap<PriceSource, (/* timestamp */ u64, Vec<Currency>)>,
}

#[derive(Default)]
pub struct GlobalCache {
    pub fiat_prices: FiatPricesCache,
}

impl GlobalCache {
    pub fn handle_fiat_message(
        &mut self,
        pane_id: pane_grid::Pane,
        pane: &mut pane::Pane,
        tab_id: usize,
        config: &Config,
        fiat_msg: FiatMessage,
    ) -> Task<Message> {
        if let Some(tab) = pane.tabs.iter().find(|t| t.id == tab_id) {
            if let Some(price_setting) = tab
                .wallet()
                .and_then(|w| w.fiat_price_setting.as_ref().filter(|sett| sett.is_enabled))
            {
                let now = now().as_secs();
                if let FiatMessage::GetPrice = fiat_msg {
                    // return self.global_cache.handle_fiat_message(i, &pane, tab_id, fiat);
                    println!("Tab id {} requested fiat price", tab_id);
                    println!("Tab fiat price enabled");

                    // If there's already a cached price no older than the update interval,
                    // return it to the specific tab that requested it.
                    if let Some(cached) = self
                        .fiat_prices
                        .prices
                        .get(&(price_setting.source, price_setting.currency))
                        .as_ref()
                        .filter(|req| req.requested_at() + FIAT_PRICE_UPDATE_INTERVAL_SECS > now)
                    {
                        if tab
                            .cache()
                            .and_then(|c| c.fiat_price.as_ref())
                            .is_some_and(|p| {
                                p.source() == cached.source()
                                    && p.currency() == cached.currency()
                                    && p.requested_at() == cached.requested_at()
                            })
                        {
                            tracing::info!(
                                "Tab already has fiat price for {} from {}",
                                cached.currency(),
                                cached.source(),
                            );
                            return Task::none();
                        }
                        // Return cached price to the tab that requested it.
                        tracing::info!(
                            "Returning cached fiat price for {} from {} to tab",
                            cached.currency(),
                            cached.source(),
                        );
                        return pane
                            .update_tab_with_fiat(
                                tab_id,
                                FiatMessage::GetPriceResult((*cached).clone()),
                                config,
                            )
                            .map(move |msg| Message::Pane(pane_id, msg));
                    }
                    // Make sure there is not a pending request.
                    // Do nothing if the last request was recent and was for the same source & currency, where
                    // "recent" means within half the update interval.
                    // Using half the update interval is sufficient as we are mostly concerned with preventing
                    // multiple requests being sent within seconds of each other (e.g. after the GUI window is
                    // inactive for an extended period). Using the full update interval could lead to a kind
                    // of race condition and cause a regular subscription message to be missed.
                    if self
                        .fiat_prices
                        .last_requests
                        .get(&(price_setting.source, price_setting.currency))
                        .as_ref()
                        .filter(|req| req.timestamp + FIAT_PRICE_UPDATE_INTERVAL_SECS / 2 > now)
                        .is_some()
                    {
                        // Cached request is still valid, no need to fetch a new one.
                        tracing::info!(
                            "Fiat price for {} from {} has been requested recently",
                            price_setting.currency,
                            price_setting.source,
                        );
                        return Task::none();
                    }
                    let new_request = cache::FiatPriceRequest {
                        source: price_setting.source,
                        currency: price_setting.currency,
                        timestamp: now,
                    };
                    self.fiat_prices.last_requests.insert(
                        (new_request.source, new_request.currency),
                        new_request.clone(),
                    );
                    tracing::info!(
                        "Getting fiat price in {} from {}",
                        price_setting.currency,
                        price_setting.source,
                    );
                    return Task::perform(
                        async move { new_request.send_default().await },
                        Message::GetFiatPriceResult,
                    );
                } else if let FiatMessage::ListCurrencies(source) = fiat_msg {
                    println!("Tab requested currencies");
                    match self.fiat_prices.currencies.get(&source) {
                        Some((old, list))
                            if now.saturating_sub(*old) <= CURRENCIES_LIST_TTL_SECS =>
                        {
                            return pane
                                .update_tab_with_fiat(
                                    tab_id,
                                    FiatMessage::ListCurrenciesResult(
                                        source,
                                        *old,
                                        Ok(ListCurrenciesResult {
                                            currencies: list.clone(),
                                        }),
                                    ),
                                    config,
                                )
                                .map(move |msg| Message::Pane(pane_id, msg));
                        }
                        _ => {
                            // return the full message and handle below
                            return Task::perform(
                                async move {
                                    let client = PriceClient::default_from_source(source);
                                    (tab_id, source, now, client.list_currencies().await)
                                },
                                move |(tab_id, source, now, res)| {
                                    Message::Pane(
                                        pane_id,
                                        pane::Message::Tab(
                                            tab_id,
                                            tab::Message::Run(Box::new(app::Message::Fiat(
                                                FiatMessage::ListCurrenciesResult(source, now, res),
                                            ))),
                                        ),
                                    )
                                },
                            );
                        }
                    }
                } else if let FiatMessage::ListCurrenciesResult(
                    source,
                    requested_at,
                    Ok(ref list),
                ) = fiat_msg
                {
                    tracing::debug!(
                        "Updating currencies list for source '{}' as requested at {}.",
                        source,
                        requested_at,
                    );
                    self.fiat_prices
                        .currencies
                        .insert(source, (requested_at, list.currencies.clone()));
                }
            }
        }
        pane.update_tab_with_fiat(tab_id, fiat_msg, config)
            .map(move |msg| Message::Pane(pane_id, msg))
    }
}

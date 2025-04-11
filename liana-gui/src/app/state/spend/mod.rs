mod step;

use std::collections::HashSet;
use std::convert::TryInto;
use std::sync::Arc;

use iced::Task;

use liana::miniscript::bitcoin::{Network, OutPoint};
use liana_ui::widget::Element;
use lianad::commands::CoinStatus;

use super::{redirect, State};
use crate::{
    app::{cache::Cache, error::Error, menu::Menu, message::Message, view, wallet::Wallet},
    daemon::{
        model::{Coin, LabelItem},
        Daemon,
    },
};

pub struct CreateSpendPanel {
    draft: step::TransactionDraft,
    /// The timelock of the recovery path to use for spending.
    /// If `None`, the primary path will be used.
    ///
    /// For a given instance of `CreateSpendPanel`, this value must either
    /// always be set or otherwise remain `None`. If set, the value can
    /// change from one recovery timelock to another.
    recovery_timelock: Option<u16>,
    current: usize,
    steps: Vec<Box<dyn step::Step>>,
}

impl CreateSpendPanel {
    pub fn new(wallet: Arc<Wallet>, coins: &[Coin], blockheight: u32, network: Network) -> Self {
        let descriptor = wallet.main_descriptor.clone();
        Self {
            draft: step::TransactionDraft::new(network),
            recovery_timelock: None,
            current: 0,
            steps: vec![
                Box::new(
                    step::DefineSpend::new(network, descriptor, coins, blockheight, None, true)
                        .with_coins_sorted(blockheight),
                ),
                Box::new(step::SaveSpend::new(wallet)),
            ],
        }
    }

    /// Create a new instance to be used for a recovery spend.
    ///
    /// By default, the wallet's first timelock value is used for `DefineSpend`.
    pub fn new_recovery(
        wallet: Arc<Wallet>,
        coins: &[Coin],
        blockheight: u32,
        network: Network,
    ) -> Self {
        let descriptor = wallet.main_descriptor.clone();
        let timelock = descriptor.first_timelock_value();
        Self {
            draft: step::TransactionDraft::new(network),
            recovery_timelock: None,
            current: 0,
            steps: vec![
                Box::new(step::SelectRecoveryPath::new(
                    wallet.clone(),
                    coins,
                    blockheight.try_into().expect("i32 by consensus"),
                )),
                Box::new(
                    step::DefineSpend::new(
                        network,
                        descriptor,
                        coins,
                        blockheight,
                        Some(timelock),
                        false,
                    )
                    .with_coins_sorted(blockheight),
                ),
                Box::new(step::SaveSpend::new(wallet)),
            ],
        }
    }

    pub fn new_self_send(
        wallet: Arc<Wallet>,
        coins: &[Coin],
        blockheight: u32,
        preselected_coins: &[OutPoint],
        network: Network,
    ) -> Self {
        let descriptor = wallet.main_descriptor.clone();
        Self {
            draft: step::TransactionDraft::new(network),
            recovery_timelock: None,
            current: 0,
            steps: vec![
                Box::new(
                    step::DefineSpend::new(network, descriptor, coins, blockheight, None, true)
                        .with_preselected_coins(preselected_coins)
                        .with_coins_sorted(blockheight)
                        .self_send(),
                ),
                Box::new(step::SaveSpend::new(wallet)),
            ],
        }
    }

    pub fn keep_state(&self) -> bool {
        if self.recovery_timelock.is_some() {
            // retain the state if user is on the first 2 steps
            // (choosing recovery path and defining spend)
            self.current < 2
        } else {
            self.current == 0
        }
    }
}

impl State for CreateSpendPanel {
    fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, view::Message> {
        self.steps.get(self.current).unwrap().view(cache)
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        self.steps.get(self.current).unwrap().subscription()
    }

    fn interrupt(&mut self) {
        self.steps.get_mut(self.current).unwrap().interrupt();
    }

    fn update(
        &mut self,
        daemon: Arc<dyn Daemon + Sync + Send>,
        cache: &Cache,
        message: Message,
    ) -> Task<Message> {
        if matches!(message, Message::View(view::Message::Close)) {
            return redirect(Menu::PSBTs);
        }

        if matches!(message, Message::View(view::Message::Next)) {
            if let Some(step) = self.steps.get(self.current) {
                step.apply(&mut self.draft, &mut self.recovery_timelock);
            }

            if let Some(step) = self.steps.get_mut(self.current + 1) {
                self.current += 1;
                step.load(cache, &self.draft, self.recovery_timelock);
            }
        }

        if matches!(message, Message::View(view::Message::Previous))
            && self.steps.get(self.current - 1).is_some()
        {
            self.current -= 1;
        }

        if let Some(step) = self.steps.get_mut(self.current) {
            return step.update(daemon, cache, message);
        }

        Task::none()
    }

    fn reload(
        &mut self,
        daemon: Arc<dyn Daemon + Sync + Send>,
        _wallet: Arc<Wallet>,
    ) -> Task<Message> {
        let daemon1 = daemon.clone();
        let daemon2 = daemon.clone();
        Task::batch(vec![
            Task::perform(
                async move {
                    daemon1
                        .list_coins(&[CoinStatus::Unconfirmed, CoinStatus::Confirmed], &[])
                        .await
                        .map(|res| res.coins)
                        .map_err(|e| e.into())
                },
                Message::Coins,
            ),
            Task::perform(
                async move {
                    let coins = daemon
                        .list_coins(&[CoinStatus::Unconfirmed, CoinStatus::Confirmed], &[])
                        .await
                        .map(|res| res.coins)
                        .map_err(Error::from)?;
                    let mut targets = HashSet::<LabelItem>::new();
                    for coin in coins {
                        targets.insert(LabelItem::OutPoint(coin.outpoint));
                        targets.insert(LabelItem::Txid(coin.outpoint.txid));
                    }
                    daemon2.get_labels(&targets).await.map_err(|e| e.into())
                },
                Message::Labels,
            ),
        ])
    }
}

impl From<CreateSpendPanel> for Box<dyn State> {
    fn from(s: CreateSpendPanel) -> Box<dyn State> {
        Box::new(s)
    }
}

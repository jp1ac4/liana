use std::collections::HashMap;

use bdk_electrum::bdk_chain::{
    bitcoin::{self, bip32::ChildNumber, BlockHash, OutPoint},
    local_chain::LocalChain,
    spk_client::{FullScanRequest, SyncRequest},
    ChainPosition,
};

pub mod client;
mod utils;
pub mod wallet;
use crate::bitcoin::{Block, BlockChainTip, Coin};

/// An error in the Electrum interface.
#[derive(Debug)]
pub enum ElectrumError {
    Client(client::Error),
    GenesisHashMismatch(
        BlockHash, /*expected hash*/
        BlockHash, /*server hash*/
        BlockHash, /*wallet hash*/
    ),
}

impl std::fmt::Display for ElectrumError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ElectrumError::Client(e) => write!(f, "Electrum client error: '{}'.", e),
            ElectrumError::GenesisHashMismatch(expected, server, wallet) => {
                write!(
                    f,
                    "Genesis hash mismatch. The genesis hash is expected to be '{}'. \
                    The server has hash '{}' and the wallet has hash '{}'.",
                    expected, server, wallet,
                )
            }
        }
    }
}

/// Interface for Electrum backend.
pub struct Electrum {
    client: client::Client,
    bdk_wallet: wallet::BdkWallet,
    /// Used for setting the `last_seen` of unconfirmed transactions in a strictly
    /// increasing manner.
    sync_count: u64,
}

impl Electrum {
    pub fn new(
        client: client::Client,
        bdk_wallet: wallet::BdkWallet,
    ) -> Result<Self, ElectrumError> {
        Ok(Self {
            client,
            bdk_wallet,
            sync_count: 0,
        })
    }

    pub fn sanity_checks(&self, expected_hash: &bitcoin::BlockHash) -> Result<(), ElectrumError> {
        let server_hash = self.client.genesis_block().hash;
        let wallet_hash = self.bdk_wallet.local_chain().genesis_hash();
        if server_hash != *expected_hash || wallet_hash != *expected_hash {
            return Err(ElectrumError::GenesisHashMismatch(
                *expected_hash,
                server_hash,
                wallet_hash,
            ));
        }
        Ok(())
    }

    pub fn client(&self) -> &client::Client {
        &self.client
    }

    fn local_chain(&self) -> &LocalChain {
        self.bdk_wallet.local_chain()
    }

    /// Get all coins stored in the wallet, taking into consideration only those unconfirmed
    /// transactions that were seen in the last wallet sync.
    pub fn wallet_coins(&self, outpoints: Option<&[OutPoint]>) -> HashMap<OutPoint, Coin> {
        self.bdk_wallet.coins(outpoints, Some(self.sync_count))
    }

    pub fn rollback_wallet_tip(&mut self, new_tip: &BlockChainTip) {
        self.bdk_wallet.rollback_tip(new_tip)
    }

    /// Sync the wallet with the Electrum server.
    pub fn sync_wallet(
        &mut self,
        receive_index: ChildNumber,
        change_index: ChildNumber,
    ) -> Result<(), ElectrumError> {
        self.bdk_wallet.reveal_spks(receive_index, change_index);
        let local_chain_tip = self.local_chain().tip();
        log::debug!(
            "local chain tip height before sync with electrum: {}",
            local_chain_tip.block_id().height
        );

        const BATCH_SIZE: usize = 200;
        // We'll only need to calculate fees of mempool transactions and this will be done separately from our graph
        // so we don't need to fetch prev txouts. In any case, we'll already have these for our own transactions.
        const FETCH_PREV_TXOUTS: bool = false;
        const STOP_GAP: usize = 50;

        let (chain_update, mut graph_update, keychain_update) = if local_chain_tip.height() > 0 {
            log::info!("Performing sync.");
            let mut request = SyncRequest::from_chain_tip(local_chain_tip.clone())
                .cache_graph_txs(self.bdk_wallet.graph());

            let all_spks: Vec<_> = self
                .bdk_wallet
                .index()
                .inner() // we include lookahead SPKs
                .all_spks()
                .iter()
                .map(|(_, script)| script.clone())
                .collect();
            request = request.chain_spks(all_spks);
            log::debug!("num SPKs for sync: {}", request.spks.len());

            let sync_result = self
                .client
                .sync_with_confirmation_time_height_anchor(request, BATCH_SIZE, FETCH_PREV_TXOUTS)
                .map_err(ElectrumError::Client)?;
            (sync_result.chain_update, sync_result.graph_update, None)
        } else {
            log::info!("Performing full scan.");
            let mut request = FullScanRequest::from_chain_tip(local_chain_tip.clone())
                .cache_graph_txs(self.bdk_wallet.graph());

            for (k, spks) in self.bdk_wallet.index().all_unbounded_spk_iters() {
                request = request.set_spks_for_keychain(k, spks);
            }
            let scan_result = self
                .client
                .full_scan_with_confirmation_time_height_anchor(
                    request,
                    STOP_GAP,
                    BATCH_SIZE,
                    FETCH_PREV_TXOUTS,
                )
                .map_err(ElectrumError::Client)?;
            (
                scan_result.chain_update,
                scan_result.graph_update,
                Some(scan_result.last_active_indices),
            )
        };
        log::debug!(
            "chain update height after sync with electrum: {}",
            chain_update.height()
        );
        // Make sure there has not been a reorg since local chain was last updated.
        match chain_update.get(local_chain_tip.height()) {
            Some(cp) => {
                if cp.hash() != local_chain_tip.hash() {
                    log::debug!(
                        "hash for current local tip would change from {} to {}",
                        local_chain_tip.hash(),
                        chain_update.hash()
                    );
                    return Ok(());
                }
            }
            None => {
                log::debug!(
                    "new chain tip would have lower height {} and hash: {}",
                    chain_update.height(),
                    chain_update.hash(),
                );
                return Ok(());
            }
        }
        // No reorg detected, so we increment the sync count and apply changes.
        self.sync_count = self.sync_count.checked_add(1).expect("must fit");
        if let Some(keychain_update) = keychain_update {
            self.bdk_wallet.apply_keychain_update(keychain_update);
        }
        self.bdk_wallet.apply_connected_chain_update(chain_update);

        // Unconfirmed transactions have their last seen as 0, so we override to the `sync_count`
        // so that conflicts can be properly handled. We use `sync_count` instead of current time
        // in seconds to ensure strictly increasing values between poller iterations.
        for tx in &graph_update.initial_changeset().txs {
            let txid = tx.txid();
            if let Some(ChainPosition::Unconfirmed(_)) = graph_update.get_chain_position(
                self.local_chain(),
                self.local_chain().tip().block_id(),
                txid,
            ) {
                log::debug!(
                    "changing last seen for txid '{}' to {}",
                    txid,
                    self.sync_count
                );
                let _ = graph_update.insert_seen_at(txid, self.sync_count);
            }
        }
        self.bdk_wallet.apply_graph_update(graph_update);
        Ok(())
    }

    /// Get the block in our wallet's `local_chain` that `tip` has in common with the Electrum server.
    pub fn common_ancestor(&self, tip: &BlockChainTip) -> Option<BlockChainTip> {
        self.client.common_ancestor(self.local_chain(), tip)
    }

    pub fn wallet_transaction(
        &self,
        txid: &bitcoin::Txid,
    ) -> Option<(bitcoin::Transaction, Option<Block>)> {
        self.bdk_wallet.get_transaction(txid)
    }
}

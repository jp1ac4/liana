use std::collections::HashMap;

use bdk_chain::{
    bitcoin::{self, bip32::ChildNumber, BlockHash, OutPoint},
    keychain_txout::SyncRequestBuilderExt,
    local_chain::LocalChain,
    spk_client::{FullScanRequest, SyncRequest},
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
    /// Set to `true` to force a full scan from the genesis block regardless of
    /// the wallet's local chain height.
    full_scan: bool,
}

impl Electrum {
    pub fn new(
        client: client::Client,
        bdk_wallet: wallet::BdkWallet,
        full_scan: bool,
    ) -> Result<Self, ElectrumError> {
        Ok(Self {
            client,
            bdk_wallet,
            sync_count: 0,
            full_scan,
        })
    }

    pub fn sanity_checks(&self, expected_hash: &bitcoin::BlockHash) -> Result<(), ElectrumError> {
        let server_hash = self
            .client
            .genesis_block()
            .map_err(ElectrumError::Client)?
            .hash;
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

    /// Get the tip of the wallet's local chain.
    pub fn wallet_tip(&self) -> BlockChainTip {
        utils::tip_from_block_id(self.local_chain().tip().block_id())
    }

    /// Whether `tip` exists in the wallet's `local_chain`.
    ///
    /// Returns `None` if no block at that height exists in `local_chain`.
    pub fn is_in_wallet_chain(&self, tip: BlockChainTip) -> Option<bool> {
        self.bdk_wallet.is_in_chain(tip)
    }

    /// Whether we'll perform a full scan at the next poll.
    pub fn is_rescanning(&self) -> bool {
        self.full_scan || self.local_chain().tip().height() == 0
    }

    /// Make the poller perform a full scan on the next iteration.
    pub fn trigger_rescan(&mut self) {
        self.full_scan = true;
    }

    /// Sync the wallet with the Electrum server. If there was any reorg since the last poll, this
    /// returns the first common ancestor between the previous and the new chain.
    pub fn sync_wallet(
        &mut self,
        receive_index: ChildNumber,
        change_index: ChildNumber,
    ) -> Result<Option<BlockChainTip>, ElectrumError> {
        self.bdk_wallet.reveal_spks(receive_index, change_index);
        let local_chain_tip = self.local_chain().tip();
        log::debug!(
            "local chain tip height before sync with electrum: {}",
            local_chain_tip.block_id().height
        );

        // We'll only need to calculate fees of mempool transactions and this will be done separately from our graph
        // so we don't need to fetch prev txouts. In any case, we'll already have these for our own transactions.
        const FETCH_PREV_TXOUTS: bool = false;
        const STOP_GAP: usize = 200;

        // TODO: cache only the new txs, perhaps after the sync/scan.
        self.client
            .populate_tx_cache(self.bdk_wallet.graph().full_txs().map(|node| node.tx));
        let (chain_update, tx_update, keychain_update) = if !self.is_rescanning() {
            log::debug!("Performing sync.");
            let mut request = SyncRequest::builder().chain_tip(local_chain_tip.clone());
            request = request.revealed_spks_from_indexer(self.bdk_wallet.index(), ..);
            // Include lookahead SPKs, e.g. in case they have been revealed by another wallet participant.
            for (k, _) in self.bdk_wallet.index().keychains() {
                let (next, _) = self
                    .bdk_wallet
                    .index()
                    .next_index(k)
                    .expect("keychain exists");
                log::debug!(
                    "keychain={:?} next={} lookahead={}",
                    k,
                    next,
                    self.bdk_wallet.index().lookahead()
                );
                let lookahead_spks = (0..self.bdk_wallet.index().lookahead()).map(|i| {
                    let lookahead_idx = next + i;
                    let s = self
                        .bdk_wallet
                        .index()
                        .spk_at_index(k, lookahead_idx)
                        .expect("lookahead index has been inserted");
                    ((k, lookahead_idx), s)
                });
                request = request.spks_with_indexes(lookahead_spks);
            }
            let sync_result = self
                .client
                .sync(request.build(), FETCH_PREV_TXOUTS)
                .map_err(ElectrumError::Client)?;
            log::debug!("Sync complete.");
            (sync_result.chain_update, sync_result.tx_update, None)
        } else {
            log::info!("Performing full scan.");
            // Either local_chain has height 0 or we want to trigger a full scan.
            let mut request = FullScanRequest::builder().chain_tip(local_chain_tip.clone());

            for (k, spks) in self.bdk_wallet.index().all_unbounded_spk_iters() {
                request = request.spks_for_keychain(k, spks);
            }
            let scan_result = self
                .client
                .full_scan(request.build(), STOP_GAP, FETCH_PREV_TXOUTS)
                .map_err(ElectrumError::Client)?;
            // A full scan only makes sense to do once, in most cases. Don't do it again unless
            // explicitly asked to by a user.
            self.full_scan = false;
            log::info!("Full scan complete.");
            (
                scan_result.chain_update,
                scan_result.tx_update,
                Some(scan_result.last_active_indices),
            )
        };
        let chain_update = chain_update.expect("request included chain tip");
        log::debug!(
            "chain update height after sync with electrum: {}",
            chain_update.height()
        );

        log::debug!("Full local chain: {:?}", self.local_chain());
        log::debug!("Full chain update: {:?}", chain_update);

        // Increment the sync count and apply changes.
        self.sync_count = self.sync_count.checked_add(1).expect("must fit");
        if let Some(keychain_update) = keychain_update {
            self.bdk_wallet.apply_keychain_update(keychain_update);
        }
        let changeset = self.bdk_wallet.apply_connected_chain_update(chain_update);

        // Look for updated/invalidated blocks at or below our height before syncing.
        // Since we iterate in ascending height order, we'll see the lowest block height first.
        // Note that the changeset after the first sync may include new blocks below the tip,
        // e.g. corresponding to confirmation blocks of any existing wallet coins and the most
        // recent ~8 blocks below the tip, but such new blocks don't imply a reorg.
        let reorg_common_ancestor = changeset
            .blocks
            .into_iter()
            .filter(|(height, _)| height <= &local_chain_tip.height())
            .find_map(|(height, _)| {
                // If our local chain already contains a block at this height, then either the block hash
                // has been updated or the block has been invalidated, so it's a reorg.
                local_chain_tip.get(height).map(|_| {
                    log::info!("Block chain reorganization detected.");
                    // Return the block in our local chain before this height.
                    // We must only consider blocks that were in our local chain *before* the sync
                    // in order to make sure we update all affected coins in the DB.
                    local_chain_tip
                        .iter() // in descending height order
                        .find(|cp| cp.height() < height)
                        .map(|cp| BlockChainTip {
                            height: utils::height_i32_from_u32(cp.height()),
                            hash: cp.hash(),
                        })
                        .expect("height > 0 and local chain contains genesis")
                })
            });

        // We use `sync_count` instead of current time in seconds for `seen_at`
        // to ensure strictly increasing values between poller iterations.
        self.bdk_wallet
            .apply_graph_update_at(tx_update, Some(self.sync_count));
        Ok(reorg_common_ancestor)
    }

    pub fn wallet_transaction(
        &self,
        txid: &bitcoin::Txid,
    ) -> Option<(bitcoin::Transaction, Option<Block>)> {
        self.bdk_wallet.get_transaction(txid)
    }
}

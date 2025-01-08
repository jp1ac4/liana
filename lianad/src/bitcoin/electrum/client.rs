use std::{collections::HashSet, convert::TryInto, sync::Arc};

use bdk_chain::{
    local_chain::{CheckPoint, LocalChain},
    TxGraph,
};
use bdk_electrum::{
    bdk_core::{
        bitcoin,
        spk_client::{FullScanRequest, FullScanResponse, SyncRequest, SyncResponse},
        BlockId,
    },
    electrum_client::{self, Config, ElectrumApi},
    BdkElectrumClient,
};

use super::utils::{
    block_id_from_tip, height_i32_from_usize, height_usize_from_i32, outpoints_from_tx,
};
use crate::{
    bitcoin::{electrum::utils::tip_from_block_id, BlockChainTip, MempoolEntry, MempoolEntryFees},
    config,
};

// Default batch size to use when making requests to the Electrum server.
const DEFAULT_BATCH_SIZE: usize = 200;

// If Electrum takes more than 3 minutes to answer one of our queries, fail.
const RPC_SOCKET_TIMEOUT: u8 = 180;

// Number of retries while communicating with the Electrum server.
// A retry happens with exponential back-off (base 2) so this makes us give up after (1+2+4+8+16+32=) 63 seconds.
const RETRY_LIMIT: u8 = 6;

/// An error in the Electrum client.
#[derive(Debug)]
pub enum Error {
    Server(electrum_client::Error),
    TipChanged(BlockId, BlockId),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Server(e) => write!(f, "Electrum error: '{}'.", e),
            Error::TipChanged(expected, actual) => write!(
                f,
                "Electrum error: Expected tip '{}' but actual tip was {}.",
                tip_from_block_id(*expected),
                tip_from_block_id(*actual),
            ),
        }
    }
}

pub struct Client(BdkElectrumClient<electrum_client::Client>);

impl Client {
    /// Create a new client and perform sanity checks.
    pub fn new(electrum_config: &config::ElectrumConfig) -> Result<Self, Error> {
        // First use a dummy config to check connectivity (no retries, short timeout).
        let dummy_config = Config::builder().retry(0).timeout(Some(3)).build();
        // Try to ping the server.
        bdk_electrum::electrum_client::Client::from_config(&electrum_config.addr, dummy_config)
            .and_then(|dummy_client| dummy_client.ping())
            .map_err(Error::Server)?;

        // Now connection has been checked, create client with required retries and timeout.
        let config = Config::builder()
            .retry(RETRY_LIMIT)
            .timeout(Some(RPC_SOCKET_TIMEOUT))
            .build();
        let client =
            bdk_electrum::electrum_client::Client::from_config(&electrum_config.addr, config)
                .map_err(Error::Server)?;
        let bdk_electrum_client = BdkElectrumClient::new(client);
        Ok(Self(bdk_electrum_client))
    }

    pub fn chain_tip(&self) -> Result<BlockChainTip, Error> {
        self.0
            .inner
            .block_headers_subscribe()
            .map_err(Error::Server)
            .map(|notif| BlockChainTip {
                height: height_i32_from_usize(notif.height),
                hash: notif.header.block_hash(),
            })
    }

    fn genesis_block_header(&self) -> Result<bitcoin::block::Header, Error> {
        self.0.inner.block_header(0).map_err(Error::Server)
    }

    pub fn genesis_block_timestamp(&self) -> Result<u32, Error> {
        self.genesis_block_header().map(|header| header.time)
    }

    pub fn genesis_block(&self) -> Result<BlockChainTip, Error> {
        self.genesis_block_header().map(|header| BlockChainTip {
            hash: header.block_hash(),
            height: 0,
        })
    }

    pub fn broadcast_tx(&self, tx: &bitcoin::Transaction) -> Result<bitcoin::Txid, Error> {
        self.0.transaction_broadcast(tx).map_err(Error::Server)
    }

    pub fn tip_time(&self) -> Result<u32, Error> {
        let tip_height = self.chain_tip()?.height;
        self.0
            .inner
            .block_header(height_usize_from_i32(tip_height))
            .map_err(Error::Server)
            .map(|bh| bh.time)
    }

    pub fn populate_tx_cache(
        &self,
        txs: impl IntoIterator<Item = impl Into<Arc<bitcoin::Transaction>>>,
    ) {
        self.0.populate_tx_cache(txs)
    }

    /// Perform the given `SyncRequest`.
    pub fn sync<I: 'static>(
        &self,
        request: SyncRequest<I>,
        fetch_prev_txouts: bool,
    ) -> Result<SyncResponse, Error> {
        self.0
            .sync::<I>(request, DEFAULT_BATCH_SIZE, fetch_prev_txouts)
            .map_err(Error::Server)
    }

    /// Perform the given `FullScanRequest`.
    pub fn full_scan<K: Ord + Clone>(
        &self,
        request: FullScanRequest<K>,
        stop_gap: usize,
        fetch_prev_txouts: bool,
    ) -> Result<FullScanResponse<K>, Error> {
        self.0
            .full_scan(request, stop_gap, DEFAULT_BATCH_SIZE, fetch_prev_txouts)
            .map_err(Error::Server)
    }

    /// Get mempool entries.
    ///
    /// If `expected_tip` is specified, the function will return `Error::TipChanged` if the chain tip
    /// changes while the entries are being found. Otherwise, the function will restart in case the
    /// chain tip changes before completion.
    fn mempool_entries(
        &self,
        txids: HashSet<bitcoin::Txid>,
        expected_tip: Option<CheckPoint>,
    ) -> Result<Vec<MempoolEntry>, Error> {
        log::debug!("Getting mempool entries for txids '{:?}'.", txids);
        let mut graph = TxGraph::default();
        let mut local_chain = LocalChain::from_genesis_hash(self.genesis_block()?.hash).0;
        let tip_block = if let Some(ref expected_tip) = expected_tip {
            expected_tip.block_id()
        } else {
            block_id_from_tip(self.chain_tip()?)
        };
        if tip_block.height > 0 {
            let _ = local_chain
                .insert_block(tip_block)
                .expect("only contains genesis block");
        }
        // First, get the tx itself and check it's unconfirmed.
        let request = SyncRequest::<()>::builder()
            .chain_tip(local_chain.tip())
            .txids(txids.clone())
            .build();
        // We'll get prev txouts for this tx when we find its ancestors below.
        let sync_result = self.sync(request, false)?;
        let _ = local_chain.apply_update(
            sync_result
                .chain_update
                .expect("request included chain tip"),
        );
        // Store local tip after first sync. This will be our reference tip.
        let local_tip_block_id = local_chain.tip().block_id();
        if let Some(ref expected_tip) = expected_tip {
            if expected_tip.block_id() != local_tip_block_id {
                return Err(Error::TipChanged(
                    expected_tip.block_id(),
                    local_tip_block_id,
                ));
            }
        }
        let mut desc_ops = Vec::new();
        let mut txs = Vec::new();
        let _ = graph.apply_update(sync_result.tx_update);
        graph
            .list_canonical_txs(&local_chain, local_chain.tip().block_id())
            .for_each(|canon_tx| {
                if !canon_tx.chain_position.is_confirmed() && txids.contains(&canon_tx.tx_node.txid)
                {
                    desc_ops.extend(outpoints_from_tx(&canon_tx.tx_node.tx));
                    txs.push(canon_tx.tx_node.tx);
                }
            });
        // Now iterate over increasing depths of descendants.
        // As they are descendants, we can assume they are all unconfirmed.
        while !desc_ops.is_empty() {
            log::debug!("Syncing descendant outpoints: {:?}", desc_ops);
            self.0
                .populate_tx_cache(graph.full_txs().map(|node| node.tx));
            let request = SyncRequest::<()>::builder()
                .chain_tip(local_chain.tip())
                .outpoints(desc_ops.clone())
                .build();
            // Fetch prev txouts to ensure we have all required txs in the graph to calculate fees.
            // An unconfirmed descendant may have a confirmed parent that we wouldn't have in our graph.
            let sync_result = self.sync(request, true)?;
            let _ = local_chain.apply_update(
                sync_result
                    .chain_update
                    .expect("request included chain tip"),
            );
            if let Some(ref expected_tip) = expected_tip {
                if expected_tip.block_id() != local_chain.tip().block_id() {
                    return Err(Error::TipChanged(
                        expected_tip.block_id(),
                        local_chain.tip().block_id(),
                    ));
                }
            }
            if local_chain.tip().block_id() != local_tip_block_id {
                log::debug!("Chain tip changed while getting mempool entry. Restarting.");
                return self.mempool_entries(txids, expected_tip.clone());
            }
            let _ = graph.apply_update(sync_result.tx_update);
            // Get any txids spending the outpoints we've just synced against.
            let desc_txids: HashSet<_> = graph
                .filter_chain_txouts(
                    &local_chain,
                    local_chain.tip().block_id(),
                    desc_ops.iter().map(|op| ((), *op)),
                )
                .filter_map(|(_, txout)| txout.spent_by.map(|(_, spend_txid)| spend_txid))
                .collect();
            desc_ops = desc_txids
                .iter()
                .flat_map(|txid| {
                    let desc_tx = graph
                        .get_tx(*txid)
                        .expect("we must have tx in graph after sync");
                    outpoints_from_tx(&desc_tx)
                })
                .collect();
        }

        // For each unconfirmed transaction, starting with `txid`, get its direct ancestors, which may be confirmed or unconfirmed.
        // Continue until there are no more unconfirmed ancestors.
        // Confirmed transactions will be filtered out from `anc_txids` later on.
        let mut anc_txids: HashSet<_> = txs
            .iter()
            .flat_map(|tx| tx.input.iter().map(|txin| txin.previous_output.txid))
            .collect();
        while !anc_txids.is_empty() {
            log::debug!("Syncing ancestor txids: {:?}", anc_txids);
            self.0
                .populate_tx_cache(graph.full_txs().map(|node| node.tx));
            let request = SyncRequest::<()>::builder()
                .chain_tip(local_chain.tip())
                .txids(anc_txids.clone())
                .build();
            // We expect to have prev txouts for all unconfirmed ancestors in our graph so no need to fetch them here.
            // Note we keep iterating through ancestors until we find one that is confirmed and only need to calculate
            // fees for unconfirmed transactions.
            let sync_result = self.sync(request, false)?;
            let _ = local_chain.apply_update(
                sync_result
                    .chain_update
                    .expect("request included chain tip"),
            );
            if let Some(expected_tip) = &expected_tip {
                if expected_tip.block_id() != local_chain.tip().block_id() {
                    return Err(Error::TipChanged(
                        expected_tip.block_id(),
                        local_chain.tip().block_id(),
                    ));
                }
            }
            if local_chain.tip().block_id() != local_tip_block_id {
                log::debug!("Chain tip changed while getting mempool entry. Restarting.");
                return self.mempool_entries(txids, expected_tip);
            }
            let _ = graph.apply_update(sync_result.tx_update);

            // Add ancestors of any unconfirmed txs.
            anc_txids = graph
                .list_canonical_txs(&local_chain, local_chain.tip().block_id())
                .filter_map(|canon_tx| {
                    (!canon_tx.chain_position.is_confirmed()
                        && anc_txids.contains(&canon_tx.tx_node.txid))
                    .then_some(
                        canon_tx
                            .tx_node
                            .tx
                            .input
                            .iter()
                            .map(|txin| txin.previous_output.txid)
                            .collect::<HashSet<_>>(),
                    )
                })
                .flatten()
                .collect();
        }
        let mut entries = Vec::new();
        for tx in txs {
            // Now iterate over ancestors and descendants in the graph.
            let base_fee = graph
                .calculate_fee(&tx)
                .expect("all required txs are in graph");
            let base_size = tx.vsize();
            // Ancestor & descendant fees include those of `txid`.
            let mut desc_fees = base_fee;
            let mut anc_fees = base_fee;
            // Ancestor size includes that of `txid`.
            let mut anc_size = base_size;
            for desc_txid in
                graph.walk_descendants(tx.compute_txid(), |_, desc_txid| Some(desc_txid))
            {
                log::debug!("Getting fee for desc txid '{}'.", desc_txid);
                let desc_tx = graph
                    .get_tx(desc_txid)
                    .expect("all descendant txs are in graph");
                let fee = graph
                    .calculate_fee(&desc_tx)
                    .expect("all required txs are in graph");
                desc_fees += fee;
            }
            for anc_tx in graph.walk_ancestors(tx, |_, anc_tx| Some(anc_tx)) {
                let anc_txid = anc_tx.compute_txid();
                log::debug!("Getting fee and size for anc txid '{}'.", anc_txid);
                if graph
                    .list_canonical_txs(&local_chain, local_chain.tip().block_id())
                    .find(|canon_tx| canon_tx.tx_node.txid == anc_txid)
                    .filter(|canon_tx| !canon_tx.chain_position.is_confirmed())
                    .is_some()
                {
                    let fee = graph
                        .calculate_fee(&anc_tx)
                        .expect("all required txs are in graph");
                    anc_fees += fee;
                    anc_size += anc_tx.vsize();
                } else {
                    log::debug!("Ancestor txid '{}' is not unconfirmed.", anc_txid);
                    continue;
                }
            }
            let fees = MempoolEntryFees {
                base: base_fee,
                ancestor: anc_fees,
                descendant: desc_fees,
            };
            let entry = MempoolEntry {
                vsize: base_size.try_into().expect("tx size must fit into u64"),
                fees,
                ancestor_vsize: anc_size.try_into().expect("tx size must fit into u64"),
            };
            entries.push(entry)
        }

        // It's possible that the chain tip has now changed, but it hadn't done as of the last sync,
        // so go ahead and return the results.
        Ok(entries)
    }

    /// Get mempool entry for a single `txid`.
    ///
    /// Convenience method to call `mempool_entries` for a single `txid`,
    /// returning `Option` instead of `Vec`.
    pub fn mempool_entry(&self, txid: &bitcoin::Txid) -> Result<Option<MempoolEntry>, Error> {
        // We just require the chain tip to stay the same while running `mempool_entries` so
        // don't need to pass in an expected tip.
        self.mempool_entries(HashSet::from([*txid]), None)
            .map(|entries| entries.first().cloned())
    }

    /// Get mempool spenders of the given outpoints.
    ///
    /// Will restart if chain tip changes before completion.
    pub fn mempool_spenders(
        &self,
        outpoints: &[bitcoin::OutPoint],
    ) -> Result<Vec<MempoolEntry>, Error> {
        log::debug!("Getting mempool spenders for outpoints: {:?}.", outpoints);
        let mut local_chain = LocalChain::from_genesis_hash(self.genesis_block()?.hash).0;
        let chain_tip = self.chain_tip()?;
        if chain_tip.height > 0 {
            let _ = local_chain
                .insert_block(block_id_from_tip(chain_tip))
                .expect("only contains genesis block");
        }
        let request = SyncRequest::<()>::builder()
            .chain_tip(local_chain.tip())
            .outpoints(outpoints.to_vec())
            .build();
        // We don't need to fetch prev txouts as we just want the outspends.
        let sync_result = self.sync(request, false)?;
        let _ = local_chain.apply_update(
            sync_result
                .chain_update
                .expect("request included chain tip"),
        );
        // Store tip at which first sync was completed. This will be our reference tip.
        let local_tip = local_chain.tip();
        let mut graph = TxGraph::default();
        let _ = graph.apply_update(sync_result.tx_update);
        let txids: HashSet<_> = outpoints
            .iter()
            .flat_map(|op| graph.outspends(*op))
            .copied()
            .collect();
        let entries = match self.mempool_entries(txids, Some(local_tip)) {
            Ok(entries) => entries,
            Err(Error::TipChanged(expected, actual)) => {
                log::debug!(
                    "Chain tip changed from {:?} to {:?} while \
                    getting mempool spenders. Restarting.",
                    expected,
                    actual
                );
                return self.mempool_spenders(outpoints);
            }
            Err(e) => {
                return Err(e);
            }
        };
        Ok(entries)
    }
}

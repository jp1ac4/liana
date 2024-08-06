use std::{collections::HashSet, convert::TryInto};

use bdk_electrum::{
    bdk_chain::{
        bitcoin,
        local_chain::LocalChain,
        spk_client::{FullScanRequest, FullScanResult, SyncRequest, SyncResult},
        ChainPosition, ConfirmationHeightAnchor, TxGraph,
    },
    electrum_client::{self, Config, ElectrumApi, HeaderNotification},
    ElectrumExt,
};

use super::utils::{
    block_id_from_tip, height_i32_from_u32, height_i32_from_usize, height_usize_from_i32,
    outpoints_from_tx,
};
use crate::{
    bitcoin::{
        electrum::utils::height_u32_from_i32, BlockChainTip, MempoolEntry, MempoolEntryFees,
    },
    config,
};

// If Electrum takes more than 3 minutes to answer one of our queries, fail.
const RPC_SOCKET_TIMEOUT: u8 = 180;

// Number of retries while communicating with the Electrum server.
// A retry happens with exponential back-off (base 2) so this makes us give up after (1+2+4+8+16+32=) 63 seconds.
const RETRY_LIMIT: u8 = 6;

/// An error in the Electrum client.
#[derive(Debug)]
pub enum Error {
    Server(electrum_client::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Server(e) => write!(f, "Electrum error: '{}'.", e),
        }
    }
}

pub struct Client(electrum_client::Client);

impl Client {
    /// Create a new client and perform sanity checks.
    pub fn new(electrum_config: &config::ElectrumConfig) -> Result<Self, Error> {
        let config = Config::builder()
            .retry(RETRY_LIMIT)
            .timeout(Some(RPC_SOCKET_TIMEOUT))
            .build();
        let client =
            bdk_electrum::electrum_client::Client::from_config(&electrum_config.addr, config)
                .map_err(Error::Server)?;
        let ele_client = Self(client);
        Ok(ele_client)
    }

    pub fn chain_tip(&self) -> BlockChainTip {
        let HeaderNotification { height, header } =
            self.0.block_headers_subscribe().expect("must succeed");
        BlockChainTip {
            height: height_i32_from_usize(height),
            hash: header.block_hash(),
        }
    }

    pub fn block_hash(&self, height: i32) -> Option<bitcoin::BlockHash> {
        let hash = self
            .0
            .block_header(height_usize_from_i32(height))
            .ok()?
            .block_hash();
        Some(hash)
    }

    pub fn is_in_chain(&self, tip: &BlockChainTip) -> bool {
        self.block_hash(tip.height)
            .map(|bh| bh == tip.hash)
            .unwrap_or(false)
    }

    pub fn genesis_block_timestamp(&self) -> u32 {
        self.0
            .block_header(0)
            .expect("Genesis block must always be there")
            .time
    }

    pub fn genesis_block(&self) -> BlockChainTip {
        let hash = self
            .0
            .block_header(0)
            .expect("Genesis block hash must always be there")
            .block_hash();
        BlockChainTip { hash, height: 0 }
    }

    pub fn broadcast_tx(&self, tx: &bitcoin::Transaction) -> Result<bitcoin::Txid, Error> {
        self.0.transaction_broadcast(tx).map_err(Error::Server)
    }

    pub fn tip_time(&self) -> Option<u32> {
        let tip_height = self.chain_tip().height;
        self.0
            .block_header(height_usize_from_i32(tip_height))
            .ok()
            .map(|bh| bh.time)
    }

    fn sync_with_confirmation_height_anchor(
        &self,
        request: SyncRequest,
        batch_size: usize,
        fetch_prev_txouts: bool,
    ) -> Result<SyncResult<ConfirmationHeightAnchor>, Error> {
        Ok(self
            .0
            .sync(request, batch_size, fetch_prev_txouts)
            .map_err(Error::Server)?
            .with_confirmation_height_anchor())
    }

    /// Perform the given `SyncRequest` with `ConfirmationTimeHeightAnchor`.
    pub fn sync_with_confirmation_time_height_anchor(
        &self,
        request: SyncRequest,
        batch_size: usize,
        fetch_prev_txouts: bool,
    ) -> Result<SyncResult, Error> {
        self.0
            .sync(request, batch_size, fetch_prev_txouts)
            .map_err(Error::Server)?
            .with_confirmation_time_height_anchor(&self.0)
            .map_err(Error::Server)
    }

    /// Perform the given `FullScanRequest` with `ConfirmationTimeHeightAnchor`.
    pub fn full_scan_with_confirmation_time_height_anchor<K: Ord + Clone>(
        &self,
        request: FullScanRequest<K>,
        stop_gap: usize,
        batch_size: usize,
        fetch_prev_txouts: bool,
    ) -> Result<FullScanResult<K>, Error> {
        self.0
            .full_scan(request, stop_gap, batch_size, fetch_prev_txouts)
            .map_err(Error::Server)?
            .with_confirmation_time_height_anchor(&self.0)
            .map_err(Error::Server)
    }

    /// Get mempool entry.
    ///
    /// For simplicity, this function will restart if the server's chain tip changes before completion.
    pub fn mempool_entry(&self, txid: &bitcoin::Txid) -> Result<Option<MempoolEntry>, Error> {
        log::debug!("Getting mempool entry for txid '{}'.", txid);
        const BATCH_SIZE: usize = 200;
        let mut graph = TxGraph::default();
        let mut local_chain = LocalChain::from_genesis_hash(self.genesis_block().hash).0;
        let chain_tip = self.chain_tip();
        if chain_tip.height > 0 {
            let _ = local_chain
                .insert_block(block_id_from_tip(chain_tip))
                .expect("only contains genesis block");
        }
        let local_tip = local_chain.tip();
        // First, get the tx itself and check it's unconfirmed.
        let request = SyncRequest::from_chain_tip(local_chain.tip()).chain_txids(vec![*txid]);
        // We'll get prev txouts for this tx when we find its ancestors below.
        let sync_result = self.sync_with_confirmation_height_anchor(request, BATCH_SIZE, false)?;
        let _ = local_chain.apply_update(sync_result.chain_update);
        if local_chain.tip() != local_tip {
            log::debug!("Chain tip changed while getting mempool entry. Restarting.");
            return self.mempool_entry(txid);
        }
        match sync_result.graph_update.get_chain_position(
            &local_chain,
            local_chain.tip().block_id(),
            *txid,
        ) {
            Some(ChainPosition::Unconfirmed(_)) => {}
            _ => {
                // Either txid has been confirmed or is no longer in mempool.
                return Ok(None);
            }
        }
        let _ = graph.apply_update(sync_result.graph_update);
        let tx = graph
            .get_tx(*txid)
            .expect("we must have tx in graph after sync");
        // Now iterate over increasing depths of descendants.
        // As they are descendants, we can assume they are all unconfirmed.
        let mut desc_ops = outpoints_from_tx(&tx);
        while !desc_ops.is_empty() {
            log::debug!("Syncing descendant outpoints: {:?}", desc_ops);
            let request = SyncRequest::from_chain_tip(local_chain.tip())
                .cache_graph_txs(&graph)
                .chain_outpoints(desc_ops.clone());
            // Fetch prev txouts to ensure we have all required txs in the graph to calculate fees.
            // An unconfirmed descendant may have a confirmed parent that we wouldn't have in our graph.
            let sync_result =
                self.sync_with_confirmation_height_anchor(request, BATCH_SIZE, true)?;
            let _ = local_chain.apply_update(sync_result.chain_update);
            if local_chain.tip() != local_tip {
                log::debug!("Chain tip changed while getting mempool entry. Restarting.");
                return self.mempool_entry(txid);
            }
            let _ = graph.apply_update(sync_result.graph_update);
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
        let mut anc_txids: HashSet<_> = tx
            .input
            .iter()
            .map(|txin| txin.previous_output.txid)
            .collect();
        while !anc_txids.is_empty() {
            log::debug!("Syncing ancestor txids: {:?}", anc_txids);
            let request = SyncRequest::from_chain_tip(local_chain.tip())
                .cache_graph_txs(&graph)
                .chain_txids(anc_txids.clone());
            // We expect to have prev txouts for all unconfirmed ancestors in our graph so no need to fetch them here.
            // Note we keep iterating through ancestors until we find one that is confirmed and only need to calculate
            // fees for unconfirmed transactions.
            let sync_result =
                self.sync_with_confirmation_height_anchor(request, BATCH_SIZE, false)?;
            let _ = local_chain.apply_update(sync_result.chain_update);
            if local_chain.tip() != local_tip {
                log::debug!("Chain tip changed while getting mempool entry. Restarting.");
                return self.mempool_entry(txid);
            }
            let _ = graph.apply_update(sync_result.graph_update);

            // Add ancestors of any unconfirmed txs.
            anc_txids = anc_txids
                .iter()
                .filter_map(|anc_txid| {
                    if let Some(ChainPosition::Unconfirmed(_)) = graph.get_chain_position(
                        &local_chain,
                        local_chain.tip().block_id(),
                        *anc_txid,
                    ) {
                        let anc_tx = graph.get_tx(*anc_txid).expect("we must have it");
                        Some(
                            anc_tx
                                .input
                                .clone()
                                .iter()
                                .map(|txin| txin.previous_output.txid)
                                .collect::<HashSet<_>>(),
                        )
                    } else {
                        None
                    }
                })
                .flatten()
                .collect();
        }
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
        for desc_txid in graph.walk_descendants(*txid, |_, desc_txid| Some(desc_txid)) {
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
            log::debug!("Getting fee and size for anc txid '{}'.", anc_tx.txid());
            if let Some(ChainPosition::Unconfirmed(_)) =
                graph.get_chain_position(&local_chain, local_chain.tip().block_id(), anc_tx.txid())
            {
                let fee = graph
                    .calculate_fee(&anc_tx)
                    .expect("all required txs are in graph");
                anc_fees += fee;
                anc_size += anc_tx.vsize();
            } else {
                log::debug!("Ancestor txid '{}' is not unconfirmed.", anc_tx.txid());
                continue;
            }
        }
        let fees = MempoolEntryFees {
            base: bitcoin::Amount::from_sat(base_fee),
            ancestor: bitcoin::Amount::from_sat(anc_fees),
            descendant: bitcoin::Amount::from_sat(desc_fees),
        };
        let entry = MempoolEntry {
            vsize: base_size.try_into().expect("tx size must fit into u64"),
            fees,
            ancestor_vsize: anc_size.try_into().expect("tx size must fit into u64"),
        };
        // It's possible that the chain tip has now changed, but it hadn't done as of the last sync,
        // so go ahead and return the results.
        Ok(Some(entry))
    }

    /// Get mempool spenders of the given outpoints.
    ///
    /// Will restart if chain tip changes before completion.
    pub fn mempool_spenders(
        &self,
        outpoints: &[bitcoin::OutPoint],
    ) -> Result<Vec<MempoolEntry>, Error> {
        log::debug!("Getting mempool spenders for outpoints: {:?}.", outpoints);
        const BATCH_SIZE: usize = 200;
        let mut local_chain = LocalChain::from_genesis_hash(self.genesis_block().hash).0;
        let chain_tip = self.chain_tip();
        if chain_tip.height > 0 {
            let _ = local_chain
                .insert_block(block_id_from_tip(chain_tip))
                .expect("only contains genesis block");
        }
        let local_tip = local_chain.tip();
        let request =
            SyncRequest::from_chain_tip(local_chain.tip()).chain_outpoints(outpoints.to_vec());
        // We don't need to fetch prev txouts as we just want the outspends.
        let sync_result = self.sync_with_confirmation_height_anchor(request, BATCH_SIZE, false)?;
        let _ = local_chain.apply_update(sync_result.chain_update);
        if local_chain.tip() != local_tip {
            log::debug!("Chain tip changed while getting mempool spenders. Restarting.");
            return self.mempool_spenders(outpoints);
        }
        let graph = sync_result.graph_update;
        let txids: HashSet<_> = outpoints
            .iter()
            .flat_map(|op| graph.outspends(*op))
            .collect();
        let mut entries = Vec::new();
        for txid in txids {
            let entry = self.mempool_entry(txid)?;
            if let Some(entry) = entry {
                entries.push(entry);
            }
        }
        // Make sure the tip didn't change while going through each of the txids.
        if self.chain_tip() != chain_tip {
            log::debug!("Chain tip changed while getting mempool spenders. Restarting.");
            return self.mempool_spenders(outpoints);
        }
        Ok(entries)
    }

    /// Get the block in `local_chain` that `tip` has in common with the Electrum server.
    pub fn common_ancestor(
        &self,
        local_chain: &LocalChain,
        tip: &BlockChainTip,
    ) -> Option<BlockChainTip> {
        // TODO: Keep this assertion? Although we expect it to hold, we could still look for
        // common ancestor otherwise with the condition it be no higher than `tip.height`.
        assert_eq!(
            local_chain
                .get(height_u32_from_i32(tip.height))
                .expect("tip is in local chain")
                .hash(),
            tip.hash,
            "we should only find common ancestor of `tip` if it's in `local_chain`"
        );
        let server_tip = self.chain_tip();
        log::debug!("finding common ancestor of {}", tip);
        log::debug!(
            "server tip is {} and local tip is {:?}",
            server_tip,
            local_chain.tip()
        );

        let mut ancestor = None;
        for cp in local_chain.tip().iter() {
            let cp_height = height_i32_from_u32(cp.height());
            // Consider only our local chain checkpoints no higher than the server tip and `tip`.
            if cp_height > server_tip.height.min(tip.height) {
                continue;
            }
            // We assume the server's tip won't become lower while iterating and so we should
            // be able to retrieve the hash.
            let hash = self
                .block_hash(cp_height)
                .expect("height is below server's tip height");
            if hash == cp.hash() {
                let tip = BlockChainTip {
                    height: cp_height,
                    hash,
                };
                ancestor = Some(tip);
                break;
            }
            if self.chain_tip() != server_tip {
                log::debug!("Chain tip changed while finding common ancestor. Restarting.");
                return self.common_ancestor(local_chain, tip);
            }
        }
        ancestor
    }
}

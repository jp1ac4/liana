//! Interface to the Bitcoin network.
//!
//! Broadcast transactions, poll for new unspent coins, gather fee estimates.

pub mod d;
pub mod electrum;
pub mod poller;

use crate::{
    bitcoin::d::BitcoindError,
    database::{Coin, DatabaseConnection},
    descriptors,
    poller::looper::UpdatedCoins,
};
pub use d::{MempoolEntry, SyncProgress};

use std::{collections::HashSet, fmt, str::FromStr, sync};

use miniscript::bitcoin::{self, address, secp256k1};

pub const COINBASE_MATURITY: i32 = 100;

/// The number of script pubkeys to derive and cache from the descriptors
/// over and above the last revealed script index.
pub const LOOK_AHEAD_LIMIT: u32 = 200;

/// The expected genesis block hash.
pub fn expected_genesis_hash(network: &bitcoin::Network) -> bitcoin::BlockHash {
    let hash = match network {
        bitcoin::Network::Bitcoin => {
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
        }
        bitcoin::Network::Signet => {
            "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6"
        }
        bitcoin::Network::Testnet => {
            "000000000933ea01ad0ee984209779baaec3ced90fa3f408719526f8d77f4943"
        }
        bitcoin::Network::Regtest => {
            "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206"
        }
        net => panic!("Unexpected network '{}'", net),
    };
    bitcoin::BlockHash::from_str(hash).expect("must be valid")
}

// FIXME: We could avoid this type (and all the conversions entailing allocations) if bitcoind
// exposed the derivation index from the parent descriptor in the LSB result.
#[derive(Debug, Clone)]
pub struct UTxO {
    pub outpoint: bitcoin::OutPoint,
    pub amount: bitcoin::Amount,
    pub block_height: Option<i32>,
    pub address: bitcoin::Address<address::NetworkUnchecked>,
    pub is_immature: bool,
}

/// Information about a block
#[derive(Debug, Clone, Eq, PartialEq, Copy)]
pub struct Block {
    pub hash: bitcoin::BlockHash,
    pub height: i32,
    pub time: u32,
}

/// Information about the best block in the chain
#[derive(Debug, Clone, Eq, PartialEq, Copy)]
pub struct BlockChainTip {
    pub hash: bitcoin::BlockHash,
    pub height: i32,
}

impl fmt::Display for BlockChainTip {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({},{})", self.height, self.hash)
    }
}

/// Our Bitcoin backend.
pub trait BitcoinInterface: Send {
    fn genesis_block_timestamp(&self) -> u32;

    fn genesis_block(&self) -> BlockChainTip;

    /// Get the progress of the block chain synchronization.
    /// Returns a rounded up percentage between 0 and 1. Use the `is_synced` method to be sure the
    /// backend is completely synced to the best known tip.
    fn sync_progress(&self) -> SyncProgress;

    /// Get the best block info.
    fn chain_tip(&self) -> BlockChainTip;

    /// Get the timestamp set in the best block's header.
    fn tip_time(&self) -> Option<u32>;

    /// Get the block hash in best chain at given height.
    fn block_hash(&self, height: i32) -> Option<bitcoin::BlockHash>;

    /// Check whether this former tip is part of the current best chain.
    fn is_in_chain(&self, tip: &BlockChainTip) -> bool;

    fn update_coins(
        &mut self,
        db_conn: &mut Box<dyn DatabaseConnection>,
        previous_tip: &BlockChainTip,
        descs: &[descriptors::SinglePathLianaDesc],
        secp: &secp256k1::Secp256k1<secp256k1::VerifyOnly>,
    ) -> UpdatedCoins;

    // fn init(
    //     &mut self,
    //     db_conn: &mut Box<dyn DatabaseConnection>,
    //     descs: &[descriptors::SinglePathLianaDesc],
    //     latest_tip: BlockChainTip,
    // ) -> bool;

    /// Get coins received since the specified tip.
    // fn received_coins(
    //     &self,
    //     tip: &BlockChainTip,
    //     descs: &[descriptors::SinglePathLianaDesc],
    // ) -> Vec<UTxO>;

    // /// Get all coins that were confirmed, and at what height and time. Along with "expired"
    // /// unconfirmed coins (for instance whose creating transaction may have been replaced).
    // fn confirmed_coins(
    //     &self,
    //     outpoints: &[bitcoin::OutPoint],
    // ) -> (Vec<(bitcoin::OutPoint, i32, u32)>, Vec<bitcoin::OutPoint>);

    // /// Get all coins that are being spent, and the spending txid.
    // fn spending_coins(
    //     &self,
    //     outpoints: &[bitcoin::OutPoint],
    // ) -> Vec<(bitcoin::OutPoint, bitcoin::Txid)>;

    // /// Get all coins that are spent with the final spend tx txid and blocktime. Along with the
    // /// coins for which the spending transaction "expired" (a conflicting transaction was mined and
    // /// it wasn't spending this coin).
    // fn spent_coins(
    //     &self,
    //     outpoints: &[(bitcoin::OutPoint, bitcoin::Txid)],
    // ) -> (
    //     Vec<(bitcoin::OutPoint, bitcoin::Txid, Block)>,
    //     Vec<bitcoin::OutPoint>,
    // );

    /// Get the common ancestor between the Bitcoin backend's tip and the given tip.
    fn common_ancestor(&self, tip: &BlockChainTip) -> Option<BlockChainTip>;

    /// Broadcast this transaction to the Bitcoin P2P network
    fn broadcast_tx(&self, tx: &bitcoin::Transaction) -> Result<(), String>;

    /// Trigger a rescan of the block chain for transactions related to this descriptor since
    /// the given date.
    fn start_rescan(
        &self,
        desc: &descriptors::LianaDescriptor,
        timestamp: u32,
    ) -> Result<(), String>;

    /// Rescan progress percentage. Between 0 and 1.
    fn rescan_progress(&self) -> Option<f64>;

    /// Get the last block chain tip with a timestamp below this. Timestamp must be a valid block
    /// timestamp.
    fn block_before_date(&self, timestamp: u32) -> Option<BlockChainTip>;

    /// Get a transaction related to the wallet along with potential confirmation info.
    fn wallet_transaction(
        &self,
        txid: &bitcoin::Txid,
    ) -> Option<(bitcoin::Transaction, Option<Block>)>;

    /// Get the details of unconfirmed transactions spending these outpoints, if any.
    fn mempool_spenders(&self, outpoints: &[bitcoin::OutPoint]) -> Vec<MempoolEntry>;

    /// Get mempool data for the given transaction.
    ///
    /// Returns `None` if the transaction is not in the mempool.
    fn mempool_entry(&self, txid: &bitcoin::Txid) -> Option<MempoolEntry>;
}

impl BitcoinInterface for d::BitcoinD {
    fn update_coins(
        &mut self,
        db_conn: &mut Box<dyn DatabaseConnection>,
        previous_tip: &BlockChainTip,
        descs: &[descriptors::SinglePathLianaDesc],
        secp: &secp256k1::Secp256k1<secp256k1::VerifyOnly>,
    ) -> UpdatedCoins {
        let network = db_conn.network();
        let curr_coins = db_conn.coins(&[], &[]);
        log::debug!("Current coins: {:?}", curr_coins);

        // Start by fetching newly received coins.
        let mut received = Vec::new();
        for utxo in self.received_coins(previous_tip, descs) {
            let UTxO {
                outpoint,
                amount,
                address,
                is_immature,
                ..
            } = utxo;
            // We can only really treat them if we know the derivation index that was used.
            let address = match address.require_network(network) {
                Ok(addr) => addr,
                Err(e) => {
                    log::error!("Invalid network for address: {}", e);
                    continue;
                }
            };
            if let Some((derivation_index, is_change)) =
                db_conn.derivation_index_by_address(&address)
            {
                // First of if we are receiving coins that are beyond our next derivation index,
                // adjust it.
                if derivation_index > db_conn.receive_index() {
                    db_conn.set_receive_index(derivation_index, secp);
                }
                if derivation_index > db_conn.change_index() {
                    db_conn.set_change_index(derivation_index, secp);
                }

                // Now record this coin as a newly received one.
                if !curr_coins.contains_key(&utxo.outpoint) {
                    let coin = Coin {
                        outpoint,
                        is_immature,
                        amount,
                        derivation_index,
                        is_change,
                        block_info: None,
                        spend_txid: None,
                        spend_block: None,
                    };
                    received.push(coin);
                }
            } else {
                // TODO: maybe we could try out something here? Like bruteforcing the next 200 indexes?
                log::error!(
                    "Could not get derivation index for coin '{}' (address: '{}')",
                    &utxo.outpoint,
                    &address
                );
            }
        }
        log::debug!("Newly received coins: {:?}", received);

        // We need to take the newly received ones into account as well, as they may have been
        // confirmed within the previous tip and the current one, and we may not poll this chunk of the
        // chain anymore.
        let to_be_confirmed: Vec<bitcoin::OutPoint> = curr_coins
            .values()
            .chain(received.iter())
            .filter_map(|coin| {
                if coin.block_info.is_none() {
                    Some(coin.outpoint)
                } else {
                    None
                }
            })
            .collect();
        let (confirmed, expired) = self.confirmed_coins(&to_be_confirmed);
        log::debug!("Newly confirmed coins: {:?}", confirmed);
        log::debug!("Expired coins: {:?}", expired);

        // We need to take the newly received ones into account as well, as they may have been
        // spent within the previous tip and the current one, and we may not poll this chunk of the
        // chain anymore.
        // NOTE: curr_coins contain the "spending" coins. So this takes care of updating the spend_txid
        // if a coin's spending transaction gets RBF'd.
        let expired_set: HashSet<_> = expired.iter().collect();
        let to_be_spent: Vec<bitcoin::OutPoint> = curr_coins
            .values()
            .chain(received.iter())
            .filter_map(|coin| {
                // Always check for spends when the spend tx is not confirmed as it might get RBF'd.
                if (coin.spend_txid.is_some() && coin.spend_block.is_some())
                    || expired_set.contains(&coin.outpoint)
                {
                    None
                } else {
                    Some(coin.outpoint)
                }
            })
            .collect();
        let spending = self.spending_coins(&to_be_spent);
        log::debug!("Newly spending coins: {:?}", spending);

        // Mark coins in a spending state whose Spend transaction was confirmed as such. Note we
        // need to take into account the freshly marked as spending coins as well, as their spend
        // may have been confirmed within the previous tip and the current one, and we may not poll
        // this chunk of the chain anymore.
        let spending_coins: Vec<(bitcoin::OutPoint, bitcoin::Txid)> = db_conn
            .list_spending_coins()
            .values()
            .map(|coin| (coin.outpoint, coin.spend_txid.expect("Coin is spending")))
            .chain(spending.iter().cloned())
            .collect();
        let (spent, expired_spending) = self.spent_coins(spending_coins.as_slice());
        let spent = spent
            .into_iter()
            .map(|(oupoint, txid, block)| (oupoint, txid, block.height, block.time))
            .collect();
        log::debug!("Newly spent coins: {:?}", spent);

        UpdatedCoins {
            received,
            confirmed,
            expired,
            spending,
            expired_spending,
            spent,
        }
    }

    fn genesis_block_timestamp(&self) -> u32 {
        self.get_block_stats(
            self.get_block_hash(0)
                .expect("Genesis block hash must always be there"),
        )
        .expect("Genesis block must always be there")
        .time
    }

    fn genesis_block(&self) -> BlockChainTip {
        let height = 0;
        let hash = self
            .get_block_hash(height)
            .expect("Genesis block hash must always be there");
        BlockChainTip { hash, height }
    }

    fn sync_progress(&self) -> SyncProgress {
        self.sync_progress()
    }

    fn chain_tip(&self) -> BlockChainTip {
        self.chain_tip()
    }

    fn is_in_chain(&self, tip: &BlockChainTip) -> bool {
        self.get_block_hash(tip.height)
            .map(|bh| bh == tip.hash)
            .unwrap_or(false)
    }

    fn common_ancestor(&self, tip: &BlockChainTip) -> Option<BlockChainTip> {
        let mut stats = self.get_block_stats(tip.hash)?;
        let mut ancestor = *tip;

        while stats.confirmations == -1 {
            stats = self.get_block_stats(stats.previous_blockhash?)?;
            ancestor = BlockChainTip {
                hash: stats.blockhash,
                height: stats.height,
            };
        }

        Some(ancestor)
    }

    fn broadcast_tx(&self, tx: &bitcoin::Transaction) -> Result<(), String> {
        match self.broadcast_tx(tx) {
            Ok(()) => Ok(()),
            Err(BitcoindError::Server(e)) => Err(e.to_string()),
            // We assume the Bitcoin backend doesn't fail, so it must be a JSONRPC error.
            Err(e) => panic!(
                "Unexpected Bitcoin error when broadcast transaction: '{}'.",
                e
            ),
        }
    }

    fn start_rescan(
        &self,
        desc: &descriptors::LianaDescriptor,
        timestamp: u32,
    ) -> Result<(), String> {
        // FIXME: in theory i think this could potentially fail to actually start the rescan.
        self.start_rescan(desc, timestamp)
            .map_err(|e| e.to_string())
    }

    fn rescan_progress(&self) -> Option<f64> {
        self.rescan_progress()
    }

    fn block_before_date(&self, timestamp: u32) -> Option<BlockChainTip> {
        self.tip_before_timestamp(timestamp)
    }

    fn tip_time(&self) -> Option<u32> {
        let tip = self.chain_tip();
        Some(self.get_block_stats(tip.hash)?.time)
    }

    fn block_hash(&self, height: i32) -> Option<bitcoin::BlockHash> {
        self.get_block_hash(height)
    }

    fn wallet_transaction(
        &self,
        txid: &bitcoin::Txid,
    ) -> Option<(bitcoin::Transaction, Option<Block>)> {
        self.get_transaction(txid).map(|res| (res.tx, res.block))
    }

    fn mempool_spenders(&self, outpoints: &[bitcoin::OutPoint]) -> Vec<MempoolEntry> {
        self.mempool_txs_spending_prevouts(outpoints)
            .into_iter()
            .filter_map(|txid| self.mempool_entry(&txid))
            .collect()
    }

    fn mempool_entry(&self, txid: &bitcoin::Txid) -> Option<MempoolEntry> {
        self.mempool_entry(txid)
    }
}

// FIXME: do we need to repeat the entire trait implemenation? Isn't there a nicer way?
impl BitcoinInterface for sync::Arc<sync::Mutex<dyn BitcoinInterface + 'static>> {
    fn genesis_block_timestamp(&self) -> u32 {
        self.lock().unwrap().genesis_block_timestamp()
    }

    fn genesis_block(&self) -> BlockChainTip {
        self.lock().unwrap().genesis_block()
    }

    fn sync_progress(&self) -> SyncProgress {
        self.lock().unwrap().sync_progress()
    }

    fn chain_tip(&self) -> BlockChainTip {
        self.lock().unwrap().chain_tip()
    }

    fn block_hash(&self, height: i32) -> Option<bitcoin::BlockHash> {
        self.lock().unwrap().block_hash(height)
    }

    fn is_in_chain(&self, tip: &BlockChainTip) -> bool {
        self.lock().unwrap().is_in_chain(tip)
    }

    fn update_coins(
        &mut self,
        db_conn: &mut Box<dyn DatabaseConnection>,
        previous_tip: &BlockChainTip,
        descs: &[descriptors::SinglePathLianaDesc],
        secp: &secp256k1::Secp256k1<secp256k1::VerifyOnly>,
    ) -> UpdatedCoins {
        self.lock()
            .unwrap()
            .update_coins(db_conn, previous_tip, descs, secp)
    }

    fn common_ancestor(&self, tip: &BlockChainTip) -> Option<BlockChainTip> {
        self.lock().unwrap().common_ancestor(tip)
    }

    fn broadcast_tx(&self, tx: &bitcoin::Transaction) -> Result<(), String> {
        self.lock().unwrap().broadcast_tx(tx)
    }

    fn start_rescan(
        &self,
        desc: &descriptors::LianaDescriptor,
        timestamp: u32,
    ) -> Result<(), String> {
        self.lock().unwrap().start_rescan(desc, timestamp)
    }

    fn rescan_progress(&self) -> Option<f64> {
        self.lock().unwrap().rescan_progress()
    }

    fn block_before_date(&self, timestamp: u32) -> Option<BlockChainTip> {
        self.lock().unwrap().block_before_date(timestamp)
    }

    fn tip_time(&self) -> Option<u32> {
        self.lock().unwrap().tip_time()
    }

    fn wallet_transaction(
        &self,
        txid: &bitcoin::Txid,
    ) -> Option<(bitcoin::Transaction, Option<Block>)> {
        self.lock().unwrap().wallet_transaction(txid)
    }

    fn mempool_spenders(&self, outpoints: &[bitcoin::OutPoint]) -> Vec<MempoolEntry> {
        self.lock().unwrap().mempool_spenders(outpoints)
    }

    fn mempool_entry(&self, txid: &bitcoin::Txid) -> Option<MempoolEntry> {
        self.lock().unwrap().mempool_entry(txid)
    }
}

impl BitcoinInterface for electrum::Electrum {
    fn update_coins(
        &mut self,
        db_conn: &mut Box<dyn DatabaseConnection>,
        previous_tip: &BlockChainTip,
        _descs: &[descriptors::SinglePathLianaDesc],
        secp: &secp256k1::Secp256k1<secp256k1::VerifyOnly>,
    ) -> UpdatedCoins {
        // Make sure `previous_tip` is same as our local tip.
        let local_tip = self.bdk_wallet.chain_tip();
        assert_eq!(previous_tip, &local_tip);

        self.bdk_wallet.reveal_spks(db_conn);

        let pre_sync_coins = &self.bdk_wallet.coins();
        self.sync_wallet();
        let wallet_coins = &self.bdk_wallet.coins();

        // All newly received coins.
        let mut received = Vec::new();
        // All newly confirmed coins, which may include those from `received`.
        let mut confirmed = Vec::new();
        // All pre-sync coins whose spend txid has changed (could be None or Some).
        let mut expired_spending = Vec::new();
        // Newly received coins that are spending together with existing coins
        // whose spending info has changed.
        let mut spending = Vec::new();
        // All coins that are newly spent, which may include those from `spending`.
        let mut spent = Vec::new();

        for (w_op, w_c) in wallet_coins {
            if let Some(pre_c) = pre_sync_coins.get(w_op) {
                if pre_c != w_c {
                    // If `pre_c.block_info.is_some()`, then we can assume the value hasn't changed
                    // as otherwise there must have been a reorg and the DB would have been rolled back.
                    if pre_c.block_info.is_none() && w_c.block_info.is_some() {
                        let block = w_c.block_info.expect("already checked");
                        confirmed.push((*w_op, block.height, block.time));
                    }
                    if pre_c.spend_txid != w_c.spend_txid {
                        if pre_c.spend_txid.is_some() {
                            expired_spending.push(*w_op);
                        }
                        if let Some(txid) = w_c.spend_txid {
                            spending.push((*w_op, txid));
                        }
                    }
                    // If `pre_c.spend_block.is_some()`, then we can assume the value hasn't changed
                    // as otherwise there must have been a reorg and the DB would have been rolled back.
                    if pre_c.spend_block.is_none() && w_c.spend_block.is_some() {
                        let block = w_c.spend_block.expect("already checked");
                        let txid = w_c.spend_txid.expect("must be present if spend confirmed");
                        spent.push((*w_op, txid, block.height, block.time));
                    }
                }
            } else {
                if w_c.derivation_index > db_conn.receive_index() {
                    db_conn.set_receive_index(w_c.derivation_index, secp);
                }
                if w_c.derivation_index > db_conn.change_index() {
                    db_conn.set_change_index(w_c.derivation_index, secp);
                }
                received.push(w_c.to_db_coin());
                if let Some(block) = w_c.block_info {
                    confirmed.push((*w_op, block.height, block.time));
                }
                if let Some(txid) = w_c.spend_txid {
                    spending.push((*w_op, txid));
                }
                if let Some(block) = w_c.spend_block {
                    let spend_txid = w_c.spend_txid.expect("must be present if spend confirmed");
                    spent.push((*w_op, spend_txid, block.height, block.time));
                }
            }
        }
        // Any pre-sync coins that are no longer in the wallet.
        let expired: Vec<_> = pre_sync_coins
            .keys()
            .filter(|c| !wallet_coins.contains_key(c))
            .cloned()
            .collect();

        UpdatedCoins {
            received,
            confirmed,
            expired,
            expired_spending,
            spending,
            spent,
        }
    }

    fn genesis_block_timestamp(&self) -> u32 {
        self.client().genesis_block_timestamp()
    }

    fn genesis_block(&self) -> BlockChainTip {
        self.client().genesis_block()
    }

    fn chain_tip(&self) -> BlockChainTip {
        self.client().chain_tip()
    }

    fn block_hash(&self, height: i32) -> Option<bitcoin::BlockHash> {
        self.client().block_hash(height)
    }

    fn is_in_chain(&self, tip: &BlockChainTip) -> bool {
        self.client().is_in_chain(tip)
    }

    fn common_ancestor(&self, tip: &BlockChainTip) -> Option<BlockChainTip> {
        // Make sure `tip` is same as our local tip.
        let local_tip = self.bdk_wallet.chain_tip();
        assert_eq!(tip, &local_tip);
        self.common_ancestor()
    }

    fn broadcast_tx(&self, tx: &bitcoin::Transaction) -> Result<(), String> {
        self.client().broadcast_tx(tx)
    }

    fn wallet_transaction(
        &self,
        txid: &bitcoin::Txid,
    ) -> Option<(bitcoin::Transaction, Option<Block>)> {
        self.bdk_wallet
            .graph
            .graph()
            .get_tx_node(*txid)
            .map(|tx_node| {
                let block = tx_node.anchors.first().map(|info| Block {
                    hash: info.hash,
                    height: info.height,
                    time: 0,
                });
                let tx = tx_node.tx.as_ref().clone();
                (tx, block)
            })
    }

    fn mempool_entry(&self, _txid: &bitcoin::Txid) -> Option<MempoolEntry> {
        None
    }

    fn mempool_spenders(&self, _outpoints: &[bitcoin::OutPoint]) -> Vec<MempoolEntry> {
        Vec::new()
    }

    fn sync_progress(&self) -> SyncProgress {
        // FIXME
        let blocks = self.chain_tip().height as u64;
        SyncProgress::new(1.0, blocks, blocks)
    }

    fn start_rescan(
        &self,
        _desc: &descriptors::LianaDescriptor,
        _timestamp: u32,
    ) -> Result<(), String> {
        todo!()
    }

    fn rescan_progress(&self) -> Option<f64> {
        None
    }

    fn block_before_date(&self, _timestamp: u32) -> Option<BlockChainTip> {
        unimplemented!("db should not be marked as rescanning")
    }

    fn tip_time(&self) -> Option<u32> {
        self.client().tip_time()
    }
}

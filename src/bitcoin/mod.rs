//! Interface to the Bitcoin network.
//!
//! Broadcast transactions, poll for new unspent coins, gather fee estimates.

pub mod d;
pub mod electrum;
pub mod poller;

use crate::{
    bitcoin::{
        d::{BitcoindError, CachedTxGetter, LSBlockEntry},
        electrum::sync_through_bdk,
    },
    database::{BlockInfo, Coin, DatabaseConnection},
    descriptors,
    poller::looper::UpdatedCoins,
};
use bdk_electrum::{
    bdk_chain::local_chain::CheckPoint,
    electrum_client::{ElectrumApi, Error, HeaderNotification},
};
pub use d::{MempoolEntry, SyncProgress};
use electrum::coins_from_wallet;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    convert::TryInto,
    fmt, sync,
};

use miniscript::bitcoin::{self, address, secp256k1};

pub const COINBASE_MATURITY: i32 = 100;

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
        &self,
        db_conn: &mut Box<dyn DatabaseConnection>,
        previous_tip: &BlockChainTip,
        descs: &[descriptors::SinglePathLianaDesc],
        secp: &secp256k1::Secp256k1<secp256k1::VerifyOnly>,
    ) -> UpdatedCoins;

    fn sync(
        &mut self,
        //db_conn: &mut Box<dyn DatabaseConnection>,
    );

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
    fn sync(
        &mut self,
        //db_conn: &mut Box<dyn DatabaseConnection>,
    ) {
    }

    fn update_coins(
        &self,
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

    fn sync(
        &mut self,
        //db_conn: &mut Box<dyn DatabaseConnection>,
    ) {
        self.lock().unwrap().sync()
    }

    fn update_coins(
        &self,
        db_conn: &mut Box<dyn DatabaseConnection>,
        previous_tip: &BlockChainTip,
        descs: &[descriptors::SinglePathLianaDesc],
        secp: &secp256k1::Secp256k1<secp256k1::VerifyOnly>,
    ) -> UpdatedCoins {
        self.lock()
            .unwrap()
            .update_coins(db_conn, previous_tip, descs, secp)
    }

    // fn received_coins(
    //     &self,
    //     tip: &BlockChainTip,
    //     descs: &[descriptors::SinglePathLianaDesc],
    // ) -> Vec<UTxO> {
    //     self.lock().unwrap().received_coins(tip, descs)
    // }

    // fn confirmed_coins(
    //     &self,
    //     outpoints: &[bitcoin::OutPoint],
    // ) -> (Vec<(bitcoin::OutPoint, i32, u32)>, Vec<bitcoin::OutPoint>) {
    //     self.lock().unwrap().confirmed_coins(outpoints)
    // }

    // fn spending_coins(
    //     &self,
    //     outpoints: &[bitcoin::OutPoint],
    // ) -> Vec<(bitcoin::OutPoint, bitcoin::Txid)> {
    //     self.lock().unwrap().spending_coins(outpoints)
    // }

    // fn spent_coins(
    //     &self,
    //     outpoints: &[(bitcoin::OutPoint, bitcoin::Txid)],
    // ) -> (
    //     Vec<(bitcoin::OutPoint, bitcoin::Txid, Block)>,
    //     Vec<bitcoin::OutPoint>,
    // ) {
    //     self.lock().unwrap().spent_coins(outpoints)
    // }

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

impl BitcoinInterface for electrum::Electrum {
    fn sync(
        &mut self,
        //db_conn: &mut Box<dyn DatabaseConnection>,
    ) {
        self.bdk_wallet.existing_coins = self.bdk_wallet.wallet_coins.clone();
        sync_through_bdk(&mut self.bdk_wallet, &self.client);
        self.bdk_wallet.wallet_coins = coins_from_wallet(&self.bdk_wallet)
            .into_iter()
            .map(|c| (c.outpoint, c))
            .collect();
    }

    fn update_coins(
        &self,
        db_conn: &mut Box<dyn DatabaseConnection>,
        previous_tip: &BlockChainTip,
        descs: &[descriptors::SinglePathLianaDesc],
        secp: &secp256k1::Secp256k1<secp256k1::VerifyOnly>,
    ) -> UpdatedCoins {
        // let receive_desc = descs.first().unwrap();
        // let change_desc = descs.last().unwrap();
        let existing_coins = self.bdk_wallet.existing_coins.clone();
        let wallet_coins = self.bdk_wallet.wallet_coins.clone();
        let updated_coins: HashMap<_, _> = wallet_coins
        .iter()
        .filter_map(|c|
            // Keep new and updated
            if existing_coins.get(c.0) != Some(c.1) {
                Some(c)
                // let desc_to_use = if c.is_change { change_desc } else { receive_desc };
                // Some(UTxO {
                //     outpoint: *op,
                //     block_height: c.block_info.map(|info| info.height),
                //     amount: c.amount,
                //     address: desc_to_use.derive(c.derivation_index, &secp).address(self.bdk_wallet.network).as_unchecked().clone(),
                //     is_immature: c.is_immature,
            } else {
                None
            }
        ).collect();

        let received: Vec<_> = wallet_coins
            .values()
            .filter_map(|c| {
                if !existing_coins.contains_key(&c.outpoint) {
                    Some(Coin {
                        outpoint: c.outpoint,
                        is_immature: c.is_immature,
                        block_info: c.block_info.map(|info| BlockInfo {
                            height: info.height,
                            time: info.time,
                        }),
                        amount: c.amount,
                        derivation_index: c.derivation_index,
                        is_change: c.is_change,
                        spend_txid: c.spend_txid,
                        spend_block: c.spend_block.map(|info| BlockInfo {
                            height: info.height,
                            time: info.time,
                        }),
                    })
                } else {
                    None
                }
            })
            .collect();
        let confirmed: Vec<_> = updated_coins
            .iter()
            .filter_map(|(op, c)| {
                c.block_info
                    .map(|info| (op.clone().clone(), info.height, info.time))
            })
            .collect();
        let expired: Vec<_> = existing_coins
            .keys()
            .filter(|c| !wallet_coins.contains_key(c))
            .cloned()
            .collect();
        let expired_spending: Vec<_> = existing_coins
            .iter()
            .filter(|c| c.1.spend_txid.is_some() && c.1.spend_block.is_none())
            .filter_map(|c| {
                wallet_coins
                    .get(c.0)
                    .filter(|wc| wc.spend_txid != Some(c.1.spend_txid.unwrap()))
            })
            .map(|c| c.outpoint)
            .collect();
        let spending: Vec<_> = updated_coins
            .iter()
            .filter_map(|c| {
                if c.1.spend_block.is_none() {
                    c.1.spend_txid.map(|txid| (c.0.clone().clone(), txid))
                } else {
                    None
                }
            })
            .collect();
        let spent: Vec<_> = updated_coins
            .into_iter()
            .filter_map(|(op, c)| {
                c.spend_block
                    .map(|info| (op.clone(), c.spend_txid.unwrap(), info.height, info.time))
            })
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

    // fn received_coins(
    //         &self,
    //         tip: &BlockChainTip,
    //         descs: &[descriptors::SinglePathLianaDesc],
    //     ) -> Vec<UTxO> {
    //     let secp = secp256k1::Secp256k1::verification_only();
    //     let receive_desc = descs.first().unwrap();
    //     let change_desc = descs.last().unwrap();
    //     let existing_coins = self.bdk_wallet.existing_coins;
    //     let wallet_coins = self.bdk_wallet.wallet_coins;
    //     wallet_coins
    //     .iter()
    //     .filter_map(|(op, c)|
    //         if !existing_coins.contains_key(op) {
    //             let desc_to_use = if c.is_change { change_desc } else { receive_desc };
    //             Some(UTxO {
    //                 outpoint: *op,
    //                 block_height: c.block_info.map(|info| info.height),
    //                 amount: c.amount,
    //                 address: desc_to_use.derive(c.derivation_index, &secp).address(self.bdk_wallet.network).as_unchecked().clone(),
    //                 is_immature: c.is_immature,
    //             })
    //         } else {
    //             None
    //         }
    //     ).collect()
    // }

    // fn confirmed_coins(
    //         &self,
    //         outpoints: &[bitcoin::OutPoint],
    //     ) -> (Vec<(bitcoin::OutPoint, i32, u32)>, Vec<bitcoin::OutPoint>) {

    // }

    fn genesis_block_timestamp(&self) -> u32 {
        self.client
            .block_header(0)
            .expect("Genesis block must always be there")
            .time
    }

    fn genesis_block(&self) -> BlockChainTip {
        let hash = self
            .client
            .block_header(0)
            .expect("Genesis block hash must always be there")
            .block_hash();
        BlockChainTip { hash, height: 0 }
    }

    fn chain_tip(&self) -> BlockChainTip {
        let HeaderNotification { height, .. } = self.client.block_headers_subscribe().unwrap();
        let new_tip_height = height as i32;
        let new_tip_hash = self.block_hash(new_tip_height).unwrap();
        BlockChainTip {
            height: new_tip_height,
            hash: new_tip_hash,
        }
    }

    fn block_hash(&self, height: i32) -> Option<bitcoin::BlockHash> {
        let hash = self
            .client
            .block_header(height.try_into().unwrap())
            .expect("msg")
            .block_hash();
        Some(hash)
    }

    fn is_in_chain(&self, tip: &BlockChainTip) -> bool {
        self.block_hash(tip.height)
            .map(|bh| bh == tip.hash)
            .unwrap_or(false)
    }

    fn common_ancestor(&self, tip: &BlockChainTip) -> Option<BlockChainTip> {
        let new_tip_height = tip.height as u32;

        //     // If electrum returns a tip height that is lower than our previous tip, then checkpoints do
        //     // not need updating. We just return the previous tip and use that as the point of agreement.
        //     if new_tip_height < prev_tip.height() {
        //         return Ok((prev_tip.clone(), Some(prev_tip.height())));
        //     }

        const CHAIN_SUFFIX_LENGTH: u32 = 8;
        //     // Atomically fetch the latest `CHAIN_SUFFIX_LENGTH` count of blocks from Electrum. We use this
        //     // to construct our checkpoint update.
        let mut new_blocks = {
            let start_height = new_tip_height.saturating_sub(CHAIN_SUFFIX_LENGTH - 1);
            let hashes = self
                .client
                .block_headers(start_height as _, CHAIN_SUFFIX_LENGTH as _)
                .unwrap()
                .headers
                .into_iter()
                .map(|h| h.block_hash());
            (start_height..).zip(hashes).collect::<BTreeMap<u32, _>>()
        };

        // Find the "point of agreement" (if any).
        let agreement_cp = {
            let mut agreement_cp = Option::<CheckPoint>::None;
            for cp in self
                .bdk_wallet
                .local_chain
                .tip()
                .iter()
                .filter(|cp| cp.height() <= new_tip_height)
            {
                let cp_block = cp.block_id();
                let hash = match new_blocks.get(&cp_block.height) {
                    Some(&hash) => hash,
                    None => {
                        assert!(
                            new_tip_height >= cp_block.height,
                            "already checked that new tip cannot be smaller"
                        );
                        let hash = self
                            .client
                            .block_header(cp_block.height as _)
                            .unwrap()
                            .block_hash();
                        new_blocks.insert(cp_block.height, hash);
                        hash
                    }
                };
                if hash == cp_block.hash {
                    agreement_cp = Some(cp);
                    break;
                }
            }
            agreement_cp
        };
        agreement_cp.as_ref().map(|cp| BlockChainTip {
            height: cp.height().try_into().unwrap(),
            hash: cp.hash(),
        })
    }

    fn broadcast_tx(&self, tx: &bitcoin::Transaction) -> Result<(), String> {
        match self.client.transaction_broadcast(tx) {
            Ok(_txid) => Ok(()),
            // TODO: check for which error types we shouldn't panic
            Err(e) => panic!(
                "Unexpected Bitcoin error when broadcast transaction: '{}'.",
                e
            ),
        }
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

    fn mempool_entry(&self, txid: &bitcoin::Txid) -> Option<MempoolEntry> {
        None
    }

    fn mempool_spenders(&self, outpoints: &[bitcoin::OutPoint]) -> Vec<MempoolEntry> {
        Vec::new()
    }

    fn sync_progress(&self) -> SyncProgress {
        // FIXME
        let blocks = self.chain_tip().height as u64;
        SyncProgress::new(1.0, blocks, blocks)
    }

    fn start_rescan(
        &self,
        desc: &descriptors::LianaDescriptor,
        timestamp: u32,
    ) -> Result<(), String> {
        todo!()
    }

    fn rescan_progress(&self) -> Option<f64> {
        unimplemented!("db should not be marked as rescanning")
    }

    fn block_before_date(&self, timestamp: u32) -> Option<BlockChainTip> {
        unimplemented!("db should not be marked as rescanning")
    }

    fn tip_time(&self) -> Option<u32> {
        todo!()
    }
}

// impl BitcoinInterface for electrum::Electrum {

//     fn received_coins(
//         &self,
//         tip: &BlockChainTip,
//         descs: &[descriptors::SinglePathLianaDesc],
//     ) -> Vec<UTxO> {
//         let lsb_res = self.list_since_block(&tip.hash);

//         lsb_res
//             .received_coins
//             .into_iter()
//             .filter_map(|entry| {
//                 let LSBlockEntry {
//                     outpoint,
//                     amount,
//                     block_height,
//                     address,
//                     parent_descs,
//                     is_immature,
//                 } = entry;
//                 if parent_descs
//                     .iter()
//                     .any(|parent_desc| descs.iter().any(|desc| desc == parent_desc))
//                 {
//                     Some(UTxO {
//                         outpoint,
//                         amount,
//                         block_height,
//                         address,
//                         is_immature,
//                     })
//                 } else {
//                     None
//                 }
//             })
//             .collect()
//     }

//     fn confirmed_coins(
//         &self,
//         outpoints: &[bitcoin::OutPoint],
//     ) -> (Vec<(bitcoin::OutPoint, i32, u32)>, Vec<bitcoin::OutPoint>) {
//         // The confirmed and expired coins to be returned.
//         let mut confirmed = Vec::with_capacity(outpoints.len());
//         let mut expired = Vec::new();
//         // Cached calls to `gettransaction`.
//         let mut tx_getter = CachedTxGetter::new(self);

//         for op in outpoints {
//             let res = if let Some(res) = tx_getter.get_transaction(&op.txid) {
//                 res
//             } else {
//                 log::error!("Transaction not in wallet for coin '{}'.", op);
//                 continue;
//             };

//             // If the transaction was confirmed, mark the coin as such.
//             if let Some(block) = res.block {
//                 // Do not mark immature coinbase deposits as confirmed until they become mature.
//                 if res.is_coinbase && res.confirmations < COINBASE_MATURITY {
//                     log::debug!("Coin at '{}' comes from an immature coinbase transaction with {} confirmations. Not marking it as confirmed for now.", op, res.confirmations);
//                     continue;
//                 }
//                 confirmed.push((*op, block.height, block.time));
//                 continue;
//             }

//             // If the transaction was dropped from the mempool, discard the coin.
//             if !self.is_in_mempool(&op.txid) {
//                 expired.push(*op);
//             }
//         }

//         (confirmed, expired)
//     }

//     fn spending_coins(
//         &self,
//         outpoints: &[bitcoin::OutPoint],
//     ) -> Vec<(bitcoin::OutPoint, bitcoin::Txid)> {
//         let mut spent = Vec::with_capacity(outpoints.len());

//         for op in outpoints {
//             if self.is_spent(op) {
//                 let spending_txid = if let Some(txid) = self.get_spender_txid(op) {
//                     txid
//                 } else {
//                     // TODO: better handling of this edge case.
//                     log::error!(
//                         "Could not get spender of '{}'. Not reporting it as spending.",
//                         op
//                     );
//                     continue;
//                 };

//                 spent.push((*op, spending_txid));
//             }
//         }

//         spent
//     }

//     fn spent_coins(
//         &self,
//         outpoints: &[(bitcoin::OutPoint, bitcoin::Txid)],
//     ) -> (
//         Vec<(bitcoin::OutPoint, bitcoin::Txid, Block)>,
//         Vec<bitcoin::OutPoint>,
//     ) {
//         // Spend coins to be returned.
//         let mut spent = Vec::with_capacity(outpoints.len());
//         // Coins whose spending transaction isn't in our local mempool anymore.
//         let mut expired = Vec::new();
//         // Cached calls to `gettransaction`.
//         let mut tx_getter = CachedTxGetter::new(self);

//         for (op, txid) in outpoints {
//             let res = if let Some(res) = tx_getter.get_transaction(txid) {
//                 res
//             } else {
//                 log::error!("Could not get tx {} spending coin {}.", txid, op);
//                 continue;
//             };

//             // If the transaction was confirmed, mark it as such.
//             if let Some(block) = res.block {
//                 spent.push((*op, *txid, block));
//                 continue;
//             }

//             // If a conflicting transaction was confirmed instead, replace the txid of the
//             // spender for this coin with it and mark it as confirmed.
//             let conflict = res.conflicting_txs.iter().find_map(|txid| {
//                 tx_getter.get_transaction(txid).and_then(|tx| {
//                     tx.block.and_then(|block| {
//                         // Being part of our watchonly wallet isn't enough, as it could be a
//                         // conflicting transaction which spends a different set of coins. Make sure
//                         // it does actually spend this coin.
//                         tx.tx.input.iter().find_map(|txin| {
//                             if &txin.previous_output == op {
//                                 Some((*txid, block))
//                             } else {
//                                 None
//                             }
//                         })
//                     })
//                 })
//             });
//             if let Some((txid, block)) = conflict {
//                 spent.push((*op, txid, block));
//                 continue;
//             }

//             // If the transaction was not confirmed, a conflicting transaction spending this coin
//             // too wasn't mined, but still isn't in our mempool anymore, mark the spend as expired.
//             if !self.is_in_mempool(txid) {
//                 expired.push(*op);
//             }
//         }

//         (spent, expired)
//     }

// }

use std::{
    collections::{BTreeMap, HashMap},
    convert::TryInto,
    sync::Arc,
};

use bdk_chain::{
    bitcoin::{self, bip32, BlockHash, OutPoint, ScriptBuf, TxOut},
    indexer::keychain_txout::KeychainTxOutIndex,
    local_chain::{ChangeSet as ChainChangeSet, CheckPoint, LocalChain},
    miniscript::{Descriptor, DescriptorPublicKey},
    tx_graph::{self, TxGraph},
    Anchor, BlockId, ChainOracle, ChainPosition, ConfirmationBlockTime, IndexedTxGraph, TxUpdate,
};
use miniscript::bitcoin::bip32::ChildNumber;

use super::utils::{block_id_from_tip, height_i32_from_u32, height_u32_from_i32};
use crate::bitcoin::{Block, BlockChainTip, BlockInfo, Coin, COINBASE_MATURITY};
use liana::descriptors::LianaDescriptor;

// We don't want to overload the server (each SPK is separate call).
const LOOK_AHEAD_LIMIT: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeychainType {
    Receive,
    Change,
}

// The following anchor implementation is taken from a previous version of bdk_chain:
// https://docs.rs/bdk_chain/0.15.0/bdk_chain/struct.ConfirmationTimeHeightAnchor.html
//
// We use this when we know the confirmation height and time, but not the confirmation
// block hash, which is the case for transactions stored in our database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfirmationTimeHeightAnchor {
    /// The confirmation height of the transaction being anchored.
    pub confirmation_height: u32,
    /// The confirmation time of the transaction being anchored.
    pub confirmation_time: u64,
    /// The anchor block.
    pub anchor_block: BlockId,
}

impl Anchor for ConfirmationTimeHeightAnchor {
    fn anchor_block(&self) -> BlockId {
        self.anchor_block
    }

    fn confirmation_height_upper_bound(&self) -> u32 {
        self.confirmation_height
    }
}

// For `ConfirmationBlockTime`, the anchor block is also the confirmation block.
impl From<ConfirmationBlockTime> for ConfirmationTimeHeightAnchor {
    fn from(cbt: ConfirmationBlockTime) -> Self {
        Self {
            confirmation_height: cbt.block_id.height,
            confirmation_time: cbt.confirmation_time,
            anchor_block: cbt.block_id,
        }
    }
}

impl ConfirmationTimeHeightAnchor {
    fn into_confirmation_block_info(self) -> BlockInfo {
        BlockInfo {
            height: height_i32_from_u32(self.confirmation_height),
            time: self.confirmation_time.try_into().expect("u32 by consensus"),
        }
    }
}

pub struct BdkWallet {
    graph: IndexedTxGraph<ConfirmationTimeHeightAnchor, KeychainTxOutIndex<KeychainType>>,
    local_chain: LocalChain,
    // Store descriptors for use when getting SPKs.
    receive_desc: Descriptor<DescriptorPublicKey>,
    change_desc: Descriptor<DescriptorPublicKey>,
}

impl BdkWallet {
    /// Create a new BDK wallet and initialize with the given data that was
    /// valid as of `tip`.
    ///
    /// If there is no `tip`, then any provided data will be ignored.
    ///
    /// `receive_index` and `change_index` are the last used derivation
    /// indices for the receive and change descriptors, respectively.
    pub fn new(
        main_descriptor: &LianaDescriptor,
        genesis_hash: BlockHash,
        tip: Option<BlockChainTip>,
        coins: &[Coin],
        txs: &[bitcoin::Transaction],
        receive_index: ChildNumber,
        change_index: ChildNumber,
    ) -> Self {
        let local_chain = LocalChain::from_genesis_hash(genesis_hash).0;
        let receive_desc = main_descriptor
            .receive_descriptor()
            .as_descriptor_public_key();
        let change_desc = main_descriptor
            .change_descriptor()
            .as_descriptor_public_key();

        let mut bdk_wallet = BdkWallet {
            graph: {
                let mut indexer = KeychainTxOutIndex::<KeychainType>::new(LOOK_AHEAD_LIMIT);
                let _ = indexer.insert_descriptor(KeychainType::Receive, receive_desc.clone());
                let _ = indexer.insert_descriptor(KeychainType::Change, change_desc.clone());
                IndexedTxGraph::new(indexer)
            },
            local_chain,
            receive_desc: receive_desc.clone(),
            change_desc: change_desc.clone(),
        };
        if let Some(tip) = tip {
            // This will be our anchor for any confirmed transactions.
            let anchor_block = block_id_from_tip(tip);
            if tip.height > 0 {
                log::debug!("inserting block into local chain: {:?}", anchor_block);
                let _ = bdk_wallet
                    .local_chain
                    .insert_block(anchor_block)
                    .expect("local chain only contains genesis block");
            }
            // Update the last used derivation index for both change and receive addresses.
            log::debug!(
                "revealing SPKs up to receive index {receive_index} and change index {change_index}"
            );
            bdk_wallet.reveal_spks(receive_index, change_index);

            // Update the existing coins and transactions information using a TxGraph changeset.
            log::debug!("Number of coins to load: {}.", coins.len());
            log::debug!("Number of txs to load: {}.", txs.len());
            let mut graph_cs = tx_graph::ChangeSet::default();
            for tx in txs {
                graph_cs.txs.insert(Arc::new(tx.clone()));
            }
            for coin in coins {
                // First of all insert the txout itself.
                let script_pubkey = bdk_wallet.get_spk(coin.derivation_index, coin.is_change);
                let txout = TxOut {
                    script_pubkey,
                    value: coin.amount,
                };
                graph_cs.txouts.insert(coin.outpoint, txout);
                // If the coin's deposit transaction is confirmed, tell BDK by inserting an anchor.
                if let Some(block) = coin.block_info {
                    graph_cs.anchors.insert((
                        ConfirmationTimeHeightAnchor {
                            confirmation_height: height_u32_from_i32(block.height),
                            confirmation_time: block.time.into(),
                            anchor_block,
                        },
                        coin.outpoint.txid,
                    ));
                }
                // If the coin's spending transaction is confirmed, do the same.
                if let Some(block) = coin.spend_block {
                    let spend_txid = coin.spend_txid.expect("Must be present if confirmed.");
                    graph_cs.anchors.insert((
                        ConfirmationTimeHeightAnchor {
                            confirmation_height: height_u32_from_i32(block.height),
                            confirmation_time: block.time.into(),
                            anchor_block,
                        },
                        spend_txid,
                    ));
                }
            }
            let mut graph = TxGraph::default();
            graph.apply_changeset(graph_cs);
            // We set a `seen_at` for unconfirmed txs so that they're part of the canonical history,
            // since they have been seen in the best chain corresponding to `tip`.
            // Note that the `BdkWallet::coins()` method only considers such coins.
            // Choose the earliest possible value, 0, so that any transactions seen subsequently are
            // guaranteed to have a later `seen_at` value.
            let _ = bdk_wallet.graph.apply_update_at(graph.into(), Some(0));
        }
        bdk_wallet
    }

    /// Get a reference to the local chain.
    pub fn local_chain(&self) -> &LocalChain {
        &self.local_chain
    }

    /// Whether `tip` exists in `local_chain`.
    ///
    /// Returns `None` if no block at that height exists in `local_chain`.
    pub fn is_in_chain(&self, tip: BlockChainTip) -> Option<bool> {
        self.local_chain
            .is_block_in_chain(block_id_from_tip(tip), self.local_chain().tip().block_id())
            .expect("function is infallible")
    }

    /// Get a reference to the graph.
    pub fn graph(&self) -> &TxGraph<ConfirmationTimeHeightAnchor> {
        self.graph.graph()
    }

    /// Get a reference to the transaction index.
    pub fn index(&self) -> &KeychainTxOutIndex<KeychainType> {
        &self.graph.index
    }

    /// Reveal SPKs based on derivation indices set in DB.
    pub fn reveal_spks(&mut self, receive_index: ChildNumber, change_index: ChildNumber) {
        let mut keychain_update = BTreeMap::new();
        keychain_update.insert(KeychainType::Receive, receive_index.into());
        keychain_update.insert(KeychainType::Change, change_index.into());
        self.apply_keychain_update(keychain_update)
    }

    fn get_spk(&self, der_index: bip32::ChildNumber, is_change: bool) -> ScriptBuf {
        // Try to get it from the BDK wallet cache first, failing that derive it from the appropriate
        // descriptor.
        let chain_kind = if is_change {
            KeychainType::Change
        } else {
            KeychainType::Receive
        };
        if let Some(spk) = self.graph.index.spk_at_index(chain_kind, der_index.into()) {
            spk.to_owned()
        } else {
            let desc = if is_change {
                &self.change_desc
            } else {
                &self.receive_desc
            };
            desc.at_derivation_index(der_index.into())
                .expect("Not multipath and index isn't hardened.")
                .script_pubkey()
        }
    }

    /// Get the coins currently stored by the `BdkWallet` optionally filtered by `outpoints`.
    /// If `outpoints` is `None`, no filter will be applied.
    /// If `outpoints` is an empty slice, no coins will be returned.
    /// If `last_seen` is set, only those unconfirmed transactions with a matching last seen
    /// will be considered.
    /// Regardless of filters, any coins or spend transactions that have never been seen on the
    /// best chain will be ignored.
    pub fn coins(
        &self,
        outpoints: Option<&[bitcoin::OutPoint]>,
        last_seen: Option<u64>,
    ) -> HashMap<OutPoint, Coin> {
        // Get an iterator over all the wallet txos (not only the currently unspent ones) by using
        // lower level methods.
        let tx_graph = self.graph.graph();
        let txo_index = &self.graph.index;
        let tip_id = self.local_chain.tip().block_id();
        let wallet_txos = tx_graph.filter_chain_txouts(
            &self.local_chain,
            tip_id,
            txo_index.outpoints().iter().copied(),
        );
        let mut wallet_coins = HashMap::new();
        // Go through all the wallet txos and create a coin for each.
        for ((k, i), full_txo) in wallet_txos {
            let outpoint = full_txo.outpoint;
            if outpoints.map(|ops| !ops.contains(&outpoint)) == Some(true) {
                continue;
            }
            let amount = full_txo.txout.value;
            let derivation_index = i.into();
            let is_change = matches!(k, KeychainType::Change);
            let block_info = match full_txo.chain_position {
                ChainPosition::Unconfirmed {
                    last_seen: Some(ls),
                } => {
                    if let Some(last_seen) = last_seen.filter(|last_seen| *last_seen != ls) {
                        log::debug!("Ignoring coin at {}, which was last seen at {} instead of {} as required.", outpoint, ls, last_seen);
                        continue;
                    }
                    None
                }
                ChainPosition::Unconfirmed { last_seen: None } => {
                    log::debug!(
                        "Ignoring coin at {}, which has never been seen on the best chain.",
                        outpoint
                    );
                    continue;
                }
                ChainPosition::Confirmed { anchor, .. } => {
                    Some(anchor.into_confirmation_block_info())
                }
            };

            // Immature if from a coinbase transaction with less than a hundred confs.
            let is_immature = full_txo.is_on_coinbase
                && block_info
                    .and_then(|blk| {
                        let tip_height: i32 = height_i32_from_u32(tip_id.height);
                        tip_height
                            .checked_sub(blk.height)
                            .map(|confs| confs < COINBASE_MATURITY)
                    })
                    .unwrap_or(true);

            // Get spend status of this coin.
            let (mut spend_txid, mut spend_block) = (None, None);
            if let Some((spend_pos, txid)) = full_txo.spent_by {
                spend_txid = Some(txid);
                match spend_pos {
                    ChainPosition::Unconfirmed {
                        last_seen: Some(ls),
                    } => {
                        if let Some(last_seen) = last_seen.filter(|last_seen| *last_seen != ls) {
                            log::debug!(
                                "Ignoring spend txid {} for coin at {}, \
                                which was last seen at {} instead of {} as required.",
                                txid,
                                outpoint,
                                ls,
                                last_seen
                            );
                            spend_txid = None;
                        }
                    }
                    ChainPosition::Unconfirmed { last_seen: None } => {
                        log::debug!(
                            "Ignoring spend txid {} for coin at {}, \
                            which has never been seen on the best chain.",
                            txid,
                            outpoint,
                        );
                        spend_txid = None;
                    }
                    ChainPosition::Confirmed { anchor, .. } => {
                        spend_block = Some(anchor.into_confirmation_block_info());
                    }
                };
            }
            let coin = crate::bitcoin::Coin {
                outpoint,
                amount,
                derivation_index,
                is_change,
                is_immature,
                block_info,
                spend_txid,
                spend_block,
            };
            wallet_coins.insert(coin.outpoint, coin);
        }
        wallet_coins
    }

    pub fn get_transaction(
        &self,
        txid: &bitcoin::Txid,
    ) -> Option<(bitcoin::Transaction, Option<Block>)> {
        self.graph.graph().get_tx_node(*txid).map(|tx_node| {
            let block = tx_node.anchors.iter().next().map(|info| Block {
                hash: info.anchor_block.hash, // not necessarily the confirmation block hash
                height: height_i32_from_u32(info.confirmation_height),
                time: info.confirmation_time.try_into().expect("u32 by consensus"),
            });
            let tx = tx_node.tx.as_ref().clone();
            (tx, block)
        })
    }

    /// Apply an update to the local chain.
    /// Panics if update does not connect to the local chain.
    pub fn apply_connected_chain_update(&mut self, chain_update: CheckPoint) -> ChainChangeSet {
        self.local_chain
            .apply_update(chain_update)
            .expect("update must connect to local chain")
    }

    /// Apply a graph update.
    pub fn apply_graph_update_at<A>(&mut self, tx_update: TxUpdate<A>, seen_at: Option<u64>)
    where
        A: Ord + Into<ConfirmationTimeHeightAnchor>,
    {
        let _ = self
            .graph
            .apply_update_at(tx_update.map_anchors(|a| a.into()), seen_at);
    }

    /// Apply a keychain update.
    pub fn apply_keychain_update(&mut self, keychain_update: BTreeMap<KeychainType, u32>) {
        let _ = self.graph.index.reveal_to_target_multi(&keychain_update);
    }
}

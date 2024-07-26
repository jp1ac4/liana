use std::{
    collections::{BTreeMap, HashMap, HashSet},
    convert::TryInto,
    sync::Arc,
};

use bdk_electrum::{
    bdk_chain::{
        bitcoin::{self, bip32, hashes::Hash, BlockHash, OutPoint, ScriptBuf, TxOut},
        keychain::KeychainTxOutIndex,
        local_chain::{CheckPoint, LocalChain},
        miniscript::{Descriptor, DescriptorPublicKey},
        spk_client::{FullScanRequest, SyncRequest},
        tx_graph::{self, TxGraph},
        BlockId, ChainOracle, ChainPosition, ConfirmationTimeHeightAnchor, IndexedTxGraph,
    },
    electrum_client::{Client, ElectrumApi, HeaderNotification},
    ElectrumExt,
};

use crate::{
    bitcoin::{expected_genesis_hash, BlockChainTip, COINBASE_MATURITY, LOOK_AHEAD_LIMIT},
    database::{BlockInfo, Coin, DatabaseConnection},
};

fn height_u32_from_i32(height: i32) -> u32 {
    height.try_into().expect("height must fit into u32")
}

fn height_i32_from_u32(height: u32) -> i32 {
    height.try_into().expect("height must fit into i32")
}

fn height_usize_from_i32(height: i32) -> usize {
    height.try_into().expect("height must fit into usize")
}

fn height_usize_from_u32(height: u32) -> usize {
    height.try_into().expect("height must fit into usize")
}

fn block_id_from_tip(tip: BlockChainTip) -> BlockId {
    BlockId {
        height: height_u32_from_i32(tip.height),
        hash: tip.hash,
    }
}

/// An error in the Electrum interface.
#[derive(Debug)]
pub enum ElectrumError {
    Server(bdk_electrum::electrum_client::Error),
    GenesisHashMismatch(
        BlockHash, /*expected hash*/
        BlockHash, /*actual hash*/
    ),
    BdkWallet(String),
}

impl std::fmt::Display for ElectrumError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ElectrumError::Server(e) => write!(f, "Electrum error: '{}'.", e),
            ElectrumError::GenesisHashMismatch(expected_hash, actual_hash) => {
                write!(
                    f,
                    "Genesis hash mismatch. The genesis hash is expected to be '{}' but was found to be '{}'.",
                    expected_hash, actual_hash
                )
            }
            ElectrumError::BdkWallet(e) => write!(f, "BDK wallet error: '{}'.", e),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeychainType {
    Deposit,
    Change,
}

pub struct BdkWallet {
    pub graph: IndexedTxGraph<ConfirmationTimeHeightAnchor, KeychainTxOutIndex<KeychainType>>,
    pub local_chain: LocalChain,
    // Store descriptors for use when getting SPKs.
    receive_desc: Descriptor<DescriptorPublicKey>,
    change_desc: Descriptor<DescriptorPublicKey>,
}

impl BdkWallet {
    /// Create a new BDK wallet using existing data from the database.
    ///
    /// It retrieves deposit and spend block hashes of coins from Electrum,
    /// as long as the DB chain tip is still in the best chain. Otherwise,
    /// it skips loading existing coins.
    fn from_db(db_conn: &mut Box<dyn DatabaseConnection>) -> Result<Self, ElectrumError> {
        let main_descriptor = db_conn.main_descriptor();
        println!("setting up BDK wallet from DB");
        let network = db_conn.network();
        let genesis_hash = expected_genesis_hash(&network);
        println!("bdk_wallet_from_db: genesis_hash: {genesis_hash}");

        // Poller may not have run yet so this could be NULL.
        let mut local_chain = LocalChain::from_genesis_hash(genesis_hash).0;
        let anchor_block = db_conn.chain_tip().map(|db_tip| {
            log::debug!("Db tip: {db_tip}");
            let block_id = block_id_from_tip(db_tip);
            if db_tip.height > 0 {
                let _ = local_chain
                    .insert_block(block_id)
                    .expect("only contains genesis block");
            }
            block_id
        });
        // If we stored hashes in the DB, we would not need to check the DB tip is still in the best chain.
        // For simplicity, ignore existing coins if the DB tip is no longer in the best chain.
        // Perhaps we could load some of the data, but Wwe certainly must not retrieve hashes for them.
        let existing_coins = db_conn.coins(&[], &[]);
        let existing_txs = list_transactions(db_conn);
        log::debug!("Number of txs loaded from DB: {}.", existing_txs.len());

        let receive_desc = main_descriptor
            .receive_descriptor()
            .as_descriptor_public_key();

        let change_desc = main_descriptor
            .change_descriptor()
            .as_descriptor_public_key();

        let mut bdk_wallet = BdkWallet {
            graph: {
                let mut indexer = KeychainTxOutIndex::<KeychainType>::new(LOOK_AHEAD_LIMIT);
                let _ = indexer.insert_descriptor(KeychainType::Deposit, receive_desc.clone());
                let _ = indexer.insert_descriptor(KeychainType::Change, change_desc.clone());
                IndexedTxGraph::new(indexer)
            },
            local_chain,
            receive_desc: receive_desc.clone(),
            change_desc: change_desc.clone(),
        };
        // Update the last used derivation index for both change and receive addresses.
        // It should be fine to do this even if DB tip is no longer in best chain.
        bdk_wallet.reveal_spks(db_conn);

        // Update the existing coins and transactions information using a TxGraph changeset.
        let mut graph_cs = tx_graph::ChangeSet::default();
        for tx in existing_txs {
            graph_cs.txs.insert(Arc::new(tx.tx));
        }
        for coin in existing_coins.values() {
            // First of all insert the txout itself.
            let script_pubkey = bdk_wallet.get_spk(coin.derivation_index, coin.is_change);
            let txout = TxOut {
                script_pubkey,
                value: coin.amount,
            };
            graph_cs.txouts.insert(coin.outpoint, txout);
            // If the coin's deposit transaction is confirmed, tell BDK by inserting an anchor.
            // Otherwise, we could insert a last seen timestamp but we don't have such data stored in
            // the table.
            if let Some(block_info) = coin.block_info {
                graph_cs.anchors.insert((
                    ConfirmationTimeHeightAnchor {
                        confirmation_height: height_u32_from_i32(block_info.height),
                        confirmation_time: block_info.time.into(),
                        anchor_block: anchor_block
                            .expect("db tip must be present if coin has block info"),
                    },
                    coin.outpoint.txid,
                ));
            }
            // If the coin's spending transaction is confirmed, do the same.
            if let Some(spend_block_info) = coin.spend_block {
                let spend_txid = coin.spend_txid.expect("Must be present if confirmed.");
                graph_cs.anchors.insert((
                    ConfirmationTimeHeightAnchor {
                        confirmation_height: height_u32_from_i32(spend_block_info.height),
                        confirmation_time: spend_block_info.time.into(),
                        anchor_block: anchor_block
                            .expect("db tip must be present if coin has block info"),
                    },
                    spend_txid,
                ));
            }
        }
        let mut graph = TxGraph::default();
        graph.apply_changeset(graph_cs);
        let _ = bdk_wallet.graph.apply_update(graph);
        Ok(bdk_wallet)
    }

    /// Reveal SPKs based on derivation indices set in DB.
    pub fn reveal_spks(&mut self, db_conn: &mut Box<dyn DatabaseConnection>) {
        let mut last_active_indices = BTreeMap::new();
        let receive_index: u32 = db_conn.receive_index().into();
        last_active_indices.insert(KeychainType::Deposit, receive_index.saturating_add(0));
        let change_index: u32 = db_conn.change_index().into();
        last_active_indices.insert(KeychainType::Change, change_index.saturating_add(0));
        let a = self
            .graph
            .index
            .reveal_to_target_multi(&last_active_indices);
        println!("revealed keychains: {:?}", a.0.keys());
    }

    fn get_spk(&self, der_index: bip32::ChildNumber, is_change: bool) -> ScriptBuf {
        // Try to get it from the BDK wallet cache first, failing that derive it from the appropriate
        // descriptor.
        let chain_kind = if is_change {
            KeychainType::Change
        } else {
            KeychainType::Deposit
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

    /// Get the coins currently stored by the `BdkWallet`.
    pub fn coins(&self) -> HashMap<OutPoint, Coin> {
        // Get an iterator over all the wallet txos (not only the currently unspent ones) by using
        // lower level methods.
        let tx_graph = self.graph.graph();
        let txo_index = &self.graph.index;
        let tip_id = self.local_chain.tip().block_id();
        let wallet_txos =
            tx_graph.filter_chain_txouts(&self.local_chain, tip_id, txo_index.outpoints());
        let mut wallet_coins = Vec::new();
        // Go through all the wallet txos and create a coin for each.
        for ((k, i), full_txo) in wallet_txos {
            let outpoint = full_txo.outpoint;

            let amount = full_txo.txout.value;
            let derivation_index = i.into();
            let is_change = matches!(k, KeychainType::Change);
            let block_info = match full_txo.chain_position {
                ChainPosition::Unconfirmed(_) => None,
                ChainPosition::Confirmed(anchor) => {
                    let block_info = BlockInfo {
                        height: anchor.confirmation_height.try_into().unwrap(),
                        time: anchor.confirmation_time.try_into().unwrap(),
                    };
                    Some(block_info)
                }
            };

            // Immature if from a coinbase transaction with less than a hundred confs.
            let is_immature = full_txo.is_on_coinbase
                && block_info
                    .and_then(|blk| {
                        let tip_height: i32 = height_i32_from_u32(tip_id.height);
                        tip_height
                            .checked_sub(blk.height) //.checked_sub(blk.height)
                            .map(|confs| confs < COINBASE_MATURITY)
                    })
                    .unwrap_or(true);

            // Get spend status of this coin.
            let (mut spend_txid, mut spend_block) = (None, None);
            if let Some((spend_pos, txid)) = full_txo.spent_by {
                spend_txid = Some(txid);
                spend_block = match spend_pos {
                    ChainPosition::Unconfirmed(_) => None,
                    ChainPosition::Confirmed(anchor) => {
                        let block_info = BlockInfo {
                            height: anchor.confirmation_height.try_into().unwrap(),
                            time: anchor.confirmation_time.try_into().unwrap(),
                        };
                        Some(block_info)
                    }
                };
            }

            // Create the coin and if it doesn't exist or was modified, return it.
            let coin = Coin {
                outpoint,
                amount,
                derivation_index,
                is_change,
                is_immature,
                block_info,
                spend_txid,
                spend_block,
            };
            wallet_coins.push(coin);
        }
        println!(
            "coins from wallet: wallet coins length: {}",
            wallet_coins.len()
        );
        wallet_coins.into_iter().map(|c| (c.outpoint, c)).collect()
    }

    /// Get the (local) chain tip.
    pub fn chain_tip(&self) -> BlockChainTip {
        self.local_chain
            .get_chain_tip()
            .map(|t| BlockChainTip {
                hash: t.hash,
                height: height_i32_from_u32(t.height),
            })
            .expect("must contain at least genesis")
    }

    /// Sync the wallet using Electrum.
    fn sync_using_electrum(&mut self, client: &ElectrumClient) {
        let chain_tip = self.local_chain.tip();
        println!(
            "sync_through_electrum: local chain tip: {}",
            chain_tip.block_id().height
        );

        let (chain_update, graph_update, keychain_update) = if chain_tip.height() > 0 {
            log::info!("Performing sync.");
            let mut request =
                SyncRequest::from_chain_tip(chain_tip.clone()).cache_graph_txs(self.graph.graph());

            let all_spks: Vec<_> = self
                .graph
                .index
                .revealed_spks(..)
                .map(|(k, i, spk)| (k.to_owned(), i, spk.to_owned()))
                .collect::<Vec<_>>();
            request = request.chain_spks(all_spks.into_iter().map(|(k, spk_i, spk)| {
                eprint!("Scanning {:?}: {}", k, spk_i);
                spk
            }));

            let total_spks = request.spks.len();
            log::debug!("total_spks: {total_spks}");

            let sync_result = client
                .0
                .sync(request, 10, true)
                .unwrap()
                .with_confirmation_time_height_anchor(&client.0)
                .unwrap();
            (sync_result.chain_update, sync_result.graph_update, None)
        } else {
            log::info!("Performing full scan.");
            let mut request = FullScanRequest::from_chain_tip(chain_tip.clone())
                .cache_graph_txs(self.graph.graph());

            for (k, spks) in self.graph.index.all_unbounded_spk_iters() {
                request = request.set_spks_for_keychain(k, spks);
            }
            let scan_result = client
                .0
                .full_scan(request, 50, 10, true)
                .unwrap()
                .with_confirmation_time_height_anchor(&client.0)
                .unwrap();
            (
                scan_result.chain_update,
                scan_result.graph_update,
                Some(scan_result.last_active_indices),
            )
        };
        if let Some(keychain_update) = keychain_update {
            let _ = self.graph.index.reveal_to_target_multi(&keychain_update);
        }
        log::debug!(
            "sync_through_electrum: chain_update height: {}",
            chain_update.height()
        );
        let _ = self.local_chain.apply_update(chain_update).unwrap();
        let _ = self.graph.apply_update(graph_update);
    }

    // TODO: this is WIP
    fn _get_ancestors(&self, tx: bitcoin::Transaction) {
        let mut unconfirmed_txids = HashSet::new();
        for coin in self.coins().into_values() {
            if coin.block_info.is_none() {
                unconfirmed_txids.insert(coin.outpoint.txid);
            }
            if let Some(spend_txid) = coin.spend_txid {
                if coin.spend_block.is_none() {
                    unconfirmed_txids.insert(spend_txid);
                }
            }
        }
        let _ = self.graph.graph().walk_ancestors(tx, |_depth, anc_tx| {
            if unconfirmed_txids.contains(&anc_tx.txid()) {
                Some(anc_tx)
            } else {
                None
            }
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Transaction {
    pub txid: bitcoin::Txid,
    pub tx: bitcoin::Transaction,
}

fn list_transactions(db_conn: &mut Box<dyn DatabaseConnection>) -> Vec<Transaction> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;
    let txids = db_conn.list_txids(u32::MIN, now, 1000);
    db_conn
        .list_wallet_transactions(&txids)
        .into_iter()
        .map(|(tx, _, _)| Transaction {
            txid: tx.txid(),
            tx,
        })
        .collect()
}

/// Interface for Electrum backend.
pub struct Electrum {
    client: ElectrumClient,
    pub bdk_wallet: BdkWallet,
}

impl Electrum {
    pub fn from_db(
        db_conn: &mut Box<dyn DatabaseConnection>,
        url: &str,
    ) -> Result<Self, ElectrumError> {
        let network = db_conn.network();
        let client = ElectrumClient::new(url, &network)?;
        let bdk_wallet = BdkWallet::from_db(db_conn)?;
        Ok(Self { client, bdk_wallet })
    }

    pub fn client(&self) -> &ElectrumClient {
        &self.client
    }

    pub fn sync_wallet(&mut self) {
        self.bdk_wallet.sync_using_electrum(&self.client)
    }

    pub fn common_ancestor(&self) -> Option<BlockChainTip> {
        let server_tip_height = self.client.chain_tip().height as u32;

        // If electrum returns a tip height that is lower than our previous tip, then checkpoints do
        // not need updating. We just return the previous tip and use that as the point of agreement.
        // if new_tip_height < prev_tip.height() {
        //     return Ok((prev_tip.clone(), Some(prev_tip.height())));
        // }

        const CHAIN_SUFFIX_LENGTH: u32 = 8;
        // Atomically fetch the latest `CHAIN_SUFFIX_LENGTH` count of blocks from Electrum. We use this
        // to construct our checkpoint update.
        let mut new_blocks = {
            let start_height = server_tip_height.saturating_sub(CHAIN_SUFFIX_LENGTH - 1);
            let hashes = self
                .client
                .0
                .block_headers(
                    height_usize_from_u32(start_height),
                    CHAIN_SUFFIX_LENGTH as _,
                )
                .ok()?
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
                .filter(|cp| cp.height() <= server_tip_height)
            {
                let cp_block = cp.block_id();
                let hash = match new_blocks.get(&cp_block.height) {
                    Some(&hash) => hash,
                    None => {
                        assert!(
                            cp_block.height <= server_tip_height,
                            "already checked that server tip cannot be smaller"
                        );
                        let hash = self
                            .client
                            .0
                            .block_header(height_usize_from_u32(cp_block.height))
                            .ok()?
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
            height: height_i32_from_u32(cp.height()),
            hash: cp.hash(),
        })
    }
}

pub struct ElectrumClient(Client);

impl ElectrumClient {
    /// Create a new client and perform sanity checks.
    pub fn new(url: &str, network: &bitcoin::Network) -> Result<Self, ElectrumError> {
        let client =
            bdk_electrum::electrum_client::Client::new(url).map_err(ElectrumError::Server)?;
        let ele_client = Self(client);
        ele_client.sanity_checks(network)?;
        Ok(ele_client)
    }

    fn sanity_checks(&self, network: &bitcoin::Network) -> Result<(), ElectrumError> {
        let server_features = self.0.server_features().map_err(ElectrumError::Server)?;
        log::debug!("{:?}", server_features);
        let server_hash = {
            let mut hash = server_features.genesis_hash;
            hash.reverse();
            BlockHash::from_byte_array(hash)
        };
        let expected_hash = expected_genesis_hash(network);
        if server_hash != expected_hash {
            return Err(ElectrumError::GenesisHashMismatch(
                expected_hash,
                server_hash,
            ));
        }
        Ok(())
    }

    pub fn chain_tip(&self) -> BlockChainTip {
        let HeaderNotification { height, .. } =
            self.0.block_headers_subscribe().expect("must succeed");
        let new_tip_height = height as i32;
        let new_tip_hash = self.block_hash(new_tip_height).unwrap();
        BlockChainTip {
            height: new_tip_height,
            hash: new_tip_hash,
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

    pub fn broadcast_tx(&self, tx: &bitcoin::Transaction) -> Result<(), String> {
        match self.0.transaction_broadcast(tx) {
            Ok(_txid) => Ok(()),
            // TODO: check for which error types we shouldn't panic
            Err(e) => panic!("Unexpected error when broadcasting transaction: '{}'.", e),
        }
    }

    pub fn tip_time(&self) -> Option<u32> {
        let tip_height = self.chain_tip().height;
        self.0
            .block_header(height_usize_from_i32(tip_height))
            .ok()
            .map(|bh| bh.time)
    }
}

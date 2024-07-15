use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap},
    convert::TryInto,
    str::FromStr,
    sync::Arc,
};

// use bdk_chain::{
//     bitcoin::{self, bip32, constants::genesis_block, hashes, ScriptBuf, TxOut},
//     indexed_tx_graph,
//     keychain::{self, KeychainTxOutIndex},
//     local_chain::LocalChain,
//     miniscript::{Descriptor, DescriptorPublicKey}, spk_client::SyncRequest, tx_graph::{self, TxGraph}, Anchor, BlockId, ConfirmationTimeHeightAnchor, IndexedTxGraph
// };
use bdk_electrum::{
    bdk_chain::{
        bitcoin::{
            self, bip32,
            hashes::{self, Hash},
            BlockHash, Network, ScriptBuf, TxOut,
        },
        keychain::KeychainTxOutIndex,
        local_chain::{CheckPoint, LocalChain},
        miniscript::{Descriptor, DescriptorPublicKey},
        spk_client::SyncRequest,
        tx_graph::{self, TxGraph},
        Anchor, BlockId, ConfirmationHeightAnchor, IndexedTxGraph,
    },
    electrum_client::{Client as ElectrumClient, ElectrumApi},
    ElectrumExt,
};

use crate::{database::DatabaseConnection, descriptors, BitcoinInterface};

// The difference between the derivation index of the last seen used address and the last stored
// address in the database addresses mapping.
pub const ADDRESS_LOOK_AHEAD: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeychainType {
    Deposit,
    Change,
}

pub const ALL_KEYCHAINS: [KeychainType; 2] = [KeychainType::Deposit, KeychainType::Change];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockInfo {
    pub height: i32,
    //pub time: u32,
    pub hash: bitcoin::BlockHash,
}

impl Anchor for BlockInfo {
    fn anchor_block(&self) -> BlockId {
        BlockId {
            height: self.height.try_into().expect("height must fit into u32"),
            hash: self.hash,
        }
    }

    fn confirmation_height_upper_bound(&self) -> u32 {
        self.height.try_into().expect("height must fit into u32")
    }
}

impl From<ConfirmationHeightAnchor> for BlockInfo {
    fn from(anchor: ConfirmationHeightAnchor) -> Self {
        assert_eq!(
            anchor.confirmation_height, anchor.anchor_block.height,
            "TODO: enter message"
        );
        let anchor = anchor.anchor_block;
        Self {
            height: anchor.height.try_into().expect("height must fit into i32"),
            hash: anchor.hash,
        }
    }
}

pub struct BdkWallet {
    pub graph: IndexedTxGraph<BlockInfo, KeychainTxOutIndex<KeychainType>>,
    pub network: Network,
    // pub reorg_height: Option<u32>,
    /// Height of the next block to receive.
    // pub next_height: u32,
    pub local_chain: LocalChain,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Coin {
    pub outpoint: bitcoin::OutPoint,
    pub block_info: Option<BlockInfo>,
    pub amount: bitcoin::Amount,
    pub derivation_index: bip32::ChildNumber,
    pub is_change: bool,
    pub is_immature: bool,
    pub spend_txid: Option<bitcoin::Txid>,
    pub spend_block: Option<BlockInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Transaction {
    pub txid: bitcoin::Txid,
    pub tx: bitcoin::Transaction,
}

// TODO: These are copied from poller. Make common function?
// #[derive(Debug, Clone, Copy)]
// enum TipUpdate {
//     // The best block is still the same as in the previous poll.
//     Same,
//     // There is a new best block that extends the same chain.
//     Progress(BlockChainTip),
//     // There is a new best block that extends a chain which does not contain our former tip.
//     Reorged(BlockChainTip),
// }

// fn prev_tip(db_conn: &mut Box<dyn DatabaseConnection>) -> CheckPoint {
//     db_conn
//         .chain_tip()
//         .map(|tip|
//             CheckPoint::new(BlockId {
//                 height: tip.height.try_into().expect("TODO"),
//                 hash: tip.hash,
//             })
//         ).expect("TODO")
// }

/// TODO
fn local_chain_from_db(
    db_conn: &mut Box<dyn DatabaseConnection>,
    client: &impl ElectrumApi,
) -> LocalChain {
    //let a = LocalChain::from_genesis_hash(hash)
    let genesis_hash = client
        .block_header(0)
        .expect("Genesis block hash must always be there")
        .block_hash();
    let mut local_chain = LocalChain::from_genesis_hash(genesis_hash).0;
    let prev_tip = db_conn
        .chain_tip()
        .map(|tip| {
            CheckPoint::new(BlockId {
                height: tip.height.try_into().expect("TODO"),
                hash: tip.hash,
            })
        })
        .expect("TODO");
    let _ = local_chain.insert_block(prev_tip.block_id());
    local_chain
}

/// TODO
// fn construct_update_tip_for_chain(
//     client: &impl ElectrumApi,
//     local_chain: LocalChain,
// ) -> Result<(CheckPoint, Option<u32>), Error> {
//     let prev_tip = local_chain.tip();
//     construct_update_tip(client, prev_tip)
// }

/// Return a [`CheckPoint`] of the latest tip, that connects with `prev_tip`.
// fn construct_update_tip(
//     client: &impl ElectrumApi,
//     prev_tip: CheckPoint,
// ) -> Result<(CheckPoint, Option<u32>), Error> {
//     const CHAIN_SUFFIX_LENGTH: u32 = 8;
//     let HeaderNotification { height, .. } = client.block_headers_subscribe()?;
//     let new_tip_height = height as u32;

//     // If electrum returns a tip height that is lower than our previous tip, then checkpoints do
//     // not need updating. We just return the previous tip and use that as the point of agreement.
//     if new_tip_height < prev_tip.height() {
//         return Ok((prev_tip.clone(), Some(prev_tip.height())));
//     }

//     // Atomically fetch the latest `CHAIN_SUFFIX_LENGTH` count of blocks from Electrum. We use this
//     // to construct our checkpoint update.
//     let mut new_blocks = {
//         let start_height = new_tip_height.saturating_sub(CHAIN_SUFFIX_LENGTH - 1);
//         let hashes = client
//             .block_headers(start_height as _, CHAIN_SUFFIX_LENGTH as _)?
//             .headers
//             .into_iter()
//             .map(|h| h.block_hash());
//         (start_height..).zip(hashes).collect::<BTreeMap<u32, _>>()
//     };

//     // Find the "point of agreement" (if any).
//     let agreement_cp = {
//         let mut agreement_cp = Option::<CheckPoint>::None;
//         for cp in prev_tip.iter() {
//             let cp_block = cp.block_id();
//             let hash = match new_blocks.get(&cp_block.height) {
//                 Some(&hash) => hash,
//                 None => {
//                     assert!(
//                         new_tip_height >= cp_block.height,
//                         "already checked that electrum's tip cannot be smaller"
//                     );
//                     let hash = client.block_header(cp_block.height as _)?.block_hash();
//                     new_blocks.insert(cp_block.height, hash);
//                     hash
//                 }
//             };
//             if hash == cp_block.hash {
//                 agreement_cp = Some(cp);
//                 break;
//             }
//         }
//         agreement_cp
//     };

//     let agreement_height = agreement_cp.as_ref().map(CheckPoint::height);

//     let new_tip = new_blocks
//         .into_iter()
//         // Prune `new_blocks` to only include blocks that are actually new.
//         .filter(|(height, _)| Some(*height) > agreement_height)
//         .map(|(height, hash)| BlockId { height, hash })
//         .fold(agreement_cp, |prev_cp, block| {
//             Some(match prev_cp {
//                 Some(cp) => cp.push(block).expect("must extend checkpoint"),
//                 None => CheckPoint::new(block),
//             })
//         })
//         .expect("must have at least one checkpoint");

//     Ok((new_tip, agreement_height))
// }

// TODO: copied from poller.
// fn new_tip(bit: &impl BitcoinInterface, current_tip: &BlockChainTip) -> TipUpdate {
//     let bitcoin_tip = bit.chain_tip();

//     // If the tip didn't change, there is nothing to update.
//     if current_tip == &bitcoin_tip {
//         return TipUpdate::Same;
//     }

//     if bitcoin_tip.height > current_tip.height {
//         // Make sure we are on the same chain.
//         if bit.is_in_chain(current_tip) {
//             // All good, we just moved forward.
//             return TipUpdate::Progress(bitcoin_tip);
//         }
//     }

//     // Either the new height is lower or the same but the block hash differs. There was a
//     // block chain re-organisation. Find the common ancestor between our current chain and
//     // the new chain and return that. The caller will take care of rewinding our state.
//     log::info!("Block chain reorganization detected. Looking for common ancestor.");
//     if let Some(common_ancestor) = bit.common_ancestor(current_tip) {
//         log::info!(
//             "Common ancestor found: '{}'. Starting rescan from there. Old tip was '{}'.",
//             common_ancestor,
//             current_tip
//         );
//         TipUpdate::Reorged(common_ancestor)
//     } else {
//         log::error!(
//             "Failed to get common ancestor for tip '{}'. Starting over.",
//             current_tip
//         );
//         new_tip(bit, current_tip)
//     }
// }

fn list_transactions(db_conn: &mut Box<dyn DatabaseConnection>) -> Vec<Transaction> {
    let txids = db_conn.list_txids(u32::MIN, u32::MAX, u64::MAX);
    db_conn
        .list_wallet_transactions(&txids)
        .into_iter()
        .map(|(tx, _, _)| Transaction {
            txid: tx.txid(),
            tx,
        })
        .collect()
}

fn list_coins(db_conn: &mut Box<dyn DatabaseConnection>, bit: &impl BitcoinInterface) -> Vec<Coin> {
    let coins = db_conn.coins(&[], &[]);
    let mut hashes = HashMap::<i32, BlockHash>::new();
    coins
        .values()
        .map(|c| {
            let block_info = c.block_info.map(|info| {
                let hash = match hashes.entry(info.height) {
                    Entry::Occupied(o) => *o.get(),
                    Entry::Vacant(v) => {
                        let hash = bit
                            .block_hash(info.height)
                            .expect("coin's block hash must exist");
                        *v.insert(hash)
                    }
                };
                // TODO: change once versions match
                let hash_bdk = hashes::Hash::from_byte_array(*hash.as_raw_hash().as_byte_array());
                BlockInfo {
                    height: info.height,
                    hash: bitcoin::BlockHash::from_raw_hash(hash_bdk),
                }
            });
            let spend_block = c.spend_block.map(|info| {
                let hash = match hashes.entry(info.height) {
                    Entry::Occupied(o) => *o.get(),
                    Entry::Vacant(v) => {
                        let hash = bit
                            .block_hash(info.height)
                            .expect("coin's block hash must exist");
                        *v.insert(hash)
                    }
                };
                // TODO: change once versions match
                let hash_bdk = hashes::Hash::from_byte_array(*hash.as_raw_hash().as_byte_array());
                BlockInfo {
                    height: info.height,
                    hash: bitcoin::BlockHash::from_raw_hash(hash_bdk),
                }
            });
            Coin {
                outpoint: bitcoin::OutPoint::from_str(&format!(
                    "{}:{}",
                    c.outpoint.txid, c.outpoint.vout
                ))
                .unwrap(), // TODO: change once versions match
                block_info,
                spend_block,
                amount: bitcoin::Amount::from_sat(c.amount.to_sat()), // TODO: change once versions match
                is_change: c.is_change,
                derivation_index: bitcoin::bip32::ChildNumber::from_normal_idx(
                    c.derivation_index.into(),
                )
                .unwrap(), // TODO: change once versions match
                is_immature: c.is_immature,
                spend_txid: c
                    .spend_txid
                    .map(|txid| bitcoin::Txid::from_str(&format!("{}", txid)).unwrap()), // TODO: change once versions match
            }
        })
        .collect()
}

// fn wallet_heights(
//     db_conn: &mut Box<dyn DatabaseConnection>,
//     bit: &impl BitcoinInterface,
// ) -> (u32, Option<u32>) {
//     let current_tip = db_conn.chain_tip().expect("Always set at first startup");
//     match new_tip(bit, &current_tip) {
//         TipUpdate::Same => (current_tip.height.try_into().unwrap(), None),
//         TipUpdate::Progress(new_tip) => (new_tip.height.try_into().unwrap(), None),
//         TipUpdate::Reorged(new_tip) => (
//             new_tip.height.try_into().unwrap(),
//             Some(new_tip.height.try_into().unwrap()),
//         ),
//     }
// }

// Get the scriptpubkey at this derivation index from the corresponding keychain efficiently.
fn get_spk_from_wallet(
    bdk_wallet: &BdkWallet,
    internal_desc: &Descriptor<DescriptorPublicKey>,
    external_desc: &Descriptor<DescriptorPublicKey>,
    der_index: bip32::ChildNumber,
    is_change: bool,
) -> ScriptBuf {
    // Try to get it from the BDK wallet cache first, failing that derive it from the appropriate
    // descriptor.
    let chain_kind = if is_change {
        KeychainType::Change
    } else {
        KeychainType::Deposit
    };
    if let Some(spk) = bdk_wallet
        .graph
        .index
        .spk_at_index(chain_kind, der_index.into())
    {
        spk.to_owned()
    } else {
        let desc = if is_change {
            &internal_desc
        } else {
            &external_desc
        };
        desc.at_derivation_index(der_index.into())
            .expect("Not multipath and index isn't hardened.")
            .script_pubkey()
    }
}

// Apply existing data from the database to the BDK wallet.
pub fn bdk_wallet_from_db(
    db_conn: &mut Box<dyn DatabaseConnection>,
    client: &impl ElectrumApi,
    descs: &[descriptors::SinglePathLianaDesc],
    existing_coins: Vec<Coin>,
    existing_txs: Vec<Transaction>,
) -> Result<BdkWallet, Box<dyn std::error::Error>> {
    let network = db_conn.network();
    // Transform the multipath descriptor we store in DB in two descriptors, as expected by
    // BDK.
    #[allow(clippy::get_first)]
    let external_desc = descs
        .get(0)
        .expect("Always multipath desc in DB")
        .as_descriptor_public_key();
    let internal_desc = descs
        .get(1)
        .expect("Always multipath desc in DB")
        .as_descriptor_public_key();

    //let (next_height, reorg_height) = wallet_heights(db_conn, bit);
    let local_chain = local_chain_from_db(db_conn, client);

    let mut bdk_wallet = BdkWallet {
        graph: {
            let mut indexer = KeychainTxOutIndex::<KeychainType>::new(ADDRESS_LOOK_AHEAD);
            let _ = indexer.insert_descriptor(KeychainType::Deposit, external_desc.clone());
            let _ = indexer.insert_descriptor(KeychainType::Change, internal_desc.clone());
            IndexedTxGraph::new(indexer)
        },
        network,
        // reorg_height,
        // next_height,
        local_chain,
    };
    // Update the last used derivation index for both change and receive addresses. Note we store
    // in DB the next derivation to be used for each, hence the -1 here.
    let mut last_active_indices = BTreeMap::new();
    let deposit_index: u32 = db_conn.receive_index().into();
    last_active_indices.insert(KeychainType::Deposit, deposit_index.saturating_sub(1));
    let change_index: u32 = db_conn.change_index().into();
    last_active_indices.insert(KeychainType::Change, change_index.saturating_sub(1));
    let _ = bdk_wallet
        .graph
        .index
        .reveal_to_target_multi(&last_active_indices);

    // Update the existing coins and transactions information using a TxGraph changeset.
    let mut graph_cs = tx_graph::ChangeSet::default();
    for tx in existing_txs {
        graph_cs.txs.insert(Arc::new(tx.tx));
    }
    for coin in existing_coins {
        // First of all insert the txout itself.
        let script_pubkey = get_spk_from_wallet(
            &bdk_wallet,
            &internal_desc,
            &external_desc,
            coin.derivation_index,
            coin.is_change,
        );
        let txout = TxOut {
            script_pubkey,
            value: coin.amount,
        };
        graph_cs.txouts.insert(coin.outpoint, txout);
        // If the coin's deposit transaction is confirmed, tell BDK by inserting an anchor.
        // Otherwise, we could insert a last seen timestamp but we don't have such data stored in
        // the table.
        if let Some(block_info) = coin.block_info {
            graph_cs.anchors.insert((block_info, coin.outpoint.txid));
        }
        // If the coin's spending transaction is confirmed, do the same.
        if let Some(spend_block_info) = coin.spend_block {
            let spend_txid = coin.spend_txid.expect("Must be present if confirmed.");
            graph_cs.anchors.insert((spend_block_info, spend_txid));
        }
    }
    let mut graph = TxGraph::default();
    graph.apply_changeset(graph_cs);
    let _ = bdk_wallet.graph.apply_update(graph);
    Ok(bdk_wallet)
}

fn sync_through_bdk(
    db_conn: &mut Box<dyn DatabaseConnection>,
    bdk_wallet: &mut BdkWallet,
    client: &ElectrumClient,
) {
    let chain_tip = bdk_wallet.local_chain.tip();
    let request =
        SyncRequest::from_chain_tip(chain_tip.clone()).cache_graph_txs(bdk_wallet.graph.graph());

    //let sync_result = client.sync(request, 10, true).unwrap().with_confirmation_height_anchor();
    let sync_result = client
        .sync(request, 10, true)
        .unwrap()
        .with_confirmation_time_height_anchor(client)
        .unwrap();

    //let mut local_chain = bdk_wallet.local_chain;
    let _ = bdk_wallet
        .local_chain
        .apply_update(sync_result.chain_update)
        .unwrap();

    let graph_update = sync_result.graph_update.map_anchors(|a| BlockInfo {
        hash: a.anchor_block.hash,
        height: a.confirmation_height.try_into().unwrap(),
    });
    let _ = bdk_wallet.graph.apply_update(graph_update);
}

pub struct Electrum {
    /// Client for generalistic calls.
    pub client: ElectrumClient, // TODO: remove pub
    pub prev_tip: CheckPoint,
    pub graph: IndexedTxGraph<BlockInfo, KeychainTxOutIndex<KeychainType>>,
}

impl Electrum {}
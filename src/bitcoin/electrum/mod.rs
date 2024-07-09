use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap},
    convert::TryInto,
    str::FromStr,
};

use ::miniscript::bitcoin::{hashes::Hash, BlockHash, Network};
use bdk_chain::{
    bitcoin::{
        self, bip32,
        hashes::{self},
        ScriptBuf, TxOut,
    },
    keychain::KeychainTxOutIndex,
    miniscript::{self, Descriptor, DescriptorPublicKey},
    tx_graph::{self, TxGraph},
    Anchor, BlockId, ConfirmationTimeHeightAnchor, IndexedTxGraph,
};
use bdk_electrum::electrum_client::Client as ElectrumClient;

use crate::{bitcoin::BlockChainTip, database::DatabaseConnection, BitcoinInterface};

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

impl From<ConfirmationTimeHeightAnchor> for BlockInfo {
    fn from(anchor: ConfirmationTimeHeightAnchor) -> Self {
        assert_eq!(
            anchor.confirmation_height, anchor.anchor_block.height,
            "TODO: enter message"
        );
        //let time = anchor.confirmation_time;
        let anchor = anchor.anchor_block;
        Self {
            height: anchor.height.try_into().expect("height must fit into i32"),
            //time: time.try_into().expect("time must fit into u32"),
            hash: anchor.hash,
        }
    }
}

pub struct BdkWallet {
    pub graph: IndexedTxGraph<BlockInfo, KeychainTxOutIndex<KeychainType>>,
    pub network: Network,
    pub reorg_height: Option<u32>,
    /// Height of the next block to receive.
    pub next_height: u32,
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
#[derive(Debug, Clone, Copy)]
enum TipUpdate {
    // The best block is still the same as in the previous poll.
    Same,
    // There is a new best block that extends the same chain.
    Progress(BlockChainTip),
    // There is a new best block that extends a chain which does not contain our former tip.
    Reorged(BlockChainTip),
}

// TODO: copied from poller.
fn new_tip(bit: &impl BitcoinInterface, current_tip: &BlockChainTip) -> TipUpdate {
    let bitcoin_tip = bit.chain_tip();

    // If the tip didn't change, there is nothing to update.
    if current_tip == &bitcoin_tip {
        return TipUpdate::Same;
    }

    if bitcoin_tip.height > current_tip.height {
        // Make sure we are on the same chain.
        if bit.is_in_chain(current_tip) {
            // All good, we just moved forward.
            return TipUpdate::Progress(bitcoin_tip);
        }
    }

    // Either the new height is lower or the same but the block hash differs. There was a
    // block chain re-organisation. Find the common ancestor between our current chain and
    // the new chain and return that. The caller will take care of rewinding our state.
    log::info!("Block chain reorganization detected. Looking for common ancestor.");
    if let Some(common_ancestor) = bit.common_ancestor(current_tip) {
        log::info!(
            "Common ancestor found: '{}'. Starting rescan from there. Old tip was '{}'.",
            common_ancestor,
            current_tip
        );
        TipUpdate::Reorged(common_ancestor)
    } else {
        log::error!(
            "Failed to get common ancestor for tip '{}'. Starting over.",
            current_tip
        );
        new_tip(bit, current_tip)
    }
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

fn wallet_heights(
    db_conn: &mut Box<dyn DatabaseConnection>,
    bit: &impl BitcoinInterface,
) -> (u32, Option<u32>) {
    let current_tip = db_conn.chain_tip().expect("Always set at first startup");
    match new_tip(bit, &current_tip) {
        TipUpdate::Same => (current_tip.height.try_into().unwrap(), None),
        TipUpdate::Progress(new_tip) => (new_tip.height.try_into().unwrap(), None),
        TipUpdate::Reorged(new_tip) => (
            new_tip.height.try_into().unwrap(),
            Some(new_tip.height.try_into().unwrap()),
        ),
    }
}

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
    bit: &impl BitcoinInterface,
    descs: &[miniscript::Descriptor<miniscript::DescriptorPublicKey>],
    existing_coins: Vec<Coin>,
    existing_txs: Vec<Transaction>,
) -> Result<BdkWallet, Box<dyn std::error::Error>> {
    let network = db_conn.network();
    // Transform the multipath descriptor we store in DB in two descriptors, as expected by
    // BDK.
    #[allow(clippy::get_first)]
    let external_desc = descs.get(0).expect("Always multipath desc in DB");
    let internal_desc = descs.get(1).expect("Always multipath desc in DB");

    let (next_height, reorg_height) = wallet_heights(db_conn, bit);

    let mut bdk_wallet = BdkWallet {
        graph: {
            let mut indexer = KeychainTxOutIndex::<KeychainType>::new(ADDRESS_LOOK_AHEAD);
            indexer.add_keychain(KeychainType::Deposit, external_desc.clone());
            indexer.add_keychain(KeychainType::Change, internal_desc.clone());
            IndexedTxGraph::new(indexer)
        },
        network,
        reorg_height,
        next_height,
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
        graph_cs.txs.insert(tx.tx);
    }
    for coin in existing_coins {
        // First of all insert the txout itself.
        let script_pubkey = get_spk_from_wallet(
            &bdk_wallet,
            internal_desc,
            external_desc,
            coin.derivation_index,
            coin.is_change,
        );
        let txout = TxOut {
            script_pubkey,
            value: coin.amount.to_sat(),
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

fn update_wallet(
    bdk_wallet: &mut BdkWallet,
    db_conn: &mut Box<dyn DatabaseConnection>,
    electrum_api: &ElectrumClient,
    bit: &impl BitcoinInterface,
) {
}

pub struct Electrum {
    /// Client for generalistic calls.
    pub client: ElectrumClient, // TODO: remove pub
}

impl Electrum {}

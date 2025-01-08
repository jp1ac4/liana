use std::convert::TryInto;

use bdk_chain::{bitcoin, BlockId};

use crate::bitcoin::BlockChainTip;

pub fn height_u32_from_i32(height: i32) -> u32 {
    height.try_into().expect("height must fit into u32")
}

pub fn height_i32_from_u32(height: u32) -> i32 {
    height.try_into().expect("height must fit into i32")
}

pub fn height_i32_from_usize(height: usize) -> i32 {
    height.try_into().expect("height must fit into i32")
}

pub fn height_usize_from_i32(height: i32) -> usize {
    height.try_into().expect("height must fit into usize")
}

pub fn block_id_from_tip(tip: BlockChainTip) -> BlockId {
    BlockId {
        height: height_u32_from_i32(tip.height),
        hash: tip.hash,
    }
}

pub fn tip_from_block_id(id: BlockId) -> BlockChainTip {
    BlockChainTip {
        height: height_i32_from_u32(id.height),
        hash: id.hash,
    }
}

/// Get the transaction's outpoints.
pub fn outpoints_from_tx(tx: &bitcoin::Transaction) -> Vec<bitcoin::OutPoint> {
    let txid = tx.compute_txid();
    (0..tx.output.len())
        .map(|i| {
            bitcoin::OutPoint::new(txid, i.try_into().expect("num tx outputs must fit in u32"))
        })
        .collect::<Vec<_>>()
}

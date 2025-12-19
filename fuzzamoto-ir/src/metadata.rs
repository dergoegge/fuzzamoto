use serde::{Deserialize, Serialize};

use crate::{GetBlockTxn, RecentBlock};

/// The runtime data observed during the course of harness execution
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct PerTestcaseMetadata {
    pub block_txn_request: Vec<GetBlockTxn>,
    pub recent_blocks: Vec<RecentBlock>,
}

impl PerTestcaseMetadata {
    pub fn new() -> Self {
        Self {
            block_txn_request: Vec::new(),
            recent_blocks: Vec::new(),
        }
    }

    pub fn block_txn_request(&self) -> &[GetBlockTxn] {
        &self.block_txn_request
    }

    pub fn recent_blocks(&self) -> &[RecentBlock] {
        &self.recent_blocks
    }

    pub fn add_block_tx_request(&mut self, req: GetBlockTxn) {
        self.block_txn_request.push(req);
    }

    pub fn add_recent_blocks(&mut self, blocks: Vec<RecentBlock>) {
        self.recent_blocks = blocks;
        self.recent_blocks.sort();
    }
}

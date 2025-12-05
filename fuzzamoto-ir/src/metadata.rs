use serde::{Deserialize, Serialize};

use crate::GetBlockTxn;

/// The runtime data observed during the course of harness execution
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct PerTestcaseMetadata {
    pub block_txn_request: Vec<GetBlockTxn>,
}

impl PerTestcaseMetadata {
    pub fn new() -> Self {
        Self {
            block_txn_request: Vec::new(),
        }
    }

    pub fn block_txn_request(&self) -> &[GetBlockTxn] {
        &self.block_txn_request
    }

    pub fn add_block_tx_request(&mut self, req: GetBlockTxn) {
        self.block_txn_request.push(req);
    }
}

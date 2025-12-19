use serde::{Deserialize, Serialize};

use crate::{GetBlockTxn, MempoolTxo, TxoMetadata};

/// The runtime data observed during the course of harness execution
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct PerTestcaseMetadata {
    pub block_txn_request: Vec<GetBlockTxn>,
    pub txo_metadata: TxoMetadata,
}

impl PerTestcaseMetadata {
    pub fn new() -> Self {
        Self {
            block_txn_request: Vec::new(),
            txo_metadata: TxoMetadata::default(),
        }
    }

    pub fn block_txn_request(&self) -> &[GetBlockTxn] {
        &self.block_txn_request
    }

    pub fn txo_metadata(&self) -> &TxoMetadata {
        &self.txo_metadata
    }

    pub fn txo_metadata_mut(&mut self) -> &mut TxoMetadata {
        &mut self.txo_metadata
    }

    pub fn add_block_tx_request(&mut self, req: GetBlockTxn) {
        self.block_txn_request.push(req);
    }

    pub fn add_txo_entry(&mut self, txo_entry: Vec<MempoolTxo>) {
        let txo_metadata = TxoMetadata {
            txo_entry,
            choice: None,
        };
        self.txo_metadata = txo_metadata;
    }
}

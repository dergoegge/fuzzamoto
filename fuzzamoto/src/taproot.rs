use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaprootContext {
    pub txos: Vec<TaprootTxo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaprootTxo {
    pub outpoint: ([u8; 32], u32),
    pub value: u64,
    pub spend_info: TaprootSpendInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaprootKeypair {
    pub secret_key: [u8; 32],
    pub public_key: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaprootSpendInfo {
    pub keypair: TaprootKeypair,
    pub merkle_root: Option<[u8; 32]>,
    pub output_key: [u8; 32],
    pub output_key_parity: u8,
    pub script_pubkey: Vec<u8>,
    pub leaves: Vec<TaprootLeaf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaprootLeaf {
    pub version: u8,
    pub script: Vec<u8>,
    pub merkle_branch: Vec<[u8; 32]>,
}

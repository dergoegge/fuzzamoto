use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct TaprootKeypair {
    pub secret_key: [u8; 32],
    pub public_key: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct TaprootSpendInfo {
    pub keypair: TaprootKeypair,
    pub merkle_root: Option<[u8; 32]>,
    pub output_key: [u8; 32],
    pub output_key_parity: u8,
    pub script_pubkey: Vec<u8>,
    pub leaves: Vec<TaprootLeaf>,
    /// If present, this tapleaf is the fixed script-path used to spend; otherwise key-path.
    /// TODO: in the future, consider allowing selecting a non-first leaf at output build time to exercise multi-leaf coverage.
    #[serde(default)]
    pub selected_leaf: Option<TaprootLeaf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct TaprootLeaf {
    pub version: u8,
    pub script: Vec<u8>,
    pub merkle_branch: Vec<[u8; 32]>,
}

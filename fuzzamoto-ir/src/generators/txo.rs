use std::path::PathBuf;

use super::{Generator, GeneratorError, GeneratorResult};
use crate::{Operation, PerTestcaseMetadata, ProgramBuilder};
use rand::{Rng, RngCore};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Txo {
    pub outpoint: ([u8; 32], u32),
    pub value: u64,
    pub script_pubkey: Vec<u8>,
    pub spending_script_sig: Vec<u8>,
    pub spending_witness: Vec<Vec<u8>>,
}

/// `TxoGenerator` generates a new `LoadTxo` instruction into a program.
pub struct TxoGenerator {
    available_txos: Vec<Txo>,
}

impl TxoGenerator {
    pub fn new(available_txos: Vec<Txo>) -> Self {
        Self { available_txos }
    }

    pub fn from_file(path: &PathBuf) -> Self {
        let bytes = std::fs::read(path).unwrap();
        let txos: Vec<Txo> = postcard::from_bytes(&bytes).unwrap();
        Self::new(txos)
    }
}

impl<R: RngCore> Generator<R> for TxoGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        if self.available_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        }

        let available_txo = &self.available_txos[rng.gen_range(0..self.available_txos.len())];
        builder.force_append(
            vec![],
            Operation::LoadTxo {
                outpoint: available_txo.outpoint.clone(),
                value: available_txo.value,
                script_pubkey: available_txo.script_pubkey.clone(),
                spending_script_sig: available_txo.spending_script_sig.clone(),
                spending_witness: available_txo.spending_witness.clone(),
            },
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TxoGenerator"
    }
}

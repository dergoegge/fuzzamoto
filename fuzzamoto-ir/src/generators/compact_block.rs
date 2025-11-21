use super::{GeneratorError, GeneratorResult};
use crate::{
    Instruction, Operation, Variable,
    generators::{Generator, ProgramBuilder},
};
use bitcoin::hashes::Hash;
use rand::{Rng, RngCore};

// I need to define this because bitcoin crate's one doesn't implement serialize and deserialize
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, Hash)]
pub struct BlockTransactionsRequestRecved {
    pub conn: usize,
    pub hash: [u8; 32],
    pub indexes: Vec<u64>,
}

impl BlockTransactionsRequestRecved {
    pub fn new(conn: usize, value: bitcoin::bip152::BlockTransactionsRequest) -> Self {
        Self {
            conn,
            hash: value.block_hash.as_raw_hash().to_byte_array(),
            indexes: value.indexes,
        }
    }

    pub fn conn(&self) -> usize {
        self.conn
    }

    pub fn hash(&self) -> &[u8; 32] {
        &self.hash
    }

    pub fn indexes(&self) -> &[u64] {
        &self.indexes
    }
}

/// `CompactBlockGenerator` generates a new `cmpctblock` message.
#[derive(Debug, Default)]
pub struct CompactBlockGenerator;

pub const MAX_TX_SIZE: usize = 32;

impl<R: RngCore> Generator<R> for CompactBlockGenerator {
    fn generate(&self, builder: &mut ProgramBuilder, rng: &mut R) -> GeneratorResult {
        // choose a block upon which we build the compact block
        let Some(block) = builder.get_random_variable(rng, Variable::Block) else {
            return Err(GeneratorError::MissingVariables);
        };

        let connection_var = builder.get_or_create_random_connection(rng);

        let nonce = rng.gen_range(0..u64::MAX);
        let nonce_var = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::LoadNonce(nonce),
            })
            .expect("Inserting BeginBuildCmpctBlock should always succeed")
            .pop()
            .expect("BeginBuildCmpctBlock should always produce a var");

        let version = rng.gen_range(1..=2);
        let version_var = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::LoadTxVersion(version),
            })
            .expect("Inserting BeginBuildCmpctBlock should always succeed")
            .pop()
            .expect("BeginBuildCmpctBlock should always produce a var");

        let mut prefill_vec: Vec<usize> = vec![];
        for _ in 0..32 {
            prefill_vec.push(rng.gen_range(0..=MAX_TX_SIZE));
        }

        let prefill_var = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::LoadTxIndexes {
                    indexes: prefill_vec,
                },
            })
            .expect("Inserting BeginBuildCmpctBlock should always succeed")
            .pop()
            .expect("BeginBuildCmpctBlock should always produce a var");
        let mut_cmpct_block = builder
            .append(Instruction {
                inputs: vec![
                    block.index,
                    nonce_var.index,
                    version_var.index,
                    prefill_var.index,
                ],
                operation: Operation::BeginBuildCmpctBlock,
            })
            .expect("Inserting BeginBuildCmpctBlock should always succeed")
            .pop()
            .expect("BeginBuildCmpctBlock should always produce a var");

        let cmpct_block = builder
            .append(Instruction {
                inputs: vec![mut_cmpct_block.index],
                operation: Operation::EndBuildCmpctBlock,
            })
            .expect("Inserting EndBuildCmpctBlock should always succeed")
            .pop()
            .expect("EndBuildCmpctBlock should always produce a var");

        builder
            .append(Instruction {
                inputs: vec![connection_var.index, cmpct_block.index],
                operation: Operation::SendCompactBlock,
            })
            .expect("Inserting SendCompactBlock should always succeed");

        Ok(())
    }

    fn name(&self) -> &'static str {
        "CompactBlockGenerator"
    }
}

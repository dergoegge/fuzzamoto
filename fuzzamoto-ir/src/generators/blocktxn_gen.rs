use super::{GeneratorError, GeneratorResult};
use crate::{
    Instruction, Operation, Variable,
    generators::{Generator, ProgramBuilder, compact_block::MAX_TX_SIZE},
};
use rand::{Rng, RngCore};

/// `Blocktxn` generates a new blocktxn message;
#[derive(Debug, Default)]
pub struct BlockTxnGenerator;

impl<R: RngCore> Generator<R> for BlockTxnGenerator {
    fn generate(&self, builder: &mut ProgramBuilder, rng: &mut R) -> GeneratorResult {
        // choose a block upon which we build the compact block
        let Some(block) = builder.get_random_variable(rng, Variable::Block) else {
            return Err(GeneratorError::MissingVariables);
        };

        // just choose a random connection.
        let conn = builder.get_or_create_random_connection(rng);

        let mut indexes_vec: Vec<usize> = vec![];
        for _ in 0..32 {
            indexes_vec.push(rng.gen_range(0..=MAX_TX_SIZE));
        }

        let indexes_var = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::LoadTxIndexes {
                    indexes: indexes_vec,
                },
            })
            .expect("Inserting LoadTxIndexes should always succeed")
            .pop()
            .expect("LoadTxIndexes should always produce a var");

        let blocktxn = builder
            .append(Instruction {
                inputs: vec![block.index, indexes_var.index],
                operation: Operation::BuildBIP152BlockTxReq,
            })
            .expect("Inserting BuildBIP152BlockTxReq should always succeed")
            .pop()
            .expect("BuildBIP152BlockTxReq should always produce a var");
        builder
            .append(Instruction {
                inputs: vec![conn.index, blocktxn.index],
                operation: Operation::SendBlockTxn,
            })
            .expect("Inserting SendBlockTxn should always succeed");
        Ok(())
    }

    fn name(&self) -> &'static str {
        "BlockTxnGenerator"
    }
}

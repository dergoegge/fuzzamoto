use crate::{
    Generator, GeneratorError, GeneratorResult, Instruction, Operation, PerTestcaseMetadata,
    ProgramBuilder, Variable,
};
use rand::{Rng, RngCore};

/// `BlockTxnGenerator` inserts `blocktxn` operation in response to the `getblocktxn` message
#[derive(Debug, Copy, Clone, Default)]
pub struct BlockTxnGenerator;

impl<R: RngCore> Generator<R> for BlockTxnGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        meta: Option<&PerTestcaseMetadata>,
    ) -> GeneratorResult {
        if let Some(meta) = meta
            && !meta.block_txn_request().is_empty()
        {
            let insertion_point = builder.instructions.len();
            assert_eq!(
                builder
                    .instructions
                    .get(insertion_point - 1)
                    .expect("The insertion point should exist")
                    .operation,
                Operation::SendCompactBlock
            );
            let blocktxn_req = meta.block_txn_request();
            let choice = blocktxn_req
                .iter()
                .position(|x| x.triggering_instruction_index == insertion_point - 1)
                .expect("Triggering instruction not found");

            let block_variable = blocktxn_req[choice].block_variable;
            let mut_block_txn = builder
                .append(Instruction {
                    inputs: vec![block_variable],
                    operation: Operation::BeginBuildBlockTxn,
                })
                .expect("Inserting BeginBuildBlockTxn should always succeed")
                .pop()
                .expect("BeginBuildBlockTxn should always produce a var");

            for tx in &blocktxn_req[choice].tx_indices_variables {
                builder
                    .append(Instruction {
                        inputs: vec![mut_block_txn.index, *tx],
                        operation: Operation::AddTxToBlockTxn,
                    })
                    .expect("Inserting AddTxToBlockTxn should always suceed");
            }

            let block_txn = builder
                .append(Instruction {
                    inputs: vec![mut_block_txn.index],
                    operation: Operation::EndBuildBlockTxn,
                })
                .expect("Inserting EndBuildBlockTxn should always succeed")
                .pop()
                .expect("EndBuildBlockTxn should always produce a var");

            let connection = blocktxn_req[choice].connection_index;
            builder
                .append(Instruction {
                    inputs: vec![connection, block_txn.index],
                    operation: Operation::SendBlockTxn,
                })
                .expect("Inserting SendBlockTxn should always succeed");
        } else {
            let connection_var = builder.get_or_create_random_connection(rng);

            // choose a block upon which we build the blocktxn
            let Some(block) = builder.get_random_variable(rng, Variable::Block) else {
                return Err(GeneratorError::MissingVariables);
            };

            let mut_block_txn = builder
                .append(Instruction {
                    inputs: vec![block.index],
                    operation: Operation::BeginBuildBlockTxn,
                })
                .expect("Inserting BeginBuildBlockTxn should always succeed")
                .pop()
                .expect("BeginBuildBlockTxn should always produce a var");

            let Some(tx) = builder.get_random_variable(rng, Variable::ConstTx) else {
                return Err(GeneratorError::MissingVariables);
            };
            builder
                .append(Instruction {
                    inputs: vec![mut_block_txn.index, tx.index],
                    operation: Operation::AddTxToBlockTxn,
                })
                .expect("Inserting AddTxToBlockTxn should always suceed");

            let block_txn = builder
                .append(Instruction {
                    inputs: vec![mut_block_txn.index],
                    operation: Operation::EndBuildBlockTxn,
                })
                .expect("Inserting EndBuildBlockTxn should always succeed")
                .pop()
                .expect("EndBuildBlockTxn should always produce a var");

            builder
                .append(Instruction {
                    inputs: vec![connection_var.index, block_txn.index],
                    operation: Operation::SendBlockTxn,
                })
                .expect("Inserting SendBlockTxn should always succeed");
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "BlockTxnGenerator"
    }

    fn choose_index(
        &self,
        program: &crate::Program,
        rng: &mut R,
        meta: Option<&mut PerTestcaseMetadata>,
    ) -> Option<usize> {
        if let Some(meta) = meta
            && !meta.block_txn_request().is_empty()
        {
            let blocktxn_req = meta.block_txn_request();
            let choice = rng.gen_range(0..blocktxn_req.len());
            let insertion_point = blocktxn_req[choice].triggering_instruction_index + 1;
            Some(insertion_point)
        } else {
            program
                .get_random_instruction_index(rng, <Self as Generator<R>>::requested_context(self))
        }
    }
}

impl BlockTxnGenerator {
    pub fn new() -> Self {
        Self {}
    }
}

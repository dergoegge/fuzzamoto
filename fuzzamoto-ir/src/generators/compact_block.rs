use super::{GeneratorError, GeneratorResult};
use crate::{
    Instruction, Operation, PerTestcaseMetadata, Variable,
    generators::{Generator, ProgramBuilder},
};
use rand::{Rng, RngCore, seq::SliceRandom};

/// `CompactBlockGenerator` generates a new `cmpctblock` message.
#[derive(Debug, Default)]
pub struct CompactBlockGenerator;

impl<R: RngCore> Generator<R> for CompactBlockGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&PerTestcaseMetadata>,
    ) -> GeneratorResult {
        // Choose a block upon which we build the compact block
        let Some(block) = builder.get_random_variable(rng, &Variable::Block) else {
            return Err(GeneratorError::MissingVariables);
        };

        let Some((block_transactions_idx, tx_var_indices)) = builder.get_block_vars(block.index)
        else {
            return Err(GeneratorError::MissingVariables);
        };

        // Collect into an owned vec before the loop since we need to call
        // builder.append() which takes &mut self.
        let tx_var_indices: Vec<usize> = tx_var_indices.clone();
        let num_block_txs = tx_var_indices.len();

        let connection_var = builder.get_or_create_random_connection(rng);

        let nonce_var = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::LoadNonce(rng.gen_range(0..u64::MAX)),
            })
            .expect("LoadNonce should always succeed")
            .pop()
            .expect("LoadNonce should always produce a var");

        // Build the prefill list dynamically from actual transactions in the block.
        let prefill_list = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::BeginPrefillTransactions,
            })
            .expect("BeginPrefillTransactions should always succeed")
            .pop()
            .expect("BeginPrefillTransactions should always produce a var");

        if num_block_txs > 0 {
            // Pick a random number of transactions to prefill, bounded by the
            // actual number of transactions in the block.
            let num_prefill = rng.gen_range(0..=num_block_txs);

            // Shuffle a copy of the known-good tx var indices and take the
            // first num_prefill — every tx is guaranteed to belong to this block.
            let mut shuffled = tx_var_indices.clone();
            shuffled.shuffle(rng);

            for tx_idx in shuffled.into_iter().take(num_prefill) {
                builder
                    .append(Instruction {
                        inputs: vec![prefill_list.index, block_transactions_idx, tx_idx],
                        operation: Operation::AddPrefillTx,
                    })
                    .expect("AddPrefillTx should always succeed");
            }
        }

        let const_prefill = builder
            .append(Instruction {
                inputs: vec![prefill_list.index],
                operation: Operation::EndPrefillTransactions,
            })
            .expect("EndPrefillTransactions should always succeed")
            .pop()
            .expect("EndPrefillTransactions should always produce a var");

        let cmpct_block = builder
            .append(Instruction {
                inputs: vec![block.index, nonce_var.index, const_prefill.index],
                operation: Operation::BuildCompactBlockWithPrefill,
            })
            .expect("BuildCompactBlockWithPrefill should always succeed")
            .pop()
            .expect("BuildCompactBlockWithPrefill should always produce a var");

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

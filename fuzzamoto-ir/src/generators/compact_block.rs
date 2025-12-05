use super::{GeneratorError, GeneratorResult};
use crate::{
    Instruction, Operation, PerTestcaseMetadata, Variable,
    generators::{Generator, ProgramBuilder},
};
use rand::{Rng, RngCore};

/// `CompactBlockGenerator` generates a new `cmpctblock` message.
#[derive(Debug, Default)]
pub struct CompactBlockGenerator;

impl<R: RngCore> Generator<R> for CompactBlockGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
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
            .expect("Inserting BeginBuildCompactBlock should always succeed")
            .pop()
            .expect("BeginBuildCompactBlock should always produce a var");

        let cmpct_block = builder
            .append(Instruction {
                inputs: vec![block.index, nonce_var.index],
                operation: Operation::BuildCompactBlock,
            })
            .expect("Inserting BuildCompactBlock should always succeed")
            .pop()
            .expect("BuildCompactBlock should always produce a var");

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

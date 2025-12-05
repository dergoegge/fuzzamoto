use crate::{
    InstructionContext, Operation, PerTestcaseMetadata, Variable,
    generators::{Generator, ProgramBuilder},
};
use rand::{Rng, RngCore, seq::SliceRandom};

use super::{GeneratorError, GeneratorResult};

/// `WitnessGenerator` generates a new `AddWitness` instruction into a witness stack context
pub struct WitnessGenerator;

impl WitnessGenerator {
    pub fn new() -> Self {
        Self {}
    }
}

impl<R: RngCore> Generator<R> for WitnessGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let Some(witness_var) = builder.get_nearest_variable(Variable::MutWitnessStack) else {
            return Err(GeneratorError::MissingVariables);
        };

        let mut bytes = Vec::new();
        if rng.gen_bool(0.9) {
            bytes.resize(*[0, 1, 2, 4, 8, 32].choose(rng).unwrap(), 0);
        } else {
            bytes.resize(rng.gen_range(0..512), 0);
        }
        rng.fill_bytes(&mut bytes);

        let script_var = builder.force_append_expect_output(vec![], Operation::LoadBytes(bytes));

        builder.force_append(
            vec![witness_var.index, script_var.index],
            Operation::AddWitness,
        );

        Ok(())
    }

    fn name(&self) -> &'static str {
        "WitnessGenerator"
    }

    fn requested_context(&self) -> InstructionContext {
        InstructionContext::WitnessStack
    }
}

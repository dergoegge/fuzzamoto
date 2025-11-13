use super::{Mutator, MutatorError, MutatorResult, Splicer};
use crate::{Program, ProgramBuilder};
use rand::RngCore;

// `ConcatMutator` takes two programs and concatenates them.
pub struct ConcatMutator;

impl<R: RngCore> Mutator<R> for ConcatMutator {
    fn mutate(
        &mut self,
        _program: &mut Program,
        _rng: &mut R,
        _rt_data: &fuzzamoto::RuntimeMetadata,
    ) -> MutatorResult {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ConcatMutator"
    }
}

impl<R: RngCore> Splicer<R> for ConcatMutator {
    fn splice(
        &mut self,
        program: &mut Program,
        splice_with: &Program,
        _rng: &mut R,
    ) -> MutatorResult {
        let mut builder = ProgramBuilder::new(program.context.clone());
        builder
            .append_program_without_threshold(program.clone(), 0usize)
            .map_err(|_| MutatorError::CreatedInvalidProgram)?;
        builder
            .append_program_without_threshold(splice_with.clone(), builder.variable_count())
            .map_err(|_| MutatorError::CreatedInvalidProgram)?;

        *program = builder
            .finalize()
            .map_err(|_| MutatorError::CreatedInvalidProgram)?;
        Ok(())
    }
}

impl ConcatMutator {
    pub fn new() -> Self {
        Self {}
    }
}

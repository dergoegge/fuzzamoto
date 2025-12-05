use crate::{
    Operation, PerTestcaseMetadata, Variable,
    generators::{Generator, ProgramBuilder},
};
use rand::{Rng, RngCore, seq::SliceRandom};

use super::{GeneratorError, GeneratorResult};

/// `CompactFilterQueryGenerator` generates a new `SendGetCFilters`, `SendGetCFHeaders` or
/// `SendGetCFCheckpt` instruction into a global context.
#[derive(Debug, Default)]
pub struct CompactFilterQueryGenerator;

impl<R: RngCore> Generator<R> for CompactFilterQueryGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let Some(header_var) = builder.get_random_variable(rng, Variable::Header) else {
            return Err(GeneratorError::MissingVariables);
        };
        let connection_var = builder.get_or_create_random_connection(rng);
        let compact_filter_type_var =
            builder.force_append_expect_output(vec![], Operation::LoadCompactFilterType(0));
        let block_height_var = builder
            .force_append_expect_output(vec![], Operation::LoadBlockHeight(rng.gen_range(0..200))); // TODO: Find a better way to generate block heights

        let op = [
            Operation::SendGetCFilters,
            Operation::SendGetCFHeaders,
            Operation::SendGetCFCheckpt,
        ]
        .choose(rng)
        .unwrap()
        .clone();
        match op {
            Operation::SendGetCFilters | Operation::SendGetCFHeaders => {
                builder.force_append(
                    vec![
                        connection_var.index,
                        compact_filter_type_var.index,
                        block_height_var.index,
                        header_var.index,
                    ],
                    op,
                );
            }
            Operation::SendGetCFCheckpt => {
                builder.force_append(
                    vec![
                        connection_var.index,
                        compact_filter_type_var.index,
                        header_var.index,
                    ],
                    op,
                );
            }
            _ => unreachable!(),
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "CompactFilterQueryGenerator"
    }
}

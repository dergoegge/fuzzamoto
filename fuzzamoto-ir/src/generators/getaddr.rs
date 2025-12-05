use rand::RngCore;

use crate::{
    Operation, PerTestcaseMetadata,
    generators::{Generator, GeneratorError, GeneratorResult, ProgramBuilder},
};

/// `GetAddrGenerator` emits a single `SendGetAddr` instruction targeting a random connection. We
/// intentionally allow multiple `getaddr` messages in a program so the fuzzer can explore
/// implementations that react differently to repeated requests.
#[derive(Default)]
pub struct GetAddrGenerator;

impl<R: RngCore> Generator<R> for GetAddrGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        if builder.context().num_connections == 0 {
            return Err(GeneratorError::InvalidContext(builder.context().clone()));
        }

        let conn_var = builder.get_or_create_random_connection(rng);
        builder.force_append(vec![conn_var.index], Operation::SendGetAddr);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "GetAddrGenerator"
    }
}

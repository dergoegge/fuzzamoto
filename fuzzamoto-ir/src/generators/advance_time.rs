use std::time::Duration;

use crate::{
    Instruction, Operation, PerTestcaseMetadata, Variable,
    generators::{Generator, GeneratorResult, ProgramBuilder},
};
use rand::{Rng, RngCore, seq::SliceRandom};

/// `AdvanceTimeGenerator` generates programs that advance the time by a random amount
pub struct AdvanceTimeGenerator {
    allowed_time_deltas: Option<Vec<u64>>,
}

impl AdvanceTimeGenerator {
    pub fn new(allowed_time_deltas: Option<Vec<u64>>) -> Self {
        Self {
            allowed_time_deltas,
        }
    }
}

impl Default for AdvanceTimeGenerator {
    fn default() -> Self {
        // Exponential distribution of time deltas
        Self::new(Some(vec![
            1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536,
        ]))
    }
}

impl<R: RngCore> Generator<R> for AdvanceTimeGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        // Find the most recent time variable or load the timestamp from the context
        let time_var = match builder.get_nearest_variable(Variable::Time) {
            Some(v) => v,
            None => builder
                .append(Instruction {
                    inputs: vec![],
                    operation: Operation::LoadTime(builder.context().timestamp),
                })
                .expect("Inserting LoadTime should always succeed")
                .pop()
                .expect("LoadTime should always produce a var"),
        };

        // Load a duration by which to advance time by
        let time_delta = match &self.allowed_time_deltas {
            Some(deltas) => *deltas.choose(rng).unwrap(),
            None => rng.gen_range(0..u64::MAX),
        };
        let duration_var = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::LoadDuration(Duration::from_secs(time_delta)),
            })
            .expect("Inserting LoadDuration should always succeed")
            .pop()
            .expect("LoadDuration should always produce a var");

        // Advance the time variable
        let new_time = builder
            .append(Instruction {
                inputs: vec![time_var.index, duration_var.index],
                operation: Operation::AdvanceTime,
            })
            .expect("Inserting AdvanceTime should always succeed")
            .pop()
            .expect("AdvanceTime should always produce a var");

        // Finally, set the mock time to the advanced time
        builder
            .append(Instruction {
                inputs: vec![new_time.index],
                operation: Operation::SetTime,
            })
            .expect("Inserting SetTime should always succeed");

        Ok(())
    }

    fn name(&self) -> &'static str {
        "AdvanceTimeGenerator"
    }
}

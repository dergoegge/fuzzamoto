use rand::{Rng, RngCore};

use crate::{Operation, Program};

use super::{Mutator, MutatorError, MutatorResult};

const MAX_LEAF_OFFSET: usize = 16;

/// Mutates `TaprootSpendInfoSelectLeaf` instructions by tweaking the stored leaf
/// index. Because the compiler masks the index modulo the number of leaves,
/// simply adding a random offset is sufficient to steer the witness toward a
/// different tapscript branch whenever multiple leaves exist (and is harmless
/// when there is only a single leaf).
pub struct TaprootLeafSelectMutator;

impl TaprootLeafSelectMutator {
    pub fn new() -> Self {
        Self {}
    }
}

impl<R: RngCore> Mutator<R> for TaprootLeafSelectMutator {
    fn mutate(&mut self, program: &mut Program, rng: &mut R) -> MutatorResult {
        let indices: Vec<usize> = program
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(idx, instr)| match instr.operation {
                Operation::TaprootSpendInfoSelectLeaf { .. } => Some(idx),
                _ => None,
            })
            .collect();

        if indices.is_empty() {
            return Err(MutatorError::NoMutationsAvailable);
        }

        let target = indices[rng.gen_range(0..indices.len())];
        let offset = rng.gen_range(1..=MAX_LEAF_OFFSET);
        if let Operation::TaprootSpendInfoSelectLeaf { ref mut index } =
            program.instructions[target].operation
        {
            *index = index.wrapping_add(offset);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TaprootLeafSelectMutator"
    }
}

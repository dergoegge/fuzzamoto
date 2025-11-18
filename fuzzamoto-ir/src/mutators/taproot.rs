use bitcoin::opcodes::all::{OP_CHECKSIG, OP_PUSHNUM_1};
use rand::{Rng, RngCore};

use crate::{Instruction, Operation, Program};

use super::{Mutator, MutatorError, MutatorResult};

const MAX_LEAF_OFFSET: usize = 16;

/// Mutates `TaprootSpendInfoSelectLeaf` instructions by tweaking the stored leaf
/// index. Because the compiler masks the index modulo the number of leaves,
/// simply adding a random offset is enough to steer the witness toward a
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

/// Mutates the raw tapscript bytes feeding `AddTapLeaf` so the same tree can
/// exercise OP_TRUE, CHECKSIG, or OP_SUCCESS-like leaves without regenerating
/// the entire block.
pub struct TaprootScriptMutator;

impl TaprootScriptMutator {
    pub fn new() -> Self {
        Self {}
    }
}

impl<R: RngCore> Mutator<R> for TaprootScriptMutator {
    fn mutate(&mut self, program: &mut Program, rng: &mut R) -> MutatorResult {
        let producers = build_variable_producers(&program.instructions);
        let mut candidates = Vec::new();

        for instr in &program.instructions {
            if let Operation::AddTapLeaf { .. } = instr.operation {
                if let Some(script_var) = instr.inputs.get(1) {
                    if let Some(prod_idx) = producers.get(*script_var) {
                        if matches!(
                            program.instructions[*prod_idx].operation,
                            Operation::LoadBytes(_)
                        ) {
                            candidates.push(*prod_idx);
                        }
                    }
                }
            }
        }

        if candidates.is_empty() {
            return Err(MutatorError::NoMutationsAvailable);
        }

        let script_instr_idx = candidates[rng.gen_range(0..candidates.len())];
        if let Operation::LoadBytes(ref mut bytes) =
            program.instructions[script_instr_idx].operation
        {
            *bytes = random_tapscript(rng);
            Ok(())
        } else {
            Err(MutatorError::CreatedInvalidProgram)
        }
    }

    fn name(&self) -> &'static str {
        "TaprootScriptMutator"
    }
}

fn build_variable_producers(instructions: &[Instruction]) -> Vec<usize> {
    let mut producers = Vec::new();
    for (idx, instr) in instructions.iter().enumerate() {
        let outputs = instr.operation.num_outputs();
        for _ in 0..outputs {
            producers.push(idx);
        }
    }
    producers
}

fn random_tapscript<R: RngCore>(rng: &mut R) -> Vec<u8> {
    match rng.gen_range(0..3) {
        0 => vec![OP_PUSHNUM_1.to_u8()],
        1 => {
            let mut script = Vec::with_capacity(34);
            script.push(32);
            for _ in 0..32 {
                script.push(rng.r#gen());
            }
            script.push(OP_CHECKSIG.to_u8());
            script
        }
        _ => vec![0x50], // Acts like OP_SUCCESSxx to hit early-success paths.
    }
}

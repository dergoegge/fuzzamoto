use super::{Mutator, MutatorResult};
use crate::{Instruction, Operation, PerTestcaseMetadata, Program, ProgramBuilder};
use rand::RngCore;

/// `BlockTxn` inserts `blocktxn` operation in response to the `getblocktxn` message
#[derive(Debug, Copy, Clone, Default)]
pub struct BlockTxnMutator;

impl<R: RngCore> Mutator<R> for BlockTxnMutator {
    fn mutate(
        &mut self,
        program: &mut Program,
        _rng: &mut R,
        meta: Option<&PerTestcaseMetadata>,
    ) -> MutatorResult {
        let mut builder = ProgramBuilder::new(program.context.clone());
        let meta = if let Some(meta) = meta {
            meta
        } else {
            // there is nothing to do if metadata is not there.
            return Ok(());
        };
        let insts = program.instructions.clone();

        if insts.is_empty() {
            return Ok(());
        }

        // we first find the insertion point
        let mut i = 0;
        let (insertion_index, conn, block) = loop {
            // first check if we reached the end
            if i >= insts.len() {
                // if we reach here then we found nothing :< so there's no insertion point
                return Ok(());
            }

            // check if we have room for i+3
            if i + 3 < insts.len()
                && matches!(insts[i].operation, Operation::BeginBuildCmpctBlock)
                && matches!(insts[i + 1].operation, Operation::EndBuildCmpctBlock)
                && matches!(insts[i + 2].operation, Operation::SendCompactBlock)
                && !matches!(
                    &insts[i + 3].operation,
                    Operation::LoadBlockTxnRequestVec { vec: _ }
                )
            {
                // get it from `Operation::SendCompactBlock`
                let conn = insts[i + 2].inputs[0];
                // get it from `Operation::BeginBuildCmpctBlock`
                let block = insts[i].inputs[0];
                i += 3;
                break (i, conn, block);
            }
            i += 1;
        };

        // append the first half
        if !program.instructions.is_empty() {
            let instrs = &program.instructions[..insertion_index];
            builder.append_all(instrs.iter().cloned()).unwrap();
        }

        let variable_threshould = builder.variable_count();

        // add `blocktxn` instruction
        let reqvec = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::LoadBlockTxnRequestVec {
                    vec: meta.block_tx_request().to_vec(),
                },
            })
            .expect("Inserting LoadBlockTxnRequestVec should always succeed")
            .pop()
            .expect("LoadBlockTxnRequestVec should always produce a var");
        let blocktxn = builder
            .append(Instruction {
                inputs: vec![conn, block, reqvec.index],
                operation: Operation::BuildBIP152BlockTxReqFromMetadata,
            })
            .expect("Inserting BuildBIP152BlockTxReqFromMetadata should always succeed")
            .pop()
            .expect("BuildBIP152BlockTxReqFromMetadata should always produce a var");
        builder
            .append(Instruction {
                inputs: vec![conn, blocktxn.index],
                operation: Operation::SendBlockTxn,
            })
            .expect("Inserting SendBlockTxn should always succeed");

        let second_half = Program::unchecked_new(
            builder.context().clone(),
            program.instructions[insertion_index..]
                .iter()
                .cloned()
                .collect(),
        );
        builder
            .append_program(
                second_half,
                variable_threshould,
                builder.variable_count() - variable_threshould,
            )
            .unwrap();
        *program = builder.finalize().unwrap();
        Ok(())
    }

    fn name(&self) -> &'static str {
        "BlockTxnMutator"
    }
}

impl BlockTxnMutator {
    pub fn new() -> Self {
        Self {}
    }
}

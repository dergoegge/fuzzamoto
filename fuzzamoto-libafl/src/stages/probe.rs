use crate::feedbacks::recv::RuntimeMetadata;
use crate::input::IrInput;
use fuzzamoto_ir::{Instruction, Operation, ProbeOperation};
use libafl::{
    HasMetadata,
    corpus::{Corpus, CorpusId},
    executors::Executor,
    fuzzer::Evaluator,
    stages::{
        Restartable, Stage,
        mutational::{MutatedTransform, MutatedTransformPost},
    },
    state::{HasCorpus, HasCurrentTestcase},
};
use std::collections::HashSet;
pub struct ProbingStage {
    seen: HashSet<CorpusId>,
}

impl ProbingStage {
    pub fn new() -> Self {
        Self {
            seen: HashSet::new(),
        }
    }
}

impl<E, EM, S, Z> Stage<E, EM, S, Z> for ProbingStage
where
    E: Executor<EM, IrInput, S, Z>,
    Z: Evaluator<E, EM, IrInput, S>,
    S: HasMetadata + HasCorpus<IrInput> + HasCurrentTestcase<IrInput>,
{
    fn perform(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        state: &mut S,
        manager: &mut EM,
    ) -> Result<(), libafl::Error> {
        if !state.has_metadata::<RuntimeMetadata>() {
            state.add_metadata(RuntimeMetadata::default());
        }

        let mut testcase = state.current_testcase_mut()?.clone();
        let Ok(mut input) = IrInput::try_transform_from(&mut testcase, state) else {
            return Ok(());
        };
        let cur = state
            .corpus()
            .current()
            .expect("CorpusId should be available during stage execution");

        if self.seen.contains(&cur) {
            return Ok(());
        }

        // adding probing operation to the beginning and to the end of the instructions
        let mut builder = fuzzamoto_ir::ProgramBuilder::new(input.ir().context.clone());

        // usually "inserting" an instruction is not this straightforward.
        // since the inserted instruction will create a variable so the variable offset after the inserted instructions needs to be adjusted.
        // however in this special case of `ProbeOperation`, we don't generate any new variables
        // thus we can simply append instructions sequentially to get the modified program
        builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::Probe(ProbeOperation::EnableRecv),
            })
            .expect("appending EnableProbe should always succeed");
        builder
            .append_all(input.ir().instructions.iter().cloned())
            .expect("Partial append should always succeed if full append succeeded");
        builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::Probe(ProbeOperation::DisableRecv),
            })
            .expect("appending EnableProbe should always succeed");

        let Ok(new_program) = builder.finalize() else {
            return Ok(());
        };
        // now swap it with the new program
        *input.ir_mut() = new_program;
        let (untransformed, post) = input.try_transform_into(state)?;
        // this will automatically put metadata into the feedback
        log::info!("Probing for testcase {:?}", cur);
        let (_, corpus_id) = fuzzer.evaluate_filtered(state, executor, manager, &untransformed)?;
        post.post_exec(state, corpus_id)?;
        log::info!("Done Probing for testcase {:?}", cur);
        self.seen.insert(cur);
        Ok(())
    }
}

impl<S> Restartable<S> for ProbingStage {
    fn should_restart(&mut self, _state: &mut S) -> Result<bool, libafl::Error> {
        Ok(true)
    }

    fn clear_progress(&mut self, _state: &mut S) -> Result<(), libafl::Error> {
        Ok(())
    }
}

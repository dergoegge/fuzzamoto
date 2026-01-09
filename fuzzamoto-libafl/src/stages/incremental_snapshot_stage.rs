use std::marker::PhantomData;
use std::num::NonZeroUsize;

use libafl::{
    Error, HasMetadata,
    corpus::{Corpus, CorpusId, HasCurrentCorpusId},
    stages::{Restartable, Stage},
    state::{HasCorpus, HasCurrentTestcase, HasRand},
};
use libafl_bolts::rands::Rand;
use libafl_nyx::executor::NyxExecutor;
use serde::{Deserialize, Serialize};

use fuzzamoto_ir::Program;

use crate::input::IrInput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalSnapshotMetadata {
    /// Number of iterations using the current temporary snapshot
    pub current_reuse_count: usize,
    /// Number of times to reuse a temporary snapshot before creating a new one
    pub max_reuse_count: usize,
    /// Length of the frozen prefix (instructions 0..frozen_prefix_len are frozen)
    pub frozen_prefix_len: Option<usize>,
    /// Corpus entry the temporary snapshot was created for
    pub corpus_id: Option<CorpusId>,
}

impl Default for IncrementalSnapshotMetadata {
    fn default() -> Self {
        Self {
            current_reuse_count: 0,
            max_reuse_count: 50,
            frozen_prefix_len: None,
            corpus_id: None,
        }
    }
}

libafl_bolts::impl_serdeany!(IncrementalSnapshotMetadata);

#[derive(Debug, Clone, Copy)]
pub enum SnapshotPlacementPolicy {
    Balanced,
}

pub struct IncrementalSnapshotStage<IS, S, OT> {
    inner_stage: IS,
    policy: SnapshotPlacementPolicy,
    max_reuse_count: usize,
    phantom: PhantomData<(S, OT)>,
}

impl<IS, S, OT> IncrementalSnapshotStage<IS, S, OT> {
    pub fn new(inner_stage: IS, policy: SnapshotPlacementPolicy, max_reuse_count: usize) -> Self {
        Self {
            inner_stage,
            policy,
            max_reuse_count,
            phantom: PhantomData,
        }
    }

    /// Choose where to take the snapshot based on the placement policy
    fn choose_position(&self, rand: &mut impl Rand, program_len: usize) -> Option<usize> {
        if program_len == 0 {
            return None;
        }

        match self.policy {
            SnapshotPlacementPolicy::Balanced => {
                if program_len == 1 {
                    Some(0)
                } else {
                    if rand.coinflip(0.5_f64) {
                        // First half
                        let half = (program_len / 2).max(1);
                        let nz_half = NonZeroUsize::new(half).expect("half should be non-zero");
                        Some(rand.below(nz_half))
                    } else {
                        // Second half
                        let half = program_len / 2;
                        let range = program_len - half;
                        let nz_range = NonZeroUsize::new(range).expect("range should be non-zero");
                        Some(half + rand.below(nz_range))
                    }
                }
            }
        }
    }
}

impl<IS, EM, S, Z, OT> Stage<NyxExecutor<S, OT>, EM, S, Z> for IncrementalSnapshotStage<IS, S, OT>
where
    IS: Stage<NyxExecutor<S, OT>, EM, S, Z>,
    S: HasCorpus<IrInput>
        + HasRand
        + HasMetadata
        + HasCurrentTestcase<IrInput>
        + HasCurrentCorpusId,
    OT: libafl::observers::ObserversTuple<IrInput, S>,
{
    fn perform(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut NyxExecutor<S, OT>,
        state: &mut S,
        manager: &mut EM,
    ) -> Result<(), Error> {
        if !state.has_metadata::<IncrementalSnapshotMetadata>() {
            let mut meta = IncrementalSnapshotMetadata::default();
            meta.max_reuse_count = self.max_reuse_count;
            state.add_metadata(meta);
        }

        let qemu_has_tmp = executor.helper.nyx_process.aux_tmp_snapshot_created();

        // Check if we can reuse the tmp snapshot
        let metadata = state.metadata::<IncrementalSnapshotMetadata>()?;
        let has_corpus = metadata.corpus_id.is_some();
        let has_prefix = metadata.frozen_prefix_len.is_some();
        let count_ok = metadata.current_reuse_count < metadata.max_reuse_count;

        if has_corpus && has_prefix && count_ok && qemu_has_tmp {
            // The scheduler may select a different corpus entry than the one our snapshot
            // is based on, so override it.
            let (saved_corpus_id, prefix_len) = {
                let metadata = state.metadata::<IncrementalSnapshotMetadata>()?;
                (
                    metadata.corpus_id.unwrap(),
                    metadata.frozen_prefix_len.unwrap(),
                )
            };

            state.set_corpus_id(saved_corpus_id)?;
            *state.corpus_mut().current_mut() = Some(saved_corpus_id);

            // Reload in case input was evicted to disk
            {
                let mut testcase = state.current_testcase_mut()?;
                let _ = state.corpus().load_input_into(&mut testcase);
            }

            // Update metadata
            let reuse_count = {
                let metadata = state.metadata_mut::<IncrementalSnapshotMetadata>()?;
                metadata.current_reuse_count += 1;
                let should_discard_snapshot =
                    metadata.current_reuse_count >= metadata.max_reuse_count;

                executor
                    .helper
                    .nyx_process
                    .option_set_delete_incremental_snapshot(should_discard_snapshot);
                executor.helper.nyx_process.option_apply();

                metadata.current_reuse_count
            };

            // Set frozen_prefix_len on input
            {
                let mut testcase = state.current_testcase_mut()?;
                let input = testcase.input_mut().as_mut().unwrap();
                input.frozen_prefix_len = Some(prefix_len);
            }

            self.inner_stage.perform(fuzzer, executor, state, manager)?;

            // Clear frozen_prefix_len after execution
            {
                let mut testcase = state.current_testcase_mut()?;

                // Input may have been evicted after perform
                if let Some(input) = testcase.input_mut().as_mut() {
                    input.frozen_prefix_len = None;
                }
            }

            // Clear metadata if we're done using the tmp snapshot
            if reuse_count >= self.max_reuse_count {
                let metadata = state.metadata_mut::<IncrementalSnapshotMetadata>()?;
                metadata.frozen_prefix_len = None;
                metadata.corpus_id = None;
                metadata.current_reuse_count = 0;
            }
        } else {
            // Take a new tmp snapshot, first discarding any existing tmp snapshot
            if qemu_has_tmp {
                executor
                    .helper
                    .nyx_process
                    .option_set_delete_incremental_snapshot(true);
                executor.helper.nyx_process.option_apply();

                // Clear stale metadata
                {
                    let metadata = state.metadata_mut::<IncrementalSnapshotMetadata>()?;
                    metadata.frozen_prefix_len = None;
                    metadata.corpus_id = None;
                    metadata.current_reuse_count = 0;
                }

                // Trigger the discard by running the inner_stage so the RELEASE hypercall is called,
                // then create the new tmp snapshot the next iteration.
                return self.inner_stage.perform(fuzzer, executor, state, manager);
            }

            {
                // Load the input in case it was just evicted to disk
                let mut testcase = state.current_testcase_mut()?;
                let _ = state.corpus().load_input_into(&mut testcase);
            }

            let program_len = {
                let testcase = state.current_testcase()?;
                let input = testcase.input().as_ref().unwrap();
                input.ir().instructions.len()
            };

            if program_len == 0 {
                // Skip creating a tmp snapshot if we're using the empty program
                return self.inner_stage.perform(fuzzer, executor, state, manager);
            }

            if state.rand_mut().coinflip(0.04) {
                // Use the root snapshot some of the time
                return self.inner_stage.perform(fuzzer, executor, state, manager);
            }

            let chosen_pos = self.choose_position(state.rand_mut(), program_len);

            let new_prefix_len = {
                let testcase = state.current_testcase()?;
                let input = testcase.input().as_ref().unwrap();
                chosen_pos.and_then(|pos| find_valid_snapshot_position(input.ir(), pos))
            };

            if let Some(prefix_len) = new_prefix_len {
                executor
                    .helper
                    .nyx_process
                    .option_set_delete_incremental_snapshot(false);
                executor.helper.nyx_process.option_apply();

                // Set frozen_prefix_len on the input so inner_stage is aware of it
                {
                    let mut testcase = state.current_testcase_mut()?;
                    let input = testcase.input_mut().as_mut().unwrap();
                    input.frozen_prefix_len = Some(prefix_len);
                }

                self.inner_stage.perform(fuzzer, executor, state, manager)?;

                let scheduler_corpus_id = state.current_corpus_id()?;

                // Update metadata
                {
                    let metadata = state.metadata_mut::<IncrementalSnapshotMetadata>()?;
                    metadata.frozen_prefix_len = Some(prefix_len);
                    metadata.corpus_id = scheduler_corpus_id;
                    metadata.current_reuse_count = 0;
                }

                // Reset frozen_prefix_len
                {
                    let mut testcase = state.current_testcase_mut()?;
                    let input = testcase.input_mut().as_mut().unwrap();
                    input.frozen_prefix_len = None;
                }

                log::info!(
                    "IncrementalSnapshotStage: Created TMP snapshot at position {}, will reuse for {} iterations",
                    prefix_len,
                    self.max_reuse_count
                );
            } else {
                log::info!(
                    "IncrementalSnapshotStage: No valid position found to create TMP snapshot",
                );

                return Ok(());
            }
        }

        Ok(())
    }
}

impl<IS, S, OT> Restartable<S> for IncrementalSnapshotStage<IS, S, OT>
where
    S: HasMetadata,
    IS: Restartable<S>,
{
    fn should_restart(&mut self, state: &mut S) -> Result<bool, Error> {
        self.inner_stage.should_restart(state)
    }

    fn clear_progress(&mut self, state: &mut S) -> Result<(), Error> {
        self.inner_stage.clear_progress(state)
    }
}

/// Find a valid position for the snapshot that's not inside a block.
fn find_valid_snapshot_position(program: &Program, target_pos: usize) -> Option<usize> {
    let instructions = &program.instructions;

    if instructions.is_empty() {
        return None;
    }

    let target_pos = target_pos.min(instructions.len());

    let mut block_depth: usize = 0;
    let mut valid_positions = Vec::new();

    for (i, instr) in instructions.iter().enumerate() {
        if block_depth == 0 {
            valid_positions.push(i);
        }

        if instr.operation.is_block_begin() {
            block_depth += 1;
        }
        if instr.operation.is_block_end() {
            block_depth = block_depth.saturating_sub(1);
        }
    }

    if block_depth == 0 {
        valid_positions.push(instructions.len());
    }

    if valid_positions.is_empty() {
        return None;
    }

    valid_positions
        .into_iter()
        .min_by_key(|&pos| (pos as isize - target_pos as isize).unsigned_abs())
}

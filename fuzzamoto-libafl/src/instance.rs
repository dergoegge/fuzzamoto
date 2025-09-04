use std::{borrow::Cow, cell::RefCell, marker::PhantomData, process, time::Duration};

use fuzzamoto_ir::{
    AddTxToBlockGenerator, AdvanceTimeGenerator, BlockGenerator, CombineMutator,
    CompactFilterQueryGenerator, GetDataGenerator, HeaderGenerator, InputMutator,
    InventoryGenerator, LargeTxGenerator, LongChainGenerator, OneParentOneChildGenerator,
    OperationMutator, Program, SendBlockGenerator, SendMessageGenerator, SingleTxGenerator,
    TxoGenerator, WitnessGenerator, cutting::CuttingMinimizer, instr_block::InstrBlockMinimizer,
    nopping::NoppingMinimizer,
};

use libafl::{
    Error, NopFuzzer,
    corpus::{CachedOnDiskCorpus, Corpus, CorpusId, OnDiskCorpus, Testcase},
    events::{
        ClientDescription, EventFirer, EventReceiver, EventRestarter, NopEventManager,
        ProgressReporter, SendExiting,
    },
    executors::Executor,
    feedback_and_fast, feedback_or,
    feedbacks::{ConstFeedback, CrashFeedback, HasObserverHandle, MaxMapFeedback, TimeFeedback},
    fuzzer::{Evaluator, Fuzzer, StdFuzzer},
    mutators::{ComposedByMutations, TuneableScheduledMutator},
    observers::{CanTrack, HitcountsMapObserver, StdMapObserver, TimeObserver},
    schedulers::{
        IndexesLenTimeMinimizerScheduler, QueueScheduler, StdWeightedScheduler,
        powersched::PowerSchedule,
    },
    stages::{ClosureStage, IfStage, StagesTuple, TuneableMutationalStage, WhileStage},
    state::{HasCorpus, HasMaxSize, HasRand, StdState},
};
use libafl_bolts::{
    HasLen, current_nanos,
    rands::{Rand, StdRand},
    tuples::tuple_list,
};
use libafl_nyx::{executor::NyxExecutor, helper::NyxHelper, settings::NyxSettings};
use rand::{SeedableRng, rngs::SmallRng};
use typed_builder::TypedBuilder;

use crate::{
    input::IrInput,
    mutators::{IrGenerator, IrMutator, IrSpliceMutator, LibAflByteMutator},
    options::FuzzerOptions,
    schedulers::SupportedSchedulers,
    stages::IrMinimizerStage,
};

#[cfg(feature = "bench")]
use crate::stages::BenchStatsStage;
#[cfg(not(feature = "bench"))]
use libafl::stages::nop::NopStage;

pub type ClientState =
    StdState<CachedOnDiskCorpus<IrInput>, IrInput, StdRand, OnDiskCorpus<IrInput>>;

#[derive(TypedBuilder)]
pub struct Instance<'a, EM> {
    options: &'a FuzzerOptions,
    /// The harness. We create it before forking, then `take()` it inside the client.
    mgr: EM,
    client_description: ClientDescription,
}

impl<EM> Instance<'_, EM>
where
    EM: EventFirer<IrInput, ClientState>
        + EventRestarter<ClientState>
        + ProgressReporter<ClientState>
        + SendExiting
        + EventReceiver<IrInput, ClientState>,
{
    pub fn run(mut self, state: Option<ClientState>) -> Result<(), Error> {
        let parent_cpu_id = self
            .options
            .cores
            .ids
            .first()
            .expect("unable to get first core id");

        let timeout = Duration::from_millis(self.options.timeout as u64);
        let settings = NyxSettings::builder()
            .cpu_id(self.client_description.core_id().0)
            .parent_cpu_id(Some(parent_cpu_id.0))
            .input_buffer_size(self.options.buffer_size)
            .timeout_secs(timeout.as_secs() as u8)
            .timeout_micro_secs(timeout.subsec_micros() as u32)
            .workdir_path(Cow::from(
                self.options.work_dir().to_str().unwrap().to_string(),
            ))
            .build();

        let helper = NyxHelper::new(self.options.shared_dir(), settings)?;

        let trace_observer = HitcountsMapObserver::new(unsafe {
            StdMapObserver::from_mut_ptr("trace", helper.bitmap_buffer, helper.bitmap_size)
        })
        .track_indices()
        .track_novelties();

        // Create an observation channel to keep track of the execution time
        let time_observer = TimeObserver::new("time");

        let map_feedback = MaxMapFeedback::new(&trace_observer);

        let trace_handle = map_feedback.observer_handle().clone();

        #[cfg(feature = "bench")]
        let bench_stats_stage = BenchStatsStage::new(
            trace_handle.clone(),
            Duration::from_secs(30),
            self.options.bench_dir().join(format!(
                "bench-cpu_{:03}.csv",
                self.client_description.core_id().0
            )),
        );
        #[cfg(not(feature = "bench"))]
        let bench_stats_stage = NopStage::new();

        // let stdout_observer = StdOutObserver::new("hprintf_output");

        // Feedback to rate the interestingness of an input
        // This one is composed by two Feedbacks in OR
        let mut feedback = feedback_or!(
            // New maximization map feedback linked to the edges observer and the feedback state
            feedback_and_fast!(
                // Disable coverage feedback if the corpus is static
                ConstFeedback::new(!self.options.static_corpus),
                // Disable coverage feedback if we're minimizing an input
                ConstFeedback::new(!self.options.minimize_input.is_some()),
                map_feedback
            ),
            // Time feedback, this one does not need a feedback state
            TimeFeedback::new(&time_observer),
            // Append stdout to metadata
            // StdOutToMetadataFeedback::new(&stdout_observer)
        );

        // A feedback to choose if an input is a solution or not
        let mut objective = feedback_and_fast!(
            CrashFeedback::new(),
            // Take it only if trigger new coverage over crashes
            // For deduplication
            MaxMapFeedback::with_name("mapfeedback_metadata_objective", &trace_observer)
        );

        // If not restarting, create a State from scratch
        let mut state = match state {
            Some(x) => x,
            None => {
                StdState::new(
                    // RNG
                    StdRand::with_seed(current_nanos()),
                    // Corpus that will be evolved
                    CachedOnDiskCorpus::new(
                        self.options.queue_dir(self.client_description.core_id()),
                        100,
                    )?,
                    // Corpus in which we store solutions
                    OnDiskCorpus::new(self.options.crashes_dir(self.client_description.core_id()))?,
                    &mut feedback,
                    &mut objective,
                )?
            }
        };

        let scheduler = if self.options.minimize_input.is_some() {
            // Avoid scheduler metatdata dependency
            SupportedSchedulers::Queue(QueueScheduler::new(), PhantomData::default())
        } else {
            // A minimization+queue policy to get testcasess from the corpus
            SupportedSchedulers::LenTimeMinimizer(
                IndexesLenTimeMinimizerScheduler::new(
                    &trace_observer,
                    StdWeightedScheduler::with_schedule(
                        &mut state,
                        &trace_observer,
                        Some(PowerSchedule::explore()),
                    ),
                ),
                PhantomData::default(),
            )
        };

        let observers = tuple_list!(trace_observer, time_observer); // stdout_observer);

        state.set_max_size(self.options.buffer_size);

        // A fuzzer with feedbacks and a corpus scheduler
        let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

        if let Some(rerun_input) = &self.options.rerun_input {
            let input = IrInput::unparse(rerun_input);

            let mut executor = NyxExecutor::builder().build(helper, observers);

            let exit_kind = executor
                .run_target(
                    &mut NopFuzzer::new(),
                    &mut state,
                    &mut NopEventManager::new(),
                    &input,
                )
                .expect("Error running target");
            println!("Rerun finished with ExitKind {:?}", exit_kind);
            // We're done :)
            process::exit(0);
        }

        let mut executor = NyxExecutor::builder().build(helper, observers);

        let ir_context_dump = self.options.work_dir().join("dump/ir.context");
        let bytes = std::fs::read(ir_context_dump).expect("Could not read ir context file");
        let full_program_context: fuzzamoto_ir::FullProgramContext =
            postcard::from_bytes(&bytes).expect("could not deser ir context");

        if self
            .options
            .input_dir()
            .read_dir()
            .unwrap()
            .next()
            .is_none()
        {
            let initial_input = IrInput::new(Program::unchecked_new(
                full_program_context.context.clone(),
                vec![],
            ));
            let bytes = postcard::to_allocvec(&initial_input).unwrap();

            let file_path = self.options.input_dir().join("initial_input");
            std::fs::write(&file_path, bytes).unwrap();
        }

        let rng = SmallRng::seed_from_u64(state.rand_mut().next());

        let mutator = TuneableScheduledMutator::new(
            &mut state,
            tuple_list!(
                IrMutator::new(InputMutator::new(), rng.clone()),
                IrMutator::new(OperationMutator::new(LibAflByteMutator::new()), rng.clone()),
                //IrSpliceMutator::new(ConcatMutator::new(), rng.clone()),
                IrSpliceMutator::new(CombineMutator::new(), rng.clone()),
                IrGenerator::new(AdvanceTimeGenerator::default(), rng.clone()),
                IrGenerator::new(SendMessageGenerator::default(), rng.clone()),
                IrGenerator::new(SingleTxGenerator::default(), rng.clone()),
                IrGenerator::new(LongChainGenerator::default(), rng.clone()),
                IrGenerator::new(LargeTxGenerator::default(), rng.clone()),
                IrGenerator::new(OneParentOneChildGenerator::default(), rng.clone()),
                IrGenerator::new(
                    TxoGenerator::new(full_program_context.txos.clone()),
                    rng.clone()
                ),
                IrGenerator::new(WitnessGenerator::new(), rng.clone()),
                IrGenerator::new(InventoryGenerator::default(), rng.clone()),
                IrGenerator::new(GetDataGenerator::default(), rng.clone()),
                IrGenerator::new(BlockGenerator::default(), rng.clone()),
                IrGenerator::new(
                    HeaderGenerator::new(full_program_context.headers.clone()),
                    rng.clone()
                ),
                IrGenerator::new(SendBlockGenerator::default(), rng.clone()),
                IrGenerator::new(AddTxToBlockGenerator::default(), rng.clone()),
                IrGenerator::new(CompactFilterQueryGenerator::default(), rng.clone()),
            ),
        );

        let weights = &[
            2000f32, 1000.0, 100.0, 10.0, 40.0, 50.0, 50.0, 50.0, 50.0, 20.0, 20.0, 20.0, 20.0,
            50.0, 50.0, 50.0, 50.0, 10.0,
        ];
        let sum = weights.iter().sum::<f32>();
        assert_eq!(mutator.mutations().len(), weights.len());

        mutator
            .set_mutation_probabilities(&mut state, weights.iter().map(|w| *w / sum).collect())
            .unwrap();

        mutator
            .set_iter_probabilities_pow(&mut state, vec![0.025f32, 0.1, 0.4, 0.3, 0.1, 0.05, 0.025])
            .unwrap();

        let minimizing_crash = self.options.minimize_input.is_some();

        // Counter holding the number of successful minimizations in the last round
        let continue_minimizing = RefCell::new(1u64);

        let mut stages = tuple_list!(
            ClosureStage::new(|_a: &mut _, _b: &mut _, _c: &mut _, _d: &mut _| {
                // Always try minimizing at least for one pass
                *continue_minimizing.borrow_mut() = 1;
                Ok(())
            }),
            WhileStage::new(
                |_, _, _, _| Ok((!self.options.static_corpus || minimizing_crash)
                    && *continue_minimizing.borrow() > 0),
                tuple_list!(
                    ClosureStage::new(|_a: &mut _, _b: &mut _, _c: &mut _, _d: &mut _| {
                        // Reset the minimization counter
                        *continue_minimizing.borrow_mut() = 0;
                        Ok(())
                    }),
                    IrMinimizerStage::<CuttingMinimizer, _, _>::new(
                        trace_handle.clone(),
                        200,
                        minimizing_crash,
                        &continue_minimizing
                    ),
                    IrMinimizerStage::<InstrBlockMinimizer, _, _>::new(
                        trace_handle.clone(),
                        200,
                        minimizing_crash,
                        &continue_minimizing
                    ),
                    IrMinimizerStage::<NoppingMinimizer, _, _>::new(
                        trace_handle.clone(),
                        200,
                        minimizing_crash,
                        &continue_minimizing
                    ),
                )
            ),
            IfStage::new(
                |_, _, _, _| Ok(!self.options.minimize_input.is_some()),
                tuple_list!(TuneableMutationalStage::new(&mut state, mutator))
            ),
            bench_stats_stage,
        );
        self.fuzz(&mut state, &mut fuzzer, &mut executor, &mut stages)
    }

    fn fuzz<Z, E, ST>(
        &mut self,
        state: &mut ClientState,
        fuzzer: &mut Z,
        executor: &mut E,
        stages: &mut ST,
    ) -> Result<(), Error>
    where
        Z: Fuzzer<E, EM, IrInput, ClientState, ST> + Evaluator<E, EM, IrInput, ClientState>,
        ST: StagesTuple<E, EM, ClientState, Z>,
    {
        let corpus_dirs = [self.options.input_dir()];

        if state.must_load_initial_inputs() {
            if let Some(minimize_input) = &self.options.minimize_input {
                let input = IrInput::unparse(minimize_input);
                state.corpus_mut().add(Testcase::from(input)).unwrap();
            } else if self.options.static_corpus {
                state
                    .load_initial_inputs_forced(fuzzer, executor, &mut self.mgr, &corpus_dirs)
                    .unwrap_or_else(|_| {
                        println!("Failed to load initial corpus at {corpus_dirs:?}");
                        process::exit(0);
                    });
            } else {
                state
                    .load_initial_inputs(fuzzer, executor, &mut self.mgr, &corpus_dirs)
                    .unwrap_or_else(|_| {
                        println!("Failed to load initial corpus at {corpus_dirs:?}");
                        process::exit(0);
                    });
            }

            if self.options.prune_disabled {
                let corpus = state.corpus_mut();
                let disabled_corpus_ids: Vec<CorpusId> = (0..corpus.count_disabled())
                    .map(|idx| corpus.nth_from_all(idx + corpus.count()))
                    .collect();
                for id in disabled_corpus_ids.iter() {
                    let _ = corpus.remove(*id);
                }
                println!(
                    "Pruned {} disabled inputs from corpus",
                    disabled_corpus_ids.len()
                );
            }

            println!("We imported {} inputs from disk", state.corpus().count());
        }

        if self.options.minimize_input.is_some() {
            fuzzer.fuzz_one(stages, executor, state, &mut self.mgr)?;
        } else if let Some(iters) = self.options.iterations {
            fuzzer.fuzz_loop_for(stages, executor, state, &mut self.mgr, iters)?;

            // It's important, that we store the state before restarting!
            // Else, the parent will not respawn a new child and quit.
            self.mgr.on_restart(state)?;
        } else {
            fuzzer.fuzz_loop(stages, executor, state, &mut self.mgr)?;
        }

        Ok(())
    }
}

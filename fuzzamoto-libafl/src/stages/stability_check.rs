//! The stability check stage for fuzzamoto.

use core::marker::PhantomData;
use libafl::{
    Error, HasMetadata, HasNamedMetadata,
    corpus::{Corpus, CorpusId},
    events::{Event, EventFirer, EventWithStats},
    executors::{Executor, ExitKind, HasObservers},
    feedbacks::MapFeedbackMetadata,
    monitors::stats::{AggregatorOps, UserStats, UserStatsValue},
    observers::{MapObserver, ObserversTuple},
    stages::{Restartable, Stage, mutational::MutatedTransform},
    state::{HasCorpus, HasCurrentTestcase, HasExecutions},
};

use libafl_bolts::{impl_serdeany, tuples::Handle};
use num_traits::Bounded;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::{borrow::Cow, collections::HashSet, fmt::Debug};

use crate::input::IrInput;
/// AFL++'s `CAL_CYCLES` + 1
const CAL_STAGE_MAX: usize = 8;

/// The metadata to keep unstable entries
/// Formula is same as AFL++: number of unstable entries divided by the number of filled entries.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UnstableEntriesMetadata {
    unstable_entries: HashSet<usize>,
    filled_entries_count: usize,
    unstable_tc_path: Vec<String>,
}
impl_serdeany!(UnstableEntriesMetadata);

impl UnstableEntriesMetadata {
    #[must_use]
    /// Create a new [`struct@UnstableEntriesMetadata`]
    pub fn new() -> Self {
        Self {
            unstable_entries: HashSet::new(),
            filled_entries_count: 0,
            unstable_tc_path: Vec::new(),
        }
    }
}

impl Default for UnstableEntriesMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs the target with pre and post execution hooks and returns the exit kind and duration.
pub fn run_target_once<E, EM, Z, S, OT>(
    fuzzer: &mut Z,
    executor: &mut E,
    state: &mut S,
    mgr: &mut EM,
    input: &IrInput,
    had_errors: bool,
) -> Result<(ExitKind, bool), Error>
where
    E: HasObservers<Observers = OT> + Executor<EM, IrInput, S, Z>,
    OT: ObserversTuple<IrInput, S>,
{
    executor.observers_mut().pre_exec_all(state, input)?;

    let exit_kind = executor.run_target(fuzzer, state, mgr, input)?;
    let mut has_errors = had_errors;
    if exit_kind != ExitKind::Ok && !had_errors {
        log::info!("Corpus entry errored on execution!");

        has_errors = true;
    }
    executor
        .observers_mut()
        .post_exec_all(state, input, &exit_kind)?;

    Ok((exit_kind, has_errors))
}

/// The stability check stage will measure the stability and disable flaky entry in the coverage map.
#[derive(Debug, Clone)]
pub struct StabilityCheckStage<C, O, OT, S> {
    map_observer_handle: Handle<C>,
    map_name: Cow<'static, str>,
    stage_max: usize,
    unstable_stats: std::path::PathBuf,
    unstable_testcases: Vec<String>,
    seen: HashSet<CorpusId>,
    phantom: PhantomData<(O, OT, S)>,
}

impl<C, O, OT, S> StabilityCheckStage<C, O, OT, S> {
    pub fn new(
        observer_handle: &Handle<C>,
        map_name: &str,
        stage_max: usize,
        unstable_stats: &std::path::PathBuf,
    ) -> Self {
        Self {
            map_observer_handle: observer_handle.clone(),
            map_name: Cow::Owned(map_name.to_owned()),
            stage_max,
            unstable_stats: unstable_stats.clone(),
            unstable_testcases: Vec::new(),
            seen: HashSet::new(),
            phantom: PhantomData,
        }
    }

    pub fn write_unstable_stats(&self) -> Result<(), Error> {
        let mut tmp = std::path::PathBuf::from(self.unstable_stats.clone());
        tmp.set_extension(".tmp");

        let file = std::fs::File::create(&tmp)?;
        let mut writer = std::io::BufWriter::new(file);

        for line in &self.unstable_testcases {
            writeln!(writer, "{}", line)?;
        }

        writer.flush()?;
        drop(writer);

        std::fs::rename(tmp, self.unstable_stats.clone())?;

        Ok(())
    }
}

impl<C, E, EM, O, OT, S, Z> Stage<E, EM, S, Z> for StabilityCheckStage<C, O, OT, S>
where
    S: HasCorpus<IrInput>
        + HasCurrentTestcase<IrInput>
        + HasNamedMetadata
        + HasMetadata
        + HasExecutions,
    E: HasObservers<Observers = OT> + Executor<EM, IrInput, S, Z>,
    EM: EventFirer<IrInput, S>,
    O: MapObserver,
    for<'de> <O as MapObserver>::Entry:
        Serialize + Deserialize<'de> + 'static + Default + Debug + Bounded,
    OT: ObserversTuple<IrInput, S>,
    C: AsRef<O>,
{
    #[inline]
    #[expect(clippy::too_many_lines, clippy::cast_precision_loss)]
    fn perform(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        state: &mut S,
        mgr: &mut EM,
    ) -> Result<(), Error> {
        let cur = state
            .corpus()
            .current()
            .expect("CorpusId should be available during stage execution");

        if self.seen.contains(&cur) {
            return Ok(());
        }
        self.seen.insert(cur);

        let mut iter = self.stage_max;

        // If we restarted after a timeout or crash, do less iterations.
        let mut testcase = state.current_testcase_mut()?.clone();
        let Ok(input) = IrInput::try_transform_from(&mut testcase, state) else {
            return Ok(());
        };
        let (_, _) = run_target_once(fuzzer, executor, state, mgr, &input, false)?;

        let observers = &executor.observers();
        let map_first = observers[&self.map_observer_handle].as_ref();
        let filled_count = match state
            .named_metadata_map()
            .get::<MapFeedbackMetadata<O::Entry>>(&self.map_name)
        {
            Some(metadata) => metadata.num_covered_map_indexes,
            None => map_first.count_bytes().try_into().map_err(|len| {
                Error::illegal_state(
                    format!(
                        "map's filled entry count ({}) is greater than usize::MAX ({})",
                        len,
                        usize::MAX,
                    )
                    .as_str(),
                )
            })?,
        };
        let map_first_entries = map_first.to_vec();
        let map_first_len = map_first.to_vec().len();
        let mut unstable_entries: Vec<usize> = vec![];

        let mut i = 1;
        let mut has_errors = false;

        while i < iter {
            let (exit_kind, has_errors_result) =
                run_target_once(fuzzer, executor, state, mgr, &input, has_errors)?;
            has_errors = has_errors_result;

            if exit_kind != ExitKind::Timeout {
                let map = &executor.observers()[&self.map_observer_handle]
                    .as_ref()
                    .to_vec();

                let map_state = state
                    .named_metadata_map_mut()
                    .get_mut::<MapFeedbackMetadata<O::Entry>>(&self.map_name)
                    .unwrap();
                let history_map = &mut map_state.history_map;

                if history_map.len() < map_first_len {
                    history_map.resize(map_first_len, O::Entry::default());
                }

                for (idx, (first, (cur, history))) in map_first_entries
                    .iter()
                    .zip(map.iter().zip(history_map.iter_mut()))
                    .enumerate()
                {
                    if *first != *cur && *history != O::Entry::max_value() {
                        // If we just hit a history map entry that was not covered before, but is now flagged as flaky,
                        // we need to make sure the `num_covered_map_indexes` is kept in sync.
                        map_state.num_covered_map_indexes +=
                            usize::from(*history == O::Entry::default());
                        *history = O::Entry::max_value();
                        unstable_entries.push(idx);
                    }
                }

                if !unstable_entries.is_empty() && iter < CAL_STAGE_MAX {
                    iter += 2;
                }
            }
            i += 1;
        }

        let mut send_default_stability = false;
        let unstable_found = !unstable_entries.is_empty();
        let path = state.current_testcase()?.file_path().to_owned();
        if unstable_found {
            let metadata = state.metadata_or_insert_with(UnstableEntriesMetadata::new);
            if let Some(p) = path {
                self.unstable_testcases
                    .push(p.file_name().unwrap().to_string_lossy().to_string());
            }
            // If we see new unstable entries executing this new corpus entries, then merge with the existing one
            for item in unstable_entries {
                metadata.unstable_entries.insert(item); // Insert newly found items
            }
            metadata.filled_entries_count = filled_count;
        } else if !state.has_metadata::<UnstableEntriesMetadata>() && filled_count > 0 {
            send_default_stability = true;
            state.add_metadata(UnstableEntriesMetadata::new());
        }

        // Send the stability event to the broker
        if unstable_found {
            if let Some(meta) = state.metadata_map().get::<UnstableEntriesMetadata>() {
                let unstable_entries = meta.unstable_entries.len();
                assert_ne!(filled_count, 0, "The map's filled count must never be 0");
                // In theory `map_first_filled_count - unstable_entries` could be negative.
                // Because `map_first_filled_count` is the filled count of just one single run.
                // While the `unstable_entries` is the number of all the unstable entries across multiple runs.
                // If the target is very unstable (~100%) then this would hit more edges than `map_first_filled_count`.
                // But even in that case, we don't allow negative stability and just show 0% here.
                let stable_count: u64 = filled_count.saturating_sub(unstable_entries) as u64;
                mgr.fire(
                    state,
                    EventWithStats::with_current_time(
                        Event::UpdateUserStats {
                            name: Cow::from("stability"),
                            value: UserStats::new(
                                UserStatsValue::Ratio(stable_count, filled_count as u64),
                                AggregatorOps::Avg,
                            ),
                            phantom: PhantomData,
                        },
                        *state.executions(),
                    ),
                )?;
            }
        } else if send_default_stability {
            mgr.fire(
                state,
                EventWithStats::with_current_time(
                    Event::UpdateUserStats {
                        name: Cow::from("stability"),
                        value: UserStats::new(UserStatsValue::Ratio(1, 1), AggregatorOps::Avg),
                        phantom: PhantomData,
                    },
                    *state.executions(),
                ),
            )?;
        }

        if unstable_found {
            self.write_unstable_stats()?;
        }

        Ok(())
    }
}

impl<C, O, OT, S> Restartable<S> for StabilityCheckStage<C, O, OT, S> {
    fn should_restart(&mut self, _state: &mut S) -> Result<bool, Error> {
        Ok(true)
    }

    fn clear_progress(&mut self, _state: &mut S) -> Result<(), Error> {
        Ok(())
    }
}

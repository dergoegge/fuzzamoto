use std::{
    fs::{File, OpenOptions},
    io::Write,
    marker::PhantomData,
    path::PathBuf,
    time::{Duration, Instant},
};

use libafl::mutators::scheduled::LogMutationMetadata;
use libafl::{
    Evaluator, ExecutesInput, HasMetadata,
    corpus::Corpus,
    events::EventFirer,
    executors::{Executor, HasObservers},
    observers::{CanTrack, MapObserver, ObserversTuple},
    stages::{Restartable, Stage},
    state::{HasCorpus, HasCurrentTestcase, HasExecutions, HasSolutions},
};
use libafl_bolts::tuples::Handle;
use serde::Serialize;

use crate::input::IrInput;

/// Stage for collecting fuzzer stats useful for benchmarking
pub struct BenchStatsStage<T, O> {
    cpu_id: u32,
    trace_handle: Handle<T>,

    initialised: Instant,
    last_update: Instant,
    update_interval: Duration,

    last_execs: u64,
    last_corpus_count: usize,
    last_solution_count: usize,

    // Cumulative union coverage map to report coverage% over time
    union_map: Vec<u8>,

    stats_file_path: PathBuf,
    coverage_file_path: PathBuf,
    mutations_file_path: PathBuf,
    csv_header_written: bool,

    _phantom: PhantomData<O>,
}

impl<T, O> BenchStatsStage<T, O> {
    pub fn new(
        cpu_id: u32,
        trace_handle: Handle<T>,
        update_interval: Duration,
        stats_file_path: PathBuf,
    ) -> Self {
        let coverage_file_path = stats_file_path.with_extension("bin");
        let mutations_file_path = stats_file_path.with_extension("mutations.jsonl");
        let last_update = Instant::now() - 2 * update_interval;
        Self {
            cpu_id,
            trace_handle,
            initialised: Instant::now(),
            last_update,
            update_interval,
            last_execs: 0,
            last_corpus_count: 0,
            last_solution_count: 0,
            union_map: Vec::new(),
            stats_file_path,
            coverage_file_path,
            mutations_file_path,
            csv_header_written: false,
            _phantom: PhantomData::default(),
        }
    }
}

impl<T, O, S> Restartable<S> for BenchStatsStage<T, O> {
    fn should_restart(&mut self, _state: &mut S) -> Result<bool, libafl::Error> {
        Ok(true)
    }

    fn clear_progress(&mut self, _state: &mut S) -> Result<(), libafl::Error> {
        Ok(())
    }
}

impl<E, EM, S, Z, OT, T, O> Stage<E, EM, S, Z> for BenchStatsStage<T, O>
where
    S: HasCorpus<IrInput>
        + HasCurrentTestcase<IrInput>
        + HasMetadata
        + HasExecutions
        + HasSolutions<IrInput>,
    E: Executor<EM, IrInput, S, Z> + HasObservers<Observers = OT>,
    EM: EventFirer<IrInput, S>,
    Z: Evaluator<E, EM, IrInput, S> + ExecutesInput<E, EM, IrInput, S>,
    OT: ObserversTuple<IrInput, S>,
    O: MapObserver<Entry = u8>,
    T: CanTrack + AsRef<O>,
{
    fn perform(
        &mut self,
        _fuzzer: &mut Z,
        executor: &mut E,
        state: &mut S,
        _manager: &mut EM,
    ) -> Result<(), libafl::Error> {
        let now = Instant::now();
        if now < self.last_update + self.update_interval {
            return Ok(());
        }
        let since_last = now - self.last_update;
        self.last_update = now;

        let observers = executor.observers();
        let map_observer = observers[&self.trace_handle].as_ref();
        let initial_entry_value = map_observer.initial();

        // Ensure union map is allocated and merged with current snapshot.
        if self.union_map.len() != map_observer.len() {
            self.union_map = vec![0u8; map_observer.len()];
        }

        for (idx, byte) in self.union_map.iter_mut().enumerate() {
            let val = map_observer.get(idx);
            if val > *byte {
                *byte = val;
            }
        }

        let covered = self
            .union_map
            .iter()
            .filter(|b| **b != initial_entry_value)
            .count();
        let coverage_pct = if map_observer.len() == 0 {
            0.0
        } else {
            (covered as f64 / map_observer.len() as f64) * 100.0
        };

        let elapsed = now.duration_since(self.initialised).as_secs_f64();
        let delta_secs = since_last.as_secs_f64();

        let total_execs = *state.executions();
        let execs_per_sec = if delta_secs > 0.0 {
            (total_execs.saturating_sub(self.last_execs) as f64) / delta_secs
        } else {
            0.0
        };
        self.last_execs = total_execs;

        let corpus_size = state.corpus().count();
        let crashes = state.solutions().count();

        let _ = std::fs::create_dir_all(self.stats_file_path.parent().unwrap());
        let stats_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.stats_file_path)
            .map_err(|e| libafl::Error::unknown(format!("Failed to open stats file: {}", e)))?;

        if !self.csv_header_written {
            writeln!(
                &stats_file,
                "elapsed_s,execs,execs_per_sec,coverage_pct,corpus_size,crashes"
            )
            .map_err(|e| libafl::Error::unknown(format!("Failed to write CSV header: {}", e)))?;
            self.csv_header_written = true;
        }

        log::debug!(
            "bench_stats: cpu={} elapsed={:.3}s execs={} cov={:.4}% corpus={}",
            self.cpu_id,
            elapsed,
            total_execs,
            coverage_pct,
            corpus_size
        );

        writeln!(
            &stats_file,
            "{:.3},{},{:.2},{:.4},{},{}",
            elapsed, total_execs, execs_per_sec, coverage_pct, corpus_size, crashes
        )
        .map_err(|e| libafl::Error::unknown(format!("Failed to write CSV data: {}", e)))?;

        log::debug!(
            "bench_stats: cpu={} dumping coverage map len={}",
            self.cpu_id,
            self.union_map.len()
        );

        dump_coverage_map(&self.union_map, &self.coverage_file_path)?;
        self.record_new_mutations(state)?;

        Ok(())
    }
}

fn dump_coverage_map(data: &[u8], path: &PathBuf) -> Result<(), libafl::Error> {
    let mut file = File::create(path)
        .map_err(|e| libafl::Error::unknown(format!("Failed to open coverage file: {e}")))?;
    file.write_all(data)
        .map_err(|e| libafl::Error::unknown(format!("Failed to write coverage file: {e}")))?;
    Ok(())
}

#[derive(Serialize)]
struct MutationRecord<'a> {
    cpu: u32,
    kind: &'static str,
    corpus_id: usize,
    len: usize,
    chain: &'a [std::borrow::Cow<'static, str>],
}

impl<T, O> BenchStatsStage<T, O> {
    fn record_new_mutations<S>(&mut self, state: &mut S) -> Result<(), libafl::Error>
    where
        S: HasCorpus<IrInput> + HasSolutions<IrInput> + HasMetadata,
    {
        let mut corpus_count = self.last_corpus_count;
        let mut solution_count = self.last_solution_count;

        self.record_for_corpus(state.corpus(), &mut corpus_count, "corpus")?;
        self.record_for_corpus(state.solutions(), &mut solution_count, "solution")?;

        self.last_corpus_count = corpus_count;
        self.last_solution_count = solution_count;
        Ok(())
    }

    fn record_for_corpus<C>(
        &mut self,
        corpus: &C,
        last_count: &mut usize,
        kind: &'static str,
    ) -> Result<(), libafl::Error>
    where
        C: Corpus<IrInput>,
    {
        let count = corpus.count();
        if count <= *last_count {
            // Handle the case where entries were removed
            if count < *last_count {
                *last_count = count;
            }
            return Ok(());
        }

        let _ = std::fs::create_dir_all(
            self.mutations_file_path
                .parent()
                .unwrap_or_else(|| self.stats_file_path.parent().unwrap()),
        );
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.mutations_file_path)
            .map_err(|e| libafl::Error::unknown(format!("Failed to open mutations file: {}", e)))?;

        for idx in *last_count..count {
            let id = corpus.nth(idx);
            let testcase = corpus.get(id)?.borrow();
            if let Some(meta) = testcase.metadata_map().get::<LogMutationMetadata>() {
                // LibAFL's LoggerScheduledMutator records chains in reverse (last mutator first),
                // so flip here to store them in execution order for readability.
                let chain: Vec<_> = meta.iter().rev().cloned().collect();
                let record = MutationRecord {
                    cpu: self.cpu_id,
                    kind,
                    corpus_id: usize::from(id),
                    len: chain.len(),
                    chain: &chain,
                };
                serde_json::to_writer(&mut file, &record).map_err(|e| {
                    libafl::Error::unknown(format!("Failed to serialize mutation record: {e}"))
                })?;
                writeln!(&mut file).map_err(|e| {
                    libafl::Error::unknown(format!("Failed to write mutation record: {e}"))
                })?;
            }
        }

        *last_count = count;
        Ok(())
    }
}

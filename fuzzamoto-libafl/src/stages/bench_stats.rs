use std::{
    fs::{File, OpenOptions},
    io::Write,
    marker::PhantomData,
    path::PathBuf,
    time::{Duration, Instant},
};

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

use crate::input::IrInput;

/// Stage for collecting fuzzer stats useful for benchmarking
pub struct BenchStatsStage<T, O> {
    trace_handle: Handle<T>,

    initialised: Instant,
    last_update: Instant,
    update_interval: Duration,

    last_execs: u64,

    stats_file_path: PathBuf,
    coverage_file_path: PathBuf,
    csv_header_written: bool,

    _phantom: PhantomData<O>,
}

impl<T, O> BenchStatsStage<T, O> {
    pub fn new(
        trace_handle: Handle<T>,
        update_interval: Duration,
        stats_file_path: PathBuf,
    ) -> Self {
        let coverage_file_path = stats_file_path.with_extension("bin");
        let last_update = Instant::now() - 2 * update_interval;
        Self {
            trace_handle,
            initialised: Instant::now(),
            last_update,
            update_interval,
            last_execs: 0,
            stats_file_path,
            coverage_file_path,
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

        let covered = (0..map_observer.len())
            .filter(|idx| map_observer.get(*idx) != initial_entry_value)
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

        writeln!(
            &stats_file,
            "{:.3},{},{:.2},{:.4},{},{}",
            elapsed, total_execs, execs_per_sec, coverage_pct, corpus_size, crashes
        )
        .map_err(|e| libafl::Error::unknown(format!("Failed to write CSV data: {}", e)))?;

        dump_coverage_map(map_observer, &self.coverage_file_path)?;

        Ok(())
    }
}

fn dump_coverage_map<O: MapObserver<Entry = u8>>(
    map_observer: &O,
    path: &PathBuf,
) -> Result<(), libafl::Error> {
    let mut file = File::create(path)
        .map_err(|e| libafl::Error::unknown(format!("Failed to open coverage file: {e}")))?;
    let data = map_observer.to_vec();
    file.write_all(&data)
        .map_err(|e| libafl::Error::unknown(format!("Failed to write coverage file: {e}")))?;
    Ok(())
}

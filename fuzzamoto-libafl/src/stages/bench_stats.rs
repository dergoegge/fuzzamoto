use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

use libafl::{
    Evaluator, ExecutesInput, HasNamedMetadata,
    corpus::Corpus,
    events::EventFirer,
    executors::{Executor, HasObservers},
    feedbacks::MapFeedbackMetadata,
    observers::ObserversTuple,
    stages::{Restartable, Stage},
    state::{HasCorpus, HasExecutions, HasSolutions},
};
use crate::input::IrInput;

/// Stage for collecting fuzzer stats useful for benchmarking.
///
/// Note: `feedback_name` must match the name used to register `MapFeedbackMetadata`
/// (i.e., the feedback's name), which may differ from the observer's name.
pub struct BenchStatsStage {
    cpu_id: u32,
    feedback_name: String,
    map_size: usize,

    initialised: Instant,
    last_update: Instant,
    update_interval: Duration,

    last_execs: u64,

    stats_file_path: PathBuf,
    csv_header_written: bool,
}

impl BenchStatsStage {
    pub fn new(
        cpu_id: u32,
        feedback_name: impl Into<String>,
        map_size: usize,
        update_interval: Duration,
        stats_file_path: PathBuf,
    ) -> Self {
        let last_update = Instant::now() - 2 * update_interval;
        Self {
            cpu_id,
            feedback_name: feedback_name.into(),
            map_size,
            initialised: Instant::now(),
            last_update,
            update_interval,
            last_execs: 0,
            stats_file_path,
            csv_header_written: false,
        }
    }
}

impl<S> Restartable<S> for BenchStatsStage {
    fn should_restart(&mut self, _state: &mut S) -> Result<bool, libafl::Error> {
        Ok(true)
    }

    fn clear_progress(&mut self, _state: &mut S) -> Result<(), libafl::Error> {
        Ok(())
    }
}

impl<E, EM, S, Z, OT> Stage<E, EM, S, Z> for BenchStatsStage
where
    S: HasCorpus<IrInput> + HasExecutions + HasSolutions<IrInput> + HasNamedMetadata,
    E: Executor<EM, IrInput, S, Z> + HasObservers<Observers = OT>,
    EM: EventFirer<IrInput, S>,
    Z: Evaluator<E, EM, IrInput, S> + ExecutesInput<E, EM, IrInput, S>,
    OT: ObserversTuple<IrInput, S>,
{
    fn perform(
        &mut self,
        _fuzzer: &mut Z,
        _executor: &mut E,
        state: &mut S,
        _manager: &mut EM,
    ) -> Result<(), libafl::Error> {
        let now = Instant::now();
        if now < self.last_update + self.update_interval {
            return Ok(());
        }
        let since_last = now - self.last_update;
        self.last_update = now;

        // Get cumulative coverage from MapFeedback's metadata
        let covered = state
            .named_metadata_map()
            .get::<MapFeedbackMetadata<u8>>(&self.feedback_name)
            .map_or(0, |meta| meta.num_covered_map_indexes);

        let coverage_pct = if self.map_size == 0 {
            0.0
        } else {
            (covered as f64 / self.map_size as f64) * 100.0
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

        let Some(parent) = self.stats_file_path.parent() else {
            log::warn!(
                "bench_stats: cpu={} missing parent dir, skipping write",
                self.cpu_id
            );
            return Ok(());
        };
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!(
                "bench_stats: cpu={} failed to create bench dir {}: {e}",
                self.cpu_id,
                parent.display()
            );
            return Ok(());
        }
        let Ok(stats_file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.stats_file_path)
        else {
            log::warn!(
                "bench_stats: cpu={} failed to open stats file {}, skipping write",
                self.cpu_id,
                self.stats_file_path.display()
            );
            return Ok(());
        };

        if !self.csv_header_written {
            if writeln!(
                &stats_file,
                "elapsed_s,execs,execs_per_sec,coverage_pct,corpus_size,crashes"
            )
            .is_err()
            {
                log::warn!(
                    "bench_stats: cpu={} failed to write CSV header to {}",
                    self.cpu_id,
                    self.stats_file_path.display()
                );
                return Ok(());
            }
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

        if writeln!(
            &stats_file,
            "{:.3},{},{:.2},{:.4},{},{}",
            elapsed, total_execs, execs_per_sec, coverage_pct, corpus_size, crashes
        )
        .is_err()
        {
            log::warn!(
                "bench_stats: cpu={} failed to write CSV data to {}",
                self.cpu_id,
                self.stats_file_path.display()
            );
        }

        Ok(())
    }
}

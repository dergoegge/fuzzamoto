use std::{
    collections::BTreeMap,
    fmt::Write,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use nix::sys::signal::{Signal, kill, killpg};
#[cfg(unix)]
use nix::unistd::{Pid, getpgid};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use clap::Subcommand;
use serde::{Deserialize, Serialize};

use crate::error::{CliError, Result};

const DEFAULT_FUZZER_PATH: &str = "target/release/fuzzamoto-libafl";

pub struct BenchmarkCommand;

impl BenchmarkCommand {
    pub fn execute(cmd: &BenchmarkCommands) -> Result<()> {
        match cmd {
            BenchmarkCommands::Run { suite, output } => run_suite(suite, output),
            BenchmarkCommands::Compare {
                baseline,
                candidate,
                output,
                suite,
            } => compare_runs(
                baseline,
                candidate,
                output.as_ref().map(PathBuf::as_path),
                *suite,
            ),
        }
    }
}

#[derive(Subcommand)]
pub enum BenchmarkCommands {
    /// Run a benchmark suite sequentially
    Run {
        #[arg(long, help = "Path to the benchmark suite YAML")]
        suite: PathBuf,
        #[arg(long, help = "Output directory for run artifacts")]
        output: PathBuf,
    },
    /// Compare two benchmark run directories and report deltas
    Compare {
        #[arg(
            long,
            help = "Baseline directory (run: contains summary.json; suite: contains suite_summary.json)"
        )]
        baseline: PathBuf,
        #[arg(
            long,
            help = "Candidate directory (run: contains summary.json; suite: contains suite_summary.json)"
        )]
        candidate: PathBuf,
        #[arg(long, help = "Optional path to write a comparison report (Markdown)")]
        output: Option<PathBuf>,
        #[arg(
            long,
            default_value_t = false,
            help = "Treat baseline/candidate as suite roots (compare mean curves across run_*)"
        )]
        suite: bool,
    },
}

#[derive(Debug, Deserialize)]
struct BenchmarkConfig {
    duration: u64,
    runs: usize,
    cores: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    share_dir: PathBuf,
    corpus_seed: PathBuf,
    #[serde(default)]
    fuzzer_path: Option<PathBuf>,
    #[serde(default = "default_bench_snapshot_secs")]
    bench_snapshot_secs: u64,
}

fn default_timeout_ms() -> u64 {
    1_000
}

fn default_bench_snapshot_secs() -> u64 {
    30
}

fn run_suite(suite: &PathBuf, output: &PathBuf) -> Result<()> {
    let mut file = File::open(suite)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let config: BenchmarkConfig = serde_yaml::from_slice(&buf)?;

    if config.runs == 0 {
        return Err(CliError::InvalidInput(
            "runs must be greater than zero".to_string(),
        ));
    }

    if config.duration == 0 {
        return Err(CliError::InvalidInput(
            "duration must be greater than zero".to_string(),
        ));
    }

    fs::create_dir_all(output)?;

    for run_idx in 0..config.runs {
        log::info!("Starting benchmark run {}/{}", run_idx + 1, config.runs);
        run_single(&config, run_idx, output, suite)?;
    }

    aggregate_suite(output)?;
    Ok(())
}

fn run_single(
    config: &BenchmarkConfig,
    run_idx: usize,
    root: &Path,
    suite_path: &Path,
) -> Result<()> {
    let run_dir = root.join(format!("run_{run_idx:02}"));
    if run_dir.exists() {
        fs::remove_dir_all(&run_dir)?;
    }
    fs::create_dir_all(&run_dir)?;

    let input_dir = run_dir.join("in");
    copy_dir(&config.corpus_seed, &input_dir)?;

    let output_dir = run_dir.join("out");
    fs::create_dir_all(&output_dir)?;

    let log_path = run_dir.join("run.log");
    let log_file = File::create(&log_path)?;
    let log_clone = log_file.try_clone()?;

    let fuzzer_path = config
        .fuzzer_path
        .as_ref()
        .map_or_else(|| PathBuf::from(DEFAULT_FUZZER_PATH), Clone::clone);

    let mut command = Command::new(&fuzzer_path);
    command
        .arg("--input")
        .arg(&input_dir)
        .arg("--output")
        .arg(&output_dir)
        .arg("--share")
        .arg(&config.share_dir)
        .arg("--cores")
        .arg(&config.cores)
        .arg("--timeout")
        .arg(config.timeout_ms.to_string())
        .arg("--bench-snapshot-secs")
        .arg(config.bench_snapshot_secs.to_string());

    // Create new process group so we can terminate all child processes
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_clone))
        .spawn()
        .map_err(|e| CliError::ProcessError(format!("failed to start fuzzer: {e}")))?;

    let deadline = Instant::now() + Duration::from_secs(config.duration);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| CliError::ProcessError(format!("failed to poll fuzzer status: {e}")))?
        {
            log::info!("Fuzzer exited with status {status}");
            break;
        }

        if Instant::now() >= deadline {
            log::info!("Benchmark duration reached, terminating fuzzer");
            kill_process_tree(&mut child);
            break;
        }

        thread::sleep(Duration::from_secs(1));
    }

    aggregate_bench_stats(&run_dir, config, run_idx, suite_path, &fuzzer_path)?;
    write_run_report(&run_dir)?;

    Ok(())
}

/// Aggregate all run_* outputs into suite-level stats.
fn aggregate_suite(root: &Path) -> Result<()> {
    let suite_samples = load_suite_samples(root)?;
    let runs = count_run_dirs(root)?;

    let suite_summary = if suite_samples.is_empty() {
        SuiteSummary {
            runs,
            coverage_mean: None,
            corpus_mean: None,
        }
    } else {
        let suite_series = bucket_mean_series(&suite_samples);
        SuiteSummary {
            runs,
            coverage_mean: suite_series.coverage_mean.last().copied(),
            corpus_mean: suite_series.corpus_mean.last().copied(),
        }
    };

    fs::write(
        root.join("suite_summary.json"),
        serde_json::to_vec_pretty(&suite_summary)?,
    )?;

    Ok(())
}

fn count_run_dirs(root: &Path) -> Result<usize> {
    let mut runs = 0usize;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let is_run = path
            .file_name()
            .and_then(|n| n.to_str())
            .map_or(false, |n| n.starts_with("run_"));
        if !is_run {
            continue;
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            runs += 1;
        }
    }
    Ok(runs)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Err(CliError::FileNotFound(src.display().to_string()));
    }

    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn aggregate_bench_stats(
    run_dir: &Path,
    config: &BenchmarkConfig,
    run_idx: usize,
    suite_path: &Path,
    fuzzer_path: &Path,
) -> Result<()> {
    let bench_dir = run_dir.join("out").join("bench");
    if !bench_dir.exists() {
        return Err(CliError::FileNotFound(bench_dir.display().to_string()));
    }

    let mut merged: Vec<(String, BenchSample)> = Vec::new();
    let mut summary = BenchSummary::default();

    for entry in fs::read_dir(&bench_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext == "csv") {
            continue;
        }
        let cpu = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("bench")
            .to_string();
        let samples = parse_bench_file(&path)?;
        if samples.is_empty() {
            continue;
        }
        for sample in &samples {
            merged.push((cpu.clone(), sample.clone()));
        }
        if let Some(last) = samples.last() {
            summary.final_elapsed_s = summary.final_elapsed_s.max(last.elapsed_s);
            summary.total_execs += last.execs;
            summary.max_coverage_pct = summary.max_coverage_pct.max(last.coverage_pct);
            summary.final_corpus_size = summary.final_corpus_size.max(last.corpus_size);
        }
    }

    if merged.is_empty() {
        return Err(CliError::InvalidInput(format!(
            "no bench CSV files found under {}",
            bench_dir.display()
        )));
    }

    log::info!(
        "found {} bench samples under {}",
        merged.len(),
        bench_dir.display()
    );

    merged.sort_by(|a, b| a.1.elapsed_s.partial_cmp(&b.1.elapsed_s).unwrap());

    let mut stats_csv =
        String::from("cpu,elapsed_s,execs,execs_per_sec,coverage_pct,corpus_size,crashes\n");
    for (cpu, sample) in &merged {
        stats_csv.push_str(&format!(
            "{cpu},{:.3},{},{:.2},{:.4},{},{}\n",
            sample.elapsed_s,
            sample.execs,
            sample.execs_per_sec,
            sample.coverage_pct,
            sample.corpus_size,
            sample.crashes
        ));
    }
    fs::write(run_dir.join("stats.csv"), stats_csv)?;

    if summary.final_elapsed_s > 0.0 {
        summary.mean_execs_per_sec = summary.total_execs as f64 / summary.final_elapsed_s.max(1e-9);
    }

    summary.metadata = Some(BenchMetadata {
        suite: path_to_string(suite_path),
        run_index: run_idx,
        duration_secs: config.duration,
        cores: config.cores.clone(),
        timeout_ms: config.timeout_ms,
        share_dir: path_to_string(&config.share_dir),
        corpus_seed: path_to_string(&config.corpus_seed),
        fuzzer_path: path_to_string(fuzzer_path),
        bench_snapshot_secs: config.bench_snapshot_secs,
        git_commit: git_commit_hash(),
    });

    let summary_path = run_dir.join("summary.json");
    fs::write(summary_path, serde_json::to_vec_pretty(&summary)?)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct BenchSample {
    elapsed_s: f64,
    execs: u64,
    execs_per_sec: f64,
    coverage_pct: f64,
    corpus_size: usize,
    crashes: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BenchSummary {
    final_elapsed_s: f64,
    total_execs: u64,
    mean_execs_per_sec: f64,
    max_coverage_pct: f64,
    final_corpus_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<BenchMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SuiteSummary {
    runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_mean: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    corpus_mean: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchMetadata {
    suite: String,
    run_index: usize,
    duration_secs: u64,
    cores: String,
    timeout_ms: u64,
    share_dir: String,
    corpus_seed: String,
    fuzzer_path: String,
    bench_snapshot_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_commit: Option<String>,
}

fn parse_bench_file(path: &Path) -> Result<Vec<BenchSample>> {
    let contents = fs::read_to_string(path)?;
    let mut samples = Vec::new();
    for (idx, line) in contents.lines().enumerate() {
        if idx == 0 {
            continue;
        }
        let parts: Vec<_> = line.split(',').collect();
        if parts.len() < 6 {
            continue;
        }
        let Ok(elapsed_s) = parts[0].parse() else {
            continue;
        };
        let Ok(execs) = parts[1].parse() else {
            continue;
        };
        let Ok(execs_per_sec) = parts[2].parse() else {
            continue;
        };
        let Ok(coverage_pct) = parts[3].parse() else {
            continue;
        };
        let Ok(corpus_size) = parts[4].parse() else {
            continue;
        };
        let Ok(crashes) = parts[5].parse() else {
            continue;
        };

        samples.push(BenchSample {
            elapsed_s,
            execs,
            execs_per_sec,
            coverage_pct,
            corpus_size,
            crashes,
        });
    }
    Ok(samples)
}

/// Aggregation bucket for averaging across runs at a given elapsed time.
#[derive(Default)]
struct Bucket {
    count: usize,
    coverage_sum: f64,
    corpus_sum: f64,
}

/// Mean coverage/corpus curves across runs, time-bucketed.
struct SuiteSeries {
    elapsed: Vec<f64>,
    coverage_mean: Vec<f64>,
    corpus_mean: Vec<f64>,
}

/// Load all run_* stats.csv files under a suite root and return per-CPU samples.
fn load_suite_samples(root: &Path) -> Result<Vec<(String, BenchSample)>> {
    let mut suite_samples: Vec<(String, BenchSample)> = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .map_or(false, |n| n.starts_with("run_"))
        {
            continue;
        }
        let stats_path = path.join("stats.csv");
        if !stats_path.exists() {
            continue;
        }
        let contents = fs::read_to_string(&stats_path)?;
        suite_samples.extend(parse_stats_csv(&contents));
    }
    Ok(suite_samples)
}

/// Parse merged stats.csv content (cpu, elapsed_s, execs, execs_per_sec, coverage_pct, corpus_size, crashes).
fn parse_stats_csv(contents: &str) -> Vec<(String, BenchSample)> {
    let mut samples = Vec::new();
    for (idx, line) in contents.lines().enumerate() {
        if idx == 0 {
            continue;
        }
        let parts: Vec<_> = line.split(',').collect();
        if parts.len() < 7 {
            continue;
        }
        let cpu = parts[0].to_string();
        let Ok(elapsed_s) = parts[1].parse() else {
            continue;
        };
        let Ok(execs) = parts[2].parse() else {
            continue;
        };
        let Ok(execs_per_sec) = parts[3].parse() else {
            continue;
        };
        let Ok(coverage_pct) = parts[4].parse() else {
            continue;
        };
        let Ok(corpus_size) = parts[5].parse() else {
            continue;
        };
        let Ok(crashes) = parts[6].parse() else {
            continue;
        };
        samples.push((
            cpu,
            BenchSample {
                elapsed_s,
                execs,
                execs_per_sec,
                coverage_pct,
                corpus_size,
                crashes,
            },
        ));
    }
    samples
}

/// Bucket samples by elapsed time (ms): round each sample to the nearest ms, group by that key, and average coverage/corpus per bucket.
fn bucket_mean_series(samples: &[(String, BenchSample)]) -> SuiteSeries {
    let mut buckets: BTreeMap<u64, Bucket> = BTreeMap::new();
    for (_cpu, s) in samples {
        let key = (s.elapsed_s * 1000.0).round() as u64;
        let entry = buckets.entry(key).or_default();
        entry.count += 1;
        entry.coverage_sum += s.coverage_pct;
        entry.corpus_sum += s.corpus_size as f64;
    }

    let mut suite_series = SuiteSeries {
        elapsed: Vec::with_capacity(buckets.len()),
        coverage_mean: Vec::with_capacity(buckets.len()),
        corpus_mean: Vec::with_capacity(buckets.len()),
    };
    for (k, v) in buckets {
        if v.count == 0 {
            continue;
        }
        suite_series.elapsed.push(k as f64 / 1000.0);
        suite_series
            .coverage_mean
            .push(v.coverage_sum / v.count as f64);
        suite_series.corpus_mean.push(v.corpus_sum / v.count as f64);
    }
    suite_series
}

fn write_run_report(run_dir: &Path) -> Result<()> {
    let summary_path = run_dir.join("summary.json");
    if !summary_path.exists() {
        return Ok(());
    }
    let summary_bytes = fs::read(&summary_path)?;
    let summary: BenchSummary =
        serde_json::from_slice(&summary_bytes).map_err(|e| CliError::JsonError(e))?;

    let stats_path = run_dir.join("stats.csv");
    let mut report = String::new();
    report.push_str(&format!("# Benchmark Report ({})\n\n", run_dir.display()));
    report.push_str(&format!(
        "- Final elapsed: {:.2}s\n- Total execs: {}\n- Mean exec/sec: {:.2}\n- Max coverage: {:.4}%\n- Final corpus size: {}\n",
        summary.final_elapsed_s,
        summary.total_execs,
        summary.mean_execs_per_sec,
        summary.max_coverage_pct,
        summary.final_corpus_size
    ));
    if let Some(meta) = &summary.metadata {
        report.push_str("- Metadata:\n");
        report.push_str(&format!("  - Suite: {}\n", meta.suite));
        report.push_str(&format!("  - Run index: {}\n", meta.run_index));
        report.push_str(&format!(
            "  - Duration target (s): {}\n",
            meta.duration_secs
        ));
        report.push_str(&format!("  - Cores: {}\n", meta.cores));
        report.push_str(&format!("  - Timeout (ms): {}\n", meta.timeout_ms));
        report.push_str(&format!("  - Share dir: {}\n", meta.share_dir));
        report.push_str(&format!("  - Corpus seed: {}\n", meta.corpus_seed));
        report.push_str(&format!("  - Fuzzer: {}\n", meta.fuzzer_path));
        report.push_str(&format!(
            "  - Bench snapshot interval (s): {}\n",
            meta.bench_snapshot_secs
        ));
        if let Some(commit) = &meta.git_commit {
            report.push_str(&format!("  - Git commit: {}\n", commit));
        }
    }
    report.push('\n');
    report.push_str(&format!(
        "[stats.csv]({}) | [summary.json]({})\n",
        stats_path.display(),
        summary_path.display()
    ));
    fs::write(run_dir.join("report.md"), report)?;
    Ok(())
}

fn compare_runs(
    baseline_dir: &Path,
    candidate_dir: &Path,
    output: Option<&Path>,
    suite_level: bool,
) -> Result<()> {
    let mut report = String::new();
    writeln!(
        &mut report,
        "# Benchmark Comparison\n\n- Baseline: {}\n- Candidate: {}\n",
        baseline_dir.display(),
        candidate_dir.display()
    )
    .expect("writing to string cannot fail");

    if suite_level {
        let baseline = load_suite_summary(baseline_dir)?;
        let candidate = load_suite_summary(candidate_dir)?;

        if let (Some(base_cov), Some(cand_cov)) = (baseline.coverage_mean, candidate.coverage_mean)
        {
            write_diff_line_f64(&mut report, "Mean coverage (%)", base_cov, cand_cov);
        }
        if let (Some(base_corpus), Some(cand_corpus)) =
            (baseline.corpus_mean, candidate.corpus_mean)
        {
            write_diff_line_f64(&mut report, "Mean corpus size", base_corpus, cand_corpus);
        }
    } else {
        let baseline = load_summary(baseline_dir)?;
        let candidate = load_summary(candidate_dir)?;

        write_diff_line_u64(
            &mut report,
            "Total execs",
            baseline.total_execs,
            candidate.total_execs,
        );
        write_diff_line_f64(
            &mut report,
            "Mean exec/sec",
            baseline.mean_execs_per_sec,
            candidate.mean_execs_per_sec,
        );
        write_diff_line_f64(
            &mut report,
            "Max coverage (%)",
            baseline.max_coverage_pct,
            candidate.max_coverage_pct,
        );
        write_diff_line_u64(
            &mut report,
            "Final corpus size",
            baseline.final_corpus_size as u64,
            candidate.final_corpus_size as u64,
        );
    }

    report.push('\n');

    if let Some(path) = output {
        fs::write(path, &report)?;
        println!("Wrote comparison report to {}", path.display());
    } else {
        print!("{report}");
    }

    Ok(())
}

fn load_summary(run_dir: &Path) -> Result<BenchSummary> {
    let summary_path = run_dir.join("summary.json");
    if !summary_path.exists() {
        return Err(CliError::FileNotFound(summary_path.display().to_string()));
    }
    let summary_bytes = fs::read(&summary_path)?;
    let summary: BenchSummary = serde_json::from_slice(&summary_bytes)?;
    Ok(summary)
}

fn load_suite_summary(root: &Path) -> Result<SuiteSummary> {
    let suite_summary_path = root.join("suite_summary.json");
    if !suite_summary_path.exists() {
        return Err(CliError::FileNotFound(
            suite_summary_path.display().to_string(),
        ));
    }
    let bytes = fs::read(&suite_summary_path)?;
    let summary: SuiteSummary = serde_json::from_slice(&bytes)?;
    Ok(summary)
}

fn write_diff_line_f64(buf: &mut String, label: &str, baseline: f64, candidate: f64) {
    let delta = candidate - baseline;
    let _ = writeln!(
        buf,
        "- {label}: {candidate:.4} (delta {delta:+.4} vs {baseline:.4})"
    );
}

fn write_diff_line_u64(buf: &mut String, label: &str, baseline: u64, candidate: u64) {
    let delta = candidate as i128 - baseline as i128;
    let _ = writeln!(
        buf,
        "- {label}: {candidate} (delta {delta:+} vs {baseline})"
    );
}

fn path_to_string(path: &Path) -> String {
    path.display().to_string()
}

fn git_commit_hash() -> Option<String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit = String::from_utf8(output.stdout).ok()?;
    Some(commit.trim().to_string())
}

/// Gracefully terminate a process and all its children in the process group.
///
/// Sends SIGTERM first for graceful shutdown, waits briefly, then SIGKILL if needed.
#[cfg(unix)]
fn kill_process_tree(child: &mut Child) {
    let pid = Pid::from_raw(child.id() as i32);

    // Get the process group ID
    let pgid = match getpgid(Some(pid)) {
        Ok(pgid) => pgid,
        Err(_) => {
            // Fallback: if we can't get PGID, just kill the process
            let _ = kill(pid, Signal::SIGTERM);
            thread::sleep(Duration::from_secs(2));
            if child.try_wait().ok().flatten().is_none() {
                let _ = kill(pid, Signal::SIGKILL);
            }
            let _ = child.wait();
            return;
        }
    };

    // Try graceful shutdown first
    let _ = killpg(pgid, Signal::SIGTERM);

    // Give processes time to clean up
    thread::sleep(Duration::from_secs(2));

    // Force kill if still running
    if child.try_wait().ok().flatten().is_none() {
        let _ = killpg(pgid, Signal::SIGKILL);
    }

    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let suffix: u64 = rand::random();
        path.push(format!("{prefix}-{suffix}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn write_bench_csv(path: &Path, rows: &[(f64, u64, f64, f64, usize, usize)]) {
        let mut buf =
            String::from("elapsed_s,execs,execs_per_sec,coverage_pct,corpus_size,crashes\n");
        for (elapsed_s, execs, execs_per_sec, coverage_pct, corpus_size, crashes) in rows {
            buf.push_str(&format!(
                "{elapsed_s:.3},{execs},{execs_per_sec:.2},{coverage_pct:.4},{corpus_size},{crashes}\n"
            ));
        }
        fs::write(path, buf).expect("write bench csv");
    }

    #[test]
    fn aggregate_writes_run_and_suite_artifacts() {
        let root = make_temp_dir("fuzzamoto-bench");
        let run_dir = root.join("run_00");
        let bench_dir = run_dir.join("out").join("bench");
        fs::create_dir_all(&bench_dir).unwrap();

        write_bench_csv(
            &bench_dir.join("bench-cpu_000.csv"),
            &[
                (0.0, 0, 0.0, 0.0, 1, 0),
                (30.0, 60_000, 2000.0, 4.0, 120, 0),
                (60.0, 120_000, 2000.0, 5.0, 150, 0),
            ],
        );

        let config = BenchmarkConfig {
            duration: 60,
            runs: 1,
            cores: "0".to_string(),
            timeout_ms: 1_000,
            share_dir: PathBuf::from("/tmp/share"),
            corpus_seed: PathBuf::from("/tmp/corpus"),
            fuzzer_path: Some(PathBuf::from("/tmp/fuzzer")),
            bench_snapshot_secs: 30,
        };

        aggregate_bench_stats(
            &run_dir,
            &config,
            0,
            Path::new("/tmp/suite.yaml"),
            Path::new("/tmp/fuzzer"),
        )
        .unwrap();

        let stats = fs::read_to_string(run_dir.join("stats.csv")).unwrap();
        assert!(stats.contains("cpu,elapsed_s,execs"));

        let summary_bytes = fs::read(run_dir.join("summary.json")).unwrap();
        let summary: BenchSummary = serde_json::from_slice(&summary_bytes).unwrap();
        assert_eq!(summary.total_execs, 120_000);
        assert_eq!(summary.final_corpus_size, 150);

        aggregate_suite(&root).unwrap();
        let suite_bytes = fs::read(root.join("suite_summary.json")).unwrap();
        let suite: SuiteSummary = serde_json::from_slice(&suite_bytes).unwrap();
        assert_eq!(suite.runs, 1);
        assert!(suite.coverage_mean.unwrap() > 4.9);
    }

    #[test]
    fn aggregate_suite_writes_summary_even_without_samples() {
        let root = make_temp_dir("fuzzamoto-suite-empty");
        fs::create_dir_all(root.join("run_00")).unwrap();
        aggregate_suite(&root).unwrap();

        let suite_bytes = fs::read(root.join("suite_summary.json")).unwrap();
        let suite: SuiteSummary = serde_json::from_slice(&suite_bytes).unwrap();
        assert_eq!(suite.runs, 1);
        assert!(suite.coverage_mean.is_none());
        assert!(suite.corpus_mean.is_none());
    }

    #[test]
    fn compare_runs_writes_markdown_report() {
        let root = make_temp_dir("fuzzamoto-compare");
        let base = root.join("base");
        let cand = root.join("cand");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&cand).unwrap();

        let baseline = BenchSummary {
            final_elapsed_s: 60.0,
            total_execs: 120_000,
            mean_execs_per_sec: 2000.0,
            max_coverage_pct: 5.0,
            final_corpus_size: 150,
            metadata: None,
        };
        let candidate = BenchSummary {
            final_elapsed_s: 60.0,
            total_execs: 135_000,
            mean_execs_per_sec: 2250.0,
            max_coverage_pct: 5.3,
            final_corpus_size: 160,
            metadata: None,
        };
        fs::write(
            base.join("summary.json"),
            serde_json::to_vec_pretty(&baseline).unwrap(),
        )
        .unwrap();
        fs::write(
            cand.join("summary.json"),
            serde_json::to_vec_pretty(&candidate).unwrap(),
        )
        .unwrap();

        let out = root.join("compare.md");
        compare_runs(&base, &cand, Some(&out), false).unwrap();
        let report = fs::read_to_string(&out).unwrap();
        assert!(report.contains("Benchmark Comparison"));
        assert!(report.contains("Total execs: 135000"));
    }
}

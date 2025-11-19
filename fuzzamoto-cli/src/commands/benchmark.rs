use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use clap::Subcommand;
use serde::{Deserialize, Serialize};

use crate::error::{CliError, Result};

const DEFAULT_FUZZER_PATH: &str = "target/release/fuzzamoto-libafl";

pub struct BenchmarkCommand;

impl BenchmarkCommand {
    pub fn execute(cmd: &BenchmarkCommands) -> Result<()> {
        match cmd {
            BenchmarkCommands::Run { suite, output } => run_suite(suite, output),
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
}

fn default_timeout_ms() -> u64 {
    1_000
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
        run_single(&config, run_idx, output)?;
    }

    Ok(())
}

fn run_single(config: &BenchmarkConfig, run_idx: usize, root: &Path) -> Result<()> {
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

    let mut child = Command::new(&fuzzer_path)
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
            child
                .kill()
                .map_err(|e| CliError::ProcessError(format!("failed to kill fuzzer: {e}")))?;
            let _ = child.wait();
            break;
        }

        thread::sleep(Duration::from_secs(1));
    }

    aggregate_bench_stats(&run_dir)?;
    write_run_report(&run_dir)?;

    Ok(())
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

fn aggregate_bench_stats(run_dir: &Path) -> Result<()> {
    let bench_dir = run_dir.join("out").join("bench");
    if !bench_dir.exists() {
        log::warn!(
            "bench directory missing for {}, skipping aggregation",
            run_dir.display()
        );
        return Ok(());
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
            summary.final_corpus_size += last.corpus_size;
        }
    }

    if merged.is_empty() {
        log::warn!(
            "no bench CSV files found under {}, skipping aggregation",
            bench_dir.display()
        );
        return Ok(());
    }

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
    report.push('\n');
    report.push_str(&format!(
        "[stats.csv]({}) | [summary.json]({})\n",
        stats_path.display(),
        summary_path.display()
    ));
    fs::write(run_dir.join("report.md"), report)?;
    Ok(())
}

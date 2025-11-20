use std::{
    ffi::OsStr,
    fmt::Write,
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
            BenchmarkCommands::Compare {
                baseline,
                candidate,
                output,
            } => compare_runs(baseline, candidate, output.as_ref().map(PathBuf::as_path)),
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
        #[arg(long, help = "Baseline run directory (must contain summary.json)")]
        baseline: PathBuf,
        #[arg(long, help = "Candidate run directory (must contain summary.json)")]
        candidate: PathBuf,
        #[arg(long, help = "Optional path to write a comparison report (Markdown)")]
        output: Option<PathBuf>,
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

    compute_relcov_and_hist(run_dir, &mut summary)?;

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
    unique_edges: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    edge_histogram: Option<EdgeHistogram>,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_cpu_relcov: Option<Vec<RelcovEntry>>,
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

fn compute_relcov_and_hist(run_dir: &Path, summary: &mut BenchSummary) -> Result<()> {
    let bench_dir = run_dir.join("out").join("bench");
    if !bench_dir.exists() {
        return Ok(());
    }

    let mut cpu_maps: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in fs::read_dir(&bench_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension() != Some(OsStr::new("bin")) {
            continue;
        }
        let cpu = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let data = fs::read(&path)?;
        if data.is_empty() {
            continue;
        }
        cpu_maps.push((cpu, data));
    }

    if cpu_maps.is_empty() {
        return Ok(());
    }

    let map_len = cpu_maps[0].1.len();
    if map_len == 0 {
        return Ok(());
    }
    if cpu_maps.iter().any(|(_, map)| map.len() != map_len) {
        log::warn!(
            "coverage map sizes differ under {}, skipping relcov aggregation",
            bench_dir.display()
        );
        return Ok(());
    }

    let mut union_map = vec![0u8; map_len];
    for (_, map) in &cpu_maps {
        for (idx, &byte) in map.iter().enumerate() {
            if byte > union_map[idx] {
                union_map[idx] = byte;
            }
        }
    }

    let mut histogram = EdgeHistogram::default();
    for byte in &union_map {
        match *byte {
            0 => {}
            1 => histogram.hit_1 += 1,
            2 | 3 => histogram.hit_2_3 += 1,
            _ => histogram.hit_ge_4 += 1,
        }
    }
    let total_edges = histogram.total_edges();
    summary.unique_edges = Some(total_edges);
    summary.edge_histogram = Some(histogram);

    let per_cpu: Vec<_> = cpu_maps
        .into_iter()
        .map(|(cpu, map)| {
            let edges = map.iter().filter(|&&b| b > 0).count();
            RelcovEntry {
                cpu,
                edges,
                relcov_pct: if total_edges > 0 {
                    (edges as f64 / total_edges as f64) * 100.0
                } else {
                    0.0
                },
            }
        })
        .collect();
    summary.per_cpu_relcov = Some(per_cpu);

    Ok(())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct EdgeHistogram {
    hit_1: usize,
    hit_2_3: usize,
    hit_ge_4: usize,
}

impl EdgeHistogram {
    fn total_edges(&self) -> usize {
        self.hit_1 + self.hit_2_3 + self.hit_ge_4
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct RelcovEntry {
    cpu: String,
    edges: usize,
    relcov_pct: f64,
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
    if let Some(edges) = summary.unique_edges {
        report.push_str(&format!("- Unique edges: {}\n", edges));
    }
    if let Some(hist) = &summary.edge_histogram {
        report.push_str(&format!(
            "- Edge histogram: 1-hit={} | 2-3 hits={} | >=4 hits={}\n",
            hist.hit_1, hist.hit_2_3, hist.hit_ge_4
        ));
    }
    if let Some(relcov) = &summary.per_cpu_relcov {
        report.push_str("- Per-CPU coverage share:\n");
        for entry in relcov {
            report.push_str(&format!(
                "  - {}: {} edges ({:.2}%)\n",
                entry.cpu, entry.edges, entry.relcov_pct
            ));
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

fn compare_runs(baseline_dir: &Path, candidate_dir: &Path, output: Option<&Path>) -> Result<()> {
    let baseline = load_summary(baseline_dir)?;
    let candidate = load_summary(candidate_dir)?;

    let mut report = String::new();
    writeln!(
        &mut report,
        "# Benchmark Comparison\n\n- Baseline: {}\n- Candidate: {}\n",
        baseline_dir.display(),
        candidate_dir.display()
    )
    .expect("writing to string cannot fail");

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
    if let (Some(base_edges), Some(cand_edges)) = (baseline.unique_edges, candidate.unique_edges) {
        write_diff_line_u64(
            &mut report,
            "Unique edges",
            base_edges as u64,
            cand_edges as u64,
        );
    }
    if let (Some(base_hist), Some(cand_hist)) = (
        baseline.edge_histogram.as_ref(),
        candidate.edge_histogram.as_ref(),
    ) {
        writeln!(
            &mut report,
            "- Edge histogram: 1-hit {} -> {}, 2-3 hits {} -> {}, >=4 hits {} -> {}",
            base_hist.hit_1,
            cand_hist.hit_1,
            base_hist.hit_2_3,
            cand_hist.hit_2_3,
            base_hist.hit_ge_4,
            cand_hist.hit_ge_4
        )
        .expect("writing to string cannot fail");
    }

    if let (Some(base_relcov), Some(cand_relcov)) = (
        baseline.per_cpu_relcov.as_ref(),
        candidate.per_cpu_relcov.as_ref(),
    ) {
        let base_mean = mean_relcov(base_relcov);
        let cand_mean = mean_relcov(cand_relcov);
        write_diff_line_f64(&mut report, "Mean per-CPU relcov (%)", base_mean, cand_mean);
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

fn mean_relcov(entries: &[RelcovEntry]) -> f64 {
    if entries.is_empty() {
        return 0.0;
    }
    let sum: f64 = entries.iter().map(|e| e.relcov_pct).sum();
    sum / entries.len() as f64
}

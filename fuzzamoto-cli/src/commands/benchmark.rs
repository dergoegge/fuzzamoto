use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    fmt::Write,
    fs::{self, File},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use clap::Subcommand;
use serde::{Deserialize, Serialize};

use crate::error::{CliError, Result};

const DEFAULT_FUZZER_PATH: &str = "target/release/fuzzamoto-libafl";
// TODO: consider making bench snapshot configurable instead of hardcoding 30s.
const BENCH_SNAPSHOT_SECS: u64 = 30;

pub struct BenchmarkCommand;

impl BenchmarkCommand {
    pub fn execute(cmd: &BenchmarkCommands) -> Result<()> {
        match cmd {
            BenchmarkCommands::Run {
                suite,
                output,
                html,
            } => run_suite(suite, output, *html),
            BenchmarkCommands::Compare {
                baseline,
                candidate,
                output,
                html,
                suite,
            } => compare_runs(
                baseline,
                candidate,
                output.as_ref().map(PathBuf::as_path),
                *html,
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
        #[arg(
            long,
            default_value_t = false,
            help = "Also write report.html with plots"
        )]
        html: bool,
    },
    /// Compare two benchmark run directories and report deltas
    Compare {
        #[arg(long, help = "Baseline run directory (must contain summary.json)")]
        baseline: PathBuf,
        #[arg(long, help = "Candidate run directory (must contain summary.json)")]
        candidate: PathBuf,
        #[arg(long, help = "Optional path to write a comparison report (Markdown)")]
        output: Option<PathBuf>,
        #[arg(
            long,
            default_value_t = false,
            help = "Also write compare.html with coverage/corpus charts"
        )]
        html: bool,
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
}

fn default_timeout_ms() -> u64 {
    1_000
}

fn run_suite(suite: &PathBuf, output: &PathBuf, write_html: bool) -> Result<()> {
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
        run_single(&config, run_idx, output, suite, write_html)?;
    }

    aggregate_suite(output, write_html)?;
    Ok(())
}

fn run_single(
    config: &BenchmarkConfig,
    run_idx: usize,
    root: &Path,
    suite_path: &Path,
    write_html: bool,
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

    aggregate_bench_stats(
        &run_dir,
        config,
        run_idx,
        suite_path,
        &fuzzer_path,
        write_html,
    )?;
    write_run_report(&run_dir)?;

    Ok(())
}

/// Aggregate all run_* outputs into suite-level stats and optional HTML.
fn aggregate_suite(root: &Path, write_html: bool) -> Result<()> {
    let suite_samples = load_suite_samples(root)?;

    if suite_samples.is_empty() {
        return Ok(());
    }

    let suite_series = bucket_mean_series(&suite_samples);
    let suite_json = serde_json::to_string(&suite_series)?;
    if write_html {
        write_suite_report_html(root, &suite_json)?;
    }

    Ok(())
}

fn write_suite_report_html(root: &Path, suite_series_json: &str) -> Result<()> {
    // Reuse multi-series renderer with two charts (coverage/corpus means).
    let charts = [
        ChartSpec {
            div_id: "suite_coverage",
            title: "Coverage (%) vs Time (mean across runs)",
            y_title: "Coverage (%)",
            field: "coverage_mean",
        },
        ChartSpec {
            div_id: "suite_corpus",
            title: "Corpus Size vs Time (mean across runs)",
            y_title: "Corpus size",
            field: "corpus_mean",
        },
    ];
    let html = render_multi_series("Fuzzamoto Bench Suite Report", suite_series_json, &charts);

    fs::write(root.join("suite_report.html"), html)?;
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

fn aggregate_bench_stats(
    run_dir: &Path,
    config: &BenchmarkConfig,
    run_idx: usize,
    suite_path: &Path,
    fuzzer_path: &Path,
    write_html: bool,
) -> Result<()> {
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
    summary.mutation_stats = compute_mutation_stats(run_dir)?;

    summary.metadata = Some(BenchMetadata {
        suite: path_to_string(suite_path),
        run_index: run_idx,
        duration_secs: config.duration,
        cores: config.cores.clone(),
        timeout_ms: config.timeout_ms,
        share_dir: path_to_string(&config.share_dir),
        corpus_seed: path_to_string(&config.corpus_seed),
        fuzzer_path: path_to_string(fuzzer_path),
        bench_snapshot_secs: BENCH_SNAPSHOT_SECS,
        git_commit: git_commit_hash(),
    });

    let summary_path = run_dir.join("summary.json");
    fs::write(summary_path, serde_json::to_vec_pretty(&summary)?)?;

    if write_html {
        write_run_report_html(run_dir, &merged, &summary)?;
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    mutation_stats: Option<MutationStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<BenchMetadata>,
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

fn compute_relcov_and_hist(run_dir: &Path, summary: &mut BenchSummary) -> Result<()> {
    let bench_dir = run_dir.join("out").join("bench");
    if !bench_dir.exists() {
        return Ok(());
    }

    let cpu_maps = load_cpu_maps(&bench_dir)?;
    if cpu_maps.is_empty() {
        return Ok(());
    }

    let Some(union_map) = build_union_map(&cpu_maps, &bench_dir) else {
        return Ok(());
    };
    if union_map.is_empty() {
        return Ok(());
    }

    let (histogram, total_edges) = histogram_from_union(&union_map);
    summary.unique_edges = Some(total_edges);
    summary.edge_histogram = Some(histogram);
    summary.per_cpu_relcov = Some(compute_per_cpu_relcov(&cpu_maps, total_edges));

    Ok(())
}

/// Load per-CPU Nyx coverage bitmaps from `bench/bench-cpu_*.bin`.
fn load_cpu_maps(bench_dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut cpu_maps = Vec::new();
    for entry in fs::read_dir(bench_dir)? {
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
    Ok(cpu_maps)
}

/// Validate map sizes and OR them into a single union coverage map.
fn build_union_map(cpu_maps: &[(String, Vec<u8>)], bench_dir: &Path) -> Option<Vec<u8>> {
    let map_len = cpu_maps.first()?.1.len();
    if map_len == 0 {
        return None;
    }
    // All per-CPU maps must be the same size; otherwise relcov/hist are meaningless.
    if cpu_maps.iter().any(|(_, map)| map.len() != map_len) {
        log::warn!(
            "coverage map sizes differ under {}, skipping relcov aggregation",
            bench_dir.display()
        );
        return None;
    }

    let mut union_map = vec![0u8; map_len];
    for (_, map) in cpu_maps {
        for (idx, &byte) in map.iter().enumerate() {
            if byte > union_map[idx] {
                union_map[idx] = byte;
            }
        }
    }
    Some(union_map)
}

/// Build the edge-count histogram (1-hit, 2–3, 4+) from the union map.
fn histogram_from_union(union_map: &[u8]) -> (EdgeHistogram, usize) {
    let mut histogram = EdgeHistogram::default();
    for &byte in union_map {
        match byte {
            0 => {}
            1 => histogram.hit_1 += 1,
            2 | 3 => histogram.hit_2_3 += 1,
            _ => histogram.hit_ge_4 += 1,
        }
    }
    let total_edges = histogram.total_edges();
    (histogram, total_edges)
}

/// Compute per-CPU relative coverage (edges per CPU / union edges).
fn compute_per_cpu_relcov(cpu_maps: &[(String, Vec<u8>)], total_edges: usize) -> Vec<RelcovEntry> {
    cpu_maps
        .iter()
        .map(|(cpu, map)| {
            let edges = map.iter().filter(|&&b| b > 0).count();
            RelcovEntry {
                cpu: cpu.clone(),
                edges,
                relcov_pct: if total_edges > 0 {
                    (edges as f64 / total_edges as f64) * 100.0
                } else {
                    0.0
                },
            }
        })
        .collect()
}

fn compute_mutation_stats(run_dir: &Path) -> Result<Option<MutationStats>> {
    let bench_dir = run_dir.join("out").join("bench");
    if !bench_dir.exists() {
        return Ok(None);
    }

    let mut records = Vec::new();
    for entry in fs::read_dir(&bench_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension() != Some(OsStr::new("jsonl")) {
            continue;
        }
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !file_name.contains("mutations") {
            continue;
        }
        let file = File::open(&path)?;
        let reader = BufReader::new(file);
        for (idx, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<MutationRecord>(&line) {
                Ok(rec) => records.push(rec),
                Err(e) => log::warn!(
                    "failed to parse mutation record {} in {}: {e}",
                    idx,
                    path.display()
                ),
            }
        }
    }

    let total_records = records.iter().filter(|r| !r.chain.is_empty()).count();
    if total_records == 0 {
        return Ok(None);
    }

    let mut chain_counts: HashMap<Vec<String>, usize> = HashMap::new();

    for rec in records.into_iter().filter(|r| !r.chain.is_empty()) {
        *chain_counts.entry(rec.chain.clone()).or_default() += 1;
    }

    let make_chain_stats = |mut counts: Vec<(Vec<String>, usize)>, total: usize| {
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        counts
            .into_iter()
            .take(10)
            .map(|(chain, count)| ChainStat {
                chain,
                count,
                share_pct: (count as f64 / total as f64) * 100.0,
            })
            .collect::<Vec<_>>()
    };

    let top_chains = make_chain_stats(chain_counts.into_iter().collect(), total_records);

    Ok(Some(MutationStats {
        total_records,
        top_chains,
    }))
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

#[derive(Debug, Serialize, Deserialize)]
struct MutationRecord {
    cpu: u32,
    kind: String,
    corpus_id: usize,
    len: usize,
    chain: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MutationStats {
    total_records: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    top_chains: Vec<ChainStat>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChainStat {
    chain: Vec<String>,
    count: usize,
    share_pct: f64,
}

/// Per-CPU time series extracted from stats.csv for plotting.
#[derive(Serialize)]
struct CpuSeries {
    cpu: String,
    elapsed: Vec<f64>,
    coverage: Vec<f64>,
    corpus: Vec<usize>,
}

/// Aggregation bucket for averaging across runs at a given elapsed time.
#[derive(Default)]
struct Bucket {
    count: usize,
    coverage_sum: f64,
    corpus_sum: f64,
}

/// Mean coverage/corpus curves across runs, time-bucketed.
#[derive(Serialize)]
struct SuiteSeries {
    elapsed: Vec<f64>,
    coverage_mean: Vec<f64>,
    corpus_mean: Vec<f64>,
}

/// Parse a merged stats.csv string (with cpu column) into per-CPU series.
fn parse_stats_series(contents: &str) -> Vec<CpuSeries> {
    let mut by_cpu: HashMap<String, Vec<(f64, f64, usize)>> = HashMap::new();
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
        let Ok(coverage_pct) = parts[4].parse() else {
            continue;
        };
        let Ok(corpus_size) = parts[5].parse() else {
            continue;
        };
        by_cpu
            .entry(cpu)
            .or_default()
            .push((elapsed_s, coverage_pct, corpus_size));
    }
    let mut out = Vec::new();
    for (cpu, mut samples) in by_cpu {
        samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut elapsed = Vec::with_capacity(samples.len());
        let mut coverage = Vec::with_capacity(samples.len());
        let mut corpus = Vec::with_capacity(samples.len());
        for (e, c, cor) in samples {
            elapsed.push(e);
            coverage.push(c);
            corpus.push(cor);
        }
        out.push(CpuSeries {
            cpu,
            elapsed,
            coverage,
            corpus,
        });
    }
    out
}

/// Group per-CPU series from merged samples (cpu, BenchSample).
fn group_samples_by_cpu(samples: &[(String, BenchSample)]) -> Vec<CpuSeries> {
    let mut by_cpu: HashMap<String, Vec<&BenchSample>> = HashMap::new();
    for (cpu, sample) in samples {
        by_cpu.entry(cpu.clone()).or_default().push(sample);
    }

    let mut series: Vec<CpuSeries> = Vec::new();
    for (cpu, samples) in by_cpu {
        let mut elapsed = Vec::with_capacity(samples.len());
        let mut coverage = Vec::with_capacity(samples.len());
        let mut corpus = Vec::with_capacity(samples.len());
        for s in samples {
            elapsed.push(s.elapsed_s);
            coverage.push(s.coverage_pct);
            corpus.push(s.corpus_size);
        }
        series.push(CpuSeries {
            cpu,
            elapsed,
            coverage,
            corpus,
        });
    }
    series
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
            suite_samples.push((
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
    }
    Ok(suite_samples)
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

fn render_compare_series(
    title: &str,
    base_series_json: &str,
    cand_series_json: &str,
    coverage_title: &str,
    corpus_title: &str,
) -> String {
    // Compare two sets of Series: plots baseline vs candidate coverage/corpus.
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>{title}</title>
  <script src="https://cdn.jsdelivr.net/npm/plotly.js-dist-min@2.29.1/plotly.min.js"></script>
  <style>
    body {{ font-family: sans-serif; margin: 16px; }}
    .chart {{ width: 100%; max-width: 1100px; height: 420px; margin-bottom: 28px; }}
  </style>
</head>
<body>
  <h1>{title}</h1>
  <div id="cov" class="chart"></div>
  <div id="corpus" class="chart"></div>
  <script>
    const base = {base};
    const cand = {cand};

    function plot(div, field, plotTitle, yTitle) {{
      Plotly.newPlot(div, [
        {{
          x: base.elapsed,
          y: base[field],
          mode: 'lines',
          name: 'baseline'
        }},
        {{
          x: cand.elapsed,
          y: cand[field],
          mode: 'lines',
          name: 'candidate'
        }}
      ], {{
        title: plotTitle,
        xaxis: {{ title: 'Elapsed (s)' }},
        yaxis: {{ title: yTitle }},
        legend: {{ orientation: 'h' }}
      }});
    }}

    plot('cov', 'coverage_mean', '{cov_title}', 'Coverage (%)');
    plot('corpus', 'corpus_mean', '{corpus_title}', 'Corpus size');
  </script>
</body>
</html>
"#,
        title = title,
        base = base_series_json,
        cand = cand_series_json,
        cov_title = coverage_title,
        corpus_title = corpus_title
    )
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
    if let Some(attr) = &summary.mutation_stats {
        report.push_str(&format!(
            "- Mutation stats ({} records):\n",
            attr.total_records
        ));
        if !attr.top_chains.is_empty() {
            report.push_str("  - Top mutation chains:\n");
            for stat in attr.top_chains.iter().take(5) {
                report.push_str(&format!(
                    "    - [{}] x{} ({:.2}%)\n",
                    stat.chain.join(" -> "),
                    stat.count,
                    stat.share_pct
                ));
            }
        }
    }
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

fn write_run_report_html(
    run_dir: &Path,
    samples: &[(String, BenchSample)],
    summary: &BenchSummary,
) -> Result<()> {
    if samples.is_empty() {
        return Ok(());
    }

    let series_json = serde_json::to_string(&group_samples_by_cpu(samples))?;
    let summary_json = serde_json::to_string(summary)?;
    fs::write(
        run_dir.join("report.html"),
        render_run_report("Fuzzamoto Bench Report", &series_json, &summary_json),
    )?;
    Ok(())
}

struct ChartSpec {
    div_id: &'static str,
    title: &'static str,
    y_title: &'static str,
    field: &'static str,
}

/// Render an HTML page with multiple per-CPU line charts from a JSON array of CpuSeries.
fn render_multi_series(title: &str, series_json: &str, charts: &[ChartSpec]) -> String {
    let chart_divs = charts
        .iter()
        .map(|c| format!(r#"<div id="{id}" class="chart"></div>"#, id = c.div_id))
        .collect::<Vec<_>>()
        .join("\n  ");

    let plot_calls = charts
        .iter()
        .map(|c| {
            format!(
                r#"    (function() {{
      const traces = series.map(s => ({{
        x: s.elapsed,
        y: s.{field},
        mode: 'lines',
        name: s.cpu
      }}));
      Plotly.newPlot('{div}', traces, {{
        title: '{title}',
        xaxis: {{ title: 'Elapsed (s)' }},
        yaxis: {{ title: '{ytitle}' }},
        legend: {{ orientation: 'h' }}
      }});
    }})();"#,
                field = c.field,
                div = c.div_id,
                title = c.title,
                ytitle = c.y_title
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>{title}</title>
  <script src="https://cdn.jsdelivr.net/npm/plotly.js-dist-min@2.29.1/plotly.min.js"></script>
  <style>
    body {{ font-family: sans-serif; margin: 16px; }}
    .chart {{ width: 100%; max-width: 1100px; height: 420px; margin-bottom: 28px; }}
  </style>
</head>
<body>
  <h1>{title}</h1>
  {divs}
  <script>
    const series = {series};
{plots}
  </script>
</body>
</html>
"#,
        title = title,
        divs = chart_divs,
        series = series_json,
        plots = plot_calls
    )
}

/// Render run-level report with time-series plus relcov/histogram (if available in summary).
fn render_run_report(title: &str, series_json: &str, summary_json: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>{title}</title>
  <script src="https://cdn.jsdelivr.net/npm/plotly.js-dist-min@2.29.1/plotly.min.js"></script>
  <style>
    body {{ font-family: sans-serif; margin: 16px; }}
    .chart {{ width: 100%; max-width: 1100px; height: 420px; margin-bottom: 28px; }}
  </style>
</head>
<body>
  <h1>{title}</h1>
  <div id="coverage" class="chart"></div>
  <div id="corpus" class="chart"></div>
  <div id="edge_hist" class="chart"></div>
  <div id="relcov" class="chart"></div>
  <script>
    const series = {series_json};
    const summary = {summary_json};

    // Coverage/Corpus time series
    const chartSpecs = [
      {{ div: 'coverage', field: 'coverage', title: 'Coverage (%) vs Time', y: 'Coverage (%)' }},
      {{ div: 'corpus',   field: 'corpus',   title: 'Corpus Size vs Time', y: 'Corpus size'   }},
    ];
    chartSpecs.forEach(spec => {{
      const traces = series.map(s => ({
        x: s.elapsed,
        y: s[spec.field],
        mode: 'lines',
        name: s.cpu,
      }));
      Plotly.newPlot(spec.div, traces, {{
        title: spec.title,
        xaxis: {{ title: 'Elapsed (s)' }},
        yaxis: {{ title: spec.y }},
        legend: {{ orientation: 'h' }}
      }});
    }});

    // Edge histogram
    if (summary.edge_histogram) {{
      Plotly.newPlot('edge_hist', [{
        type: 'bar',
        x: ['1-hit', '2-3 hits', '>=4 hits'],
        y: [summary.edge_histogram.hit_1, summary.edge_histogram.hit_2_3, summary.edge_histogram.hit_ge_4],
        name: 'edges'
      }], {{
        title: 'Edge Histogram',
        xaxis: {{title: 'Bucket'}},
        yaxis: {{title: 'Count'}},
        legend: {{orientation: 'h'}}
      }});
    }}

    // Per-CPU relcov
    if (summary.per_cpu_relcov) {{
      const cpus = summary.per_cpu_relcov.map(e => e.cpu);
      const relcov = summary.per_cpu_relcov.map(e => e.relcov_pct);
      Plotly.newPlot('relcov', [{
        type: 'bar',
        x: cpus,
        y: relcov,
        name: 'relcov'
      }], {{
        title: 'Per-CPU Relative Coverage (%)',
        xaxis: {{title: 'CPU'}},
        yaxis: {{title: 'Relcov (%)'}},
        legend: {{orientation: 'h'}}
      }});
    }}
  </script>
</body>
</html>
"#,
        title = title,
        series_json = series_json,
        summary_json = summary_json,
    )
}
fn compare_runs(
    baseline_dir: &Path,
    candidate_dir: &Path,
    output: Option<&Path>,
    write_html: bool,
    suite_level: bool,
) -> Result<()> {
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

    if write_html {
        if suite_level {
            write_compare_suite_html(baseline_dir, candidate_dir)?;
        } else {
            write_compare_report_html(baseline_dir, candidate_dir)?;
        }
    }

    Ok(())
}

/// Write an HTML comparison plotting coverage and corpus per CPU for baseline vs candidate.
fn write_compare_report_html(baseline_dir: &Path, candidate_dir: &Path) -> Result<()> {
    let base_stats = fs::read_to_string(baseline_dir.join("stats.csv"))?;
    let cand_stats = fs::read_to_string(candidate_dir.join("stats.csv"))?;

    let base_series = serde_json::to_string(&parse_stats_series(&base_stats))?;
    let cand_series = serde_json::to_string(&parse_stats_series(&cand_stats))?;

    let out_dir = baseline_dir.parent().unwrap_or_else(|| Path::new("."));
    fs::write(
        out_dir.join("compare.html"),
        render_compare_series(
            "Benchmark Compare",
            &base_series,
            &cand_series,
            "Coverage vs Time",
            "Corpus vs Time",
        ),
    )?;
    Ok(())
}

/// Compare two suite roots by averaging their run_* stats and plotting mean coverage/corpus.
fn write_compare_suite_html(baseline_root: &Path, candidate_root: &Path) -> Result<()> {
    let base_series =
        serde_json::to_string(&bucket_mean_series(&load_suite_samples(baseline_root)?))?;
    let cand_series =
        serde_json::to_string(&bucket_mean_series(&load_suite_samples(candidate_root)?))?;

    let out_dir = baseline_root.parent().unwrap_or_else(|| Path::new("."));
    fs::write(
        out_dir.join("compare.html"),
        render_compare_series(
            "Benchmark Suite Compare",
            &base_series,
            &cand_series,
            "Coverage vs Time (mean across runs)",
            "Corpus vs Time (mean across runs)",
        ),
    )?;
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

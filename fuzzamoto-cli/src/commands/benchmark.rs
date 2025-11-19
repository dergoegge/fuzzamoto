use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use clap::Subcommand;
use serde::Deserialize;

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

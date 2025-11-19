mod commands;
mod error;
mod utils;

use clap::{Parser, Subcommand};
use commands::benchmark;
use commands::*;
use error::Result;
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new fuzzamoto fuzzing campaign with Nyx
    Init {
        #[arg(long, help = "Path to the nyx share directory that should be created")]
        sharedir: PathBuf,
        #[arg(
            long,
            help = "Path to the crash handler that should be copied into the share directory"
        )]
        crash_handler: PathBuf,
        #[arg(
            long,
            help = "Path to the bitcoind binary that should be copied into the share directory"
        )]
        bitcoind: PathBuf,
        #[arg(
            long,
            help = "Path to the secondary bitcoind binary that should be copied into the share directory"
        )]
        secondary_bitcoind: Option<PathBuf>,
        #[arg(
            long,
            help = "Path to the fuzzamoto scenario binary that should be copied into the share directory"
        )]
        scenario: PathBuf,

        #[arg(long, help = "Path to the nyx installation")]
        nyx_dir: PathBuf,
    },

    /// Create a html coverage report for a given corpus
    Coverage {
        #[arg(long, help = "Path to the output directory for the coverage report")]
        output: PathBuf,
        #[arg(long, help = "Path to the input corpus directory")]
        corpus: PathBuf,
        #[arg(
            long,
            help = "Path to the bitcoind binary that should be copied into the share directory"
        )]
        bitcoind: PathBuf,
        #[arg(
            long,
            help = "Path to the fuzzamoto scenario binary that should be copied into the share directory"
        )]
        scenario: PathBuf,
    },

    /// Benchmark helper commands
    Benchmark {
        #[command(subcommand)]
        command: benchmark::BenchmarkCommands,
    },

    /// Fuzzamoto intermediate representation (IR) commands
    IR {
        #[command(subcommand)]
        command: ir::IRCommands,
    },
}

fn main() -> Result<()> {
    // Log info by default
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Init {
            sharedir,
            crash_handler,
            bitcoind,
            secondary_bitcoind,
            scenario,
            nyx_dir,
        } => InitCommand::execute(
            sharedir.clone(),
            crash_handler.clone(),
            bitcoind.clone(),
            secondary_bitcoind.clone(),
            scenario.clone(),
            nyx_dir.clone(),
        ),
        Commands::Coverage {
            output,
            corpus,
            bitcoind,
            scenario,
        } => CoverageCommand::execute(
            output.clone(),
            corpus.clone(),
            bitcoind.clone(),
            scenario.clone(),
        ),
        Commands::IR { command } => IrCommand::execute(command),
        Commands::Benchmark { command } => BenchmarkCommand::execute(command),
    }
}

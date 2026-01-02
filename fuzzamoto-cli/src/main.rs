mod commands;
mod error;
mod utils;

use clap::{Parser, Subcommand};
use commands::*;
use error::Result;
use std::path::PathBuf;

use crate::commands::coverage_batch::CoverageBatchCommand;

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
        #[arg(
            long,
            value_name = "PROFDATA",
            num_args = 1..,
            help = "Existing .profdata files to merge instead of generating fresh coverage"
        )]
        profraws: Option<Vec<PathBuf>>,
        #[arg(
            long,
            default_value_t = false,
            help = "Only execute the corpus inputs and write .profraw files; skip .profdata merge and HTML report generation"
        )]
        run_only: bool,
    },

    /// Create a html coverage report for a given corpus, runs using multiple docker instances
    CoverageBatch {
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
        #[arg(long, help = "The docker image id of fuzzamoto-coverage")]
        docker_image: String,
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
            profraws,
            run_only,
        } => CoverageCommand::execute(
            output.clone(),
            corpus.clone(),
            bitcoind.clone(),
            scenario.clone(),
            profraws.clone(),
            *run_only,
        ),
        Commands::CoverageBatch {
            output,
            corpus,
            bitcoind,
            scenario,
            docker_image,
        } => CoverageBatchCommand::execute(
            output.clone(),
            corpus.clone(),
            bitcoind.clone(),
            scenario.clone(),
            docker_image.clone(),
        ),
        Commands::IR { command } => IrCommand::execute(command),
    }
}

use crate::error::{CliError, Result};
use crate::utils::{
    file_ops::{self, ensure_dir_exists},
    process,
};
use std::path::PathBuf;

pub struct CoverageCommand;

impl CoverageCommand {
    pub fn execute(
        output: PathBuf,
        corpus: PathBuf,
        bitcoind: PathBuf,
        scenarios: Vec<Scenario>,
        profraws: Option<Vec<PathBuf>>,
        run_only: bool,
    ) -> Result<()> {
        file_ops::ensure_file_exists(&bitcoind)?;
        for scenario in &scenarios {
            file_ops::ensure_file_exists(scenario.path())?;
        }

        if run_only {
            let corpus_files = file_ops::read_dir_files(&corpus)?;
            for corpus_file in corpus_files {
                if let Err(e) = Self::run_one_input(&output, &corpus_file, &bitcoind, &scenarios) {
                    log::error!("Failed to run input ({:?}): {}", corpus_file, e);
                }
            }
            return Ok(());
        }

        let profdata = match profraws {
            Some(profraws) => Self::merge_profraws(&output, &profraws)?,
            None => {
                let corpus_files = file_ops::read_dir_files(&corpus)?;
                log::info!("{:?}", corpus_files);
                // Run scenario for each corpus file
                for corpus_file in corpus_files {
                    if let Err(e) =
                        Self::run_one_input(&output, &corpus_file, &bitcoind, &scenarios)
                    {
                        log::error!("Failed to run input ({:?}): {}", corpus_file, e);
                    }
                }

                let profraws_dir = vec![output.clone()];
                Self::merge_profraws(&output, &profraws_dir)?
            }
        };

        Self::generate_report(&output, &bitcoind, &profdata)?;
        Ok(())
    }

    fn run_one_input(
        output: &PathBuf,
        input: &PathBuf,
        bitcoind: &PathBuf,
        scenarios: &Vec<Scenario>,
    ) -> Result<()> {
        log::info!("Running scenario with input: {}", input.display());

        let profraw_file = output.join(format!(
            "{}.coverage.profraw.%p",
            input.file_name().unwrap().to_str().unwrap()
        ));

        let env_vars = vec![
            ("LLVM_PROFILE_FILE", profraw_file.to_str().unwrap()),
            ("FUZZAMOTO_INPUT", input.to_str().unwrap()),
            ("RUST_LOG", "debug"),
        ];
        for scenario in scenarios {
            process::run_scenario_command(scenario, bitcoind, &env_vars)?;
        }

        Ok(())
    }

    fn generate_report(
        output: &PathBuf,
        bitcoind: &PathBuf,
        coverage_profdata: &PathBuf,
    ) -> Result<()> {
        // Generate HTML report
        let coverage_report_dir = output.join("coverage-report");
        let coverage_report_str = coverage_report_dir.to_str().unwrap();
        let instr_profile_arg = format!("-instr-profile={}", coverage_profdata.to_str().unwrap());
        let output_dir_arg = format!("-output-dir={}", coverage_report_str);

        let show_args = vec![
            "show",
            bitcoind.to_str().unwrap(),
            &instr_profile_arg,
            "-format=html",
            "-show-directory-coverage",
            "-show-branches=count",
            &output_dir_arg,
            "-Xdemangler=c++filt",
        ];

        let show_cmd = process::get_llvm_command("llvm-cov");
        process::run_command_with_status(&show_cmd, &show_args, None)?;

        log::info!(
            "Coverage report generated in: {}",
            coverage_report_dir.display()
        );

        Ok(())
    }

    fn merge_profraws(output: &PathBuf, profraws: &Vec<PathBuf>) -> Result<PathBuf> {
        if profraws.is_empty() {
            return Err(CliError::InvalidInput(
                "No profraws directory provided".to_string(),
            ));
        }

        let mut profraw_files = Vec::new();

        for p in profraws {
            for entry in std::fs::read_dir(p)? {
                let path = entry?.path();
                if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                    if file_name.contains("coverage.profraw") {
                        profraw_files.push(path.to_str().unwrap().to_string());
                    }
                }
            }
        }

        if profraw_files.is_empty() {
            return Err(CliError::ProcessError("No profraw files found".to_string()));
        }

        let merged = output.join("coverage.profdata");
        let mut merge_args = vec!["merge", "-sparse"];

        let inputs: Vec<&str> = profraw_files.iter().map(|p| p.as_str()).collect();
        log::info!("Merging profdata from {:?}", inputs);

        merge_args.extend(inputs);
        merge_args.extend(["-o", merged.to_str().unwrap()]);
        let merge_cmd = process::get_llvm_command("llvm-profdata");
        process::run_command_with_status(&merge_cmd, &merge_args, None)?;

        Ok(merged)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ScenarioType {
    CompactBlocks,
    Generic,
    HttpServer,
    ImportMempool,
    IR,
    RPCGeneric,
    WalletMigration,
}

impl ScenarioType {
    pub fn from_filename(filename: &str) -> Option<Self> {
        match filename {
            "scenario-compact-blocks" => Some(Self::CompactBlocks),
            "scenario-generic" => Some(Self::Generic),
            "scenario-http-server" => Some(Self::HttpServer),
            "scenario-import-mempool" => Some(Self::ImportMempool),
            "scenario-ir" => Some(Self::IR),
            "scenario-rpc-generic" => Some(Self::RPCGeneric),
            "scenario-wallet-migration" => Some(Self::WalletMigration),
            _ => None,
        }
    }
}

pub struct Scenario {
    path: PathBuf,
    ty: ScenarioType,
}

impl Scenario {
    pub fn from_path(path: &PathBuf) -> Option<Self> {
        let filename = path.file_name()?.to_str()?;
        let ty = ScenarioType::from_filename(filename)?;
        Some(Self {
            path: path.to_path_buf(),
            ty,
        })
    }

    pub fn name(&self) -> &str {
        match self.ty {
            ScenarioType::CompactBlocks => "scenario-compact-blocks",
            ScenarioType::Generic => "scenario-generic",
            ScenarioType::HttpServer => "scenario-http-server",
            ScenarioType::ImportMempool => "scenario-import-mempool",
            ScenarioType::IR => "scenario-ir",
            ScenarioType::RPCGeneric => "scenario-rpc-generic",
            ScenarioType::WalletMigration => "scenario-wallet-migration",
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn ty(&self) -> ScenarioType {
        self.ty
    }
}

impl std::fmt::Display for Scenario {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl std::fmt::Debug for Scenario {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

pub fn scan_scenario_dir(dir: &PathBuf) -> Result<Vec<Scenario>> {
    ensure_dir_exists(dir)?;

    let mut scenarios = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if let Some(scenario) = Scenario::from_path(&path) {
            scenarios.push(scenario);
        }
    }

    log::info!(
        "Found {} scenarios in {:?}: {:?}",
        scenarios.len(),
        dir,
        scenarios
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    Ok(scenarios)
}

use crate::error::CliError;
use crate::error::Result;
use crate::utils::{file_ops, process};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
pub struct StabilityCommand;

impl StabilityCommand {
    fn read_lines<P: AsRef<Path>>(path: P) -> io::Result<Vec<String>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut lines = Vec::new();
        for line in reader.lines() {
            lines.push(line?);
        }

        Ok(lines)
    }

    pub fn execute(
        output: PathBuf,
        corpus: PathBuf,
        unstable_testcases: PathBuf,
        llvm_profparser: PathBuf,
        bitcoind: PathBuf,
        scenario: PathBuf,
    ) -> Result<()> {
        file_ops::ensure_file_exists(&bitcoind)?;
        file_ops::ensure_file_exists(&scenario)?;

        let corpus_files = file_ops::read_dir_files(&corpus)?;
        let unstable = Self::read_lines(unstable_testcases)?;

        let unstable_files: Vec<PathBuf> = corpus_files
            .iter()
            .filter(|f| unstable.contains(&f.file_name().unwrap().to_string_lossy().to_string()))
            .cloned()
            .collect();

        // Run scenario for each corpus file
        for corpus_file in unstable_files {
            if let Err(e) = Self::run_one_input(
                &output,
                &corpus_file,
                &llvm_profparser,
                &bitcoind,
                &scenario,
            ) {
                log::error!("Failed to run input ({:?}): {}", corpus_file, e);
            }
        }
        Self::generate_coverage_report(&output, &bitcoind)?;
        Ok(())
    }

    fn run_profparser(
        llvm_profparser: &PathBuf,
        output: &PathBuf,
        base: &PathBuf,
        other: &PathBuf,
    ) -> Result<()> {
        let _status = Command::new(llvm_profparser)
            .arg("merge")
            .arg("--output")
            .arg(output)
            .arg("--input")
            .arg(base)
            .arg("--input")
            .arg(other)
            .status()?;
        Ok(())
    }

    fn run_one_input(
        output: &PathBuf,
        input: &PathBuf,
        llvm_profparser: &PathBuf,
        bitcoind: &PathBuf,
        scenario: &PathBuf,
    ) -> Result<()> {
        log::info!("Running scenario with input: {}", input.display());

        let mut profraw_files: Vec<PathBuf> = Vec::new();
        for iter in 0..11 {
            let profraw_file = output.join(format!(
                "{}.coverage.profraw.{}",
                input.file_name().unwrap().to_str().unwrap(),
                iter
            ));
            profraw_files.push(profraw_file.clone());

            let env_vars = vec![
                ("LLVM_PROFILE_FILE", profraw_file.to_str().unwrap()),
                ("FUZZAMOTO_INPUT", input.to_str().unwrap()),
                ("RUST_LOG", "debug"),
            ];
            process::run_scenario_command(scenario, bitcoind, &env_vars)?;
        }

        let base = profraw_files.first().unwrap().clone();
        profraw_files.remove(0);
        for (iter, other) in profraw_files.iter().enumerate() {
            let summary = output.join(format!(
                "{}.{}.coverage.profraw.diff",
                input.file_name().unwrap().to_str().unwrap(),
                iter
            ));
            Self::run_profparser(llvm_profparser, &summary, &base, &other)?;
        }

        Ok(())
    }

    fn generate_coverage_report(output: &PathBuf, bitcoind: &PathBuf) -> Result<()> {
        // Find all profraw files
        let mut profraw_files = Vec::new();
        for entry in std::fs::read_dir(output)? {
            let path = entry?.path();
            if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                if file_name.contains("coverage.profraw.diff") {
                    profraw_files.push(path.to_str().unwrap().to_string());
                }
            }
        }

        if profraw_files.is_empty() {
            return Err(CliError::ProcessError("No profraw files found".to_string()));
        }

        // Merge profraw files
        let coverage_profdata = output.join("coverage.profdata");
        let coverage_profdata_str = coverage_profdata.to_str().unwrap();

        let mut merge_args = vec!["merge", "-sparse"];
        let profraw_refs: Vec<&str> = profraw_files.iter().map(|s| s.as_str()).collect();
        merge_args.extend(profraw_refs);
        merge_args.extend(["-o", coverage_profdata_str]);

        let merge_cmd = process::get_llvm_command("llvm-profdata");
        process::run_command_with_status(&merge_cmd, &merge_args, None)?;

        // Generate HTML report
        let coverage_report_dir = output.join("coverage-report");
        let coverage_report_str = coverage_report_dir.to_str().unwrap();
        let instr_profile_arg = format!("-instr-profile={}", coverage_profdata_str);
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
}

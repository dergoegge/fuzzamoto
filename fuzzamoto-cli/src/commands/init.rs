use crate::error::{CliError, Result};
use crate::utils::{file_ops, nyx, process};
use std::path::PathBuf;

pub struct InitCommand;

impl InitCommand {
    pub fn execute(
        sharedir: PathBuf,
        crash_handler: PathBuf,
        bitcoind: PathBuf,
        secondary_bitcoind: Option<PathBuf>,
        scenario: PathBuf,
        nyx_dir: Option<PathBuf>,
    ) -> Result<()> {
        file_ops::ensure_sharedir_not_exists(&sharedir)?;
        file_ops::create_dir_all(&sharedir)?;

        file_ops::ensure_file_exists(&crash_handler)?;
        file_ops::ensure_file_exists(&bitcoind)?;
        file_ops::ensure_file_exists(&scenario)?;

        if let Some(ref secondary) = secondary_bitcoind {
            file_ops::ensure_file_exists(secondary)?;
        }

        let mut all_deps = Vec::new();
        let mut binary_names = Vec::new();

        // Copy each binary and its dependencies
        let mut binaries = vec![bitcoind, scenario.clone()];
        if let Some(secondary) = secondary_bitcoind.clone() {
            binaries.push(secondary);
        }

        for binary in &binaries {
            let binary_name = binary
                .file_name()
                .ok_or_else(|| CliError::InvalidInput("Invalid binary path".to_string()))?
                .to_str()
                .ok_or_else(|| CliError::InvalidInput("Invalid binary name".to_string()))?;

            file_ops::copy_file_to_dir(binary, &sharedir)?;
            all_deps.push(binary_name.to_string());
            binary_names.push(binary_name.to_string());

            // Get and copy dependencies using lddtree
            let output =
                process::run_command_with_output("lddtree", &[binary.to_str().unwrap()], None)?;

            // Parse lddtree output and copy dependencies
            let deps = String::from_utf8_lossy(&output.stdout)
                .lines()
                .skip(1) // Skip first line
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split("=>").collect();
                    if parts.len() == 2 {
                        let name = parts[0].trim();
                        let path = parts[1].trim();

                        // Copy the dependency
                        if let Err(e) = std::fs::copy(path, sharedir.join(name)) {
                            log::warn!("Failed to copy {}: {}", name, e);
                        } else {
                            log::info!("Copied dependency of {}: {}", binary_name, name);
                        }

                        Some(name.to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<String>>();

            all_deps.extend(deps);
        }

        // Add crash handler to dependencies
        let crash_handler_name = crash_handler
            .file_name()
            .ok_or_else(|| CliError::InvalidInput("Invalid crash handler path".to_string()))?
            .to_str()
            .ok_or_else(|| CliError::InvalidInput("Invalid crash handler name".to_string()))?
            .to_string();

        file_ops::copy_file_to_dir(&crash_handler, &sharedir)?;
        all_deps.push(crash_handler_name.clone());
        all_deps.sort();
        all_deps.dedup();

        log::info!("Created share directory: {}", sharedir.display());

        let nyx_dir = match nyx_dir {
            Some(nyx_dir) => nyx_dir,
            // If nyx dir isn't specified, try to locate the libafl_nyx path
            None => nyx::get_libafl_nyx_path()?,
        };
        nyx::compile_packer_binaries(&nyx_dir)?;
        nyx::copy_packer_binaries(&nyx_dir, &sharedir)?;
        nyx::generate_nyx_config(&nyx_dir, &sharedir)?;

        // Create fuzz_no_pt.sh script
        let scenario_name = scenario
            .file_name()
            .ok_or_else(|| CliError::InvalidInput("Invalid scenario path".to_string()))?
            .to_str()
            .ok_or_else(|| CliError::InvalidInput("Invalid scenario name".to_string()))?;

        let secondary_name = secondary_bitcoind
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|name| name.to_str());

        nyx::create_nyx_script(
            &sharedir,
            &all_deps,
            &binary_names,
            &crash_handler_name,
            scenario_name,
            secondary_name,
        )?;

        Ok(())
    }
}

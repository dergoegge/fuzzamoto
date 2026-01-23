use crate::error::{CliError, Result};
use std::path::Path;
use std::process::{Command, Stdio};

pub fn get_llvm_command(base: &str) -> String {
    match std::env::var("LLVM_V") {
        Ok(version) => format!("{}-{}", base, version),
        Err(_) => base.to_string(),
    }
}

pub fn run_command_with_status(command: &str, args: &[&str], cwd: Option<&Path>) -> Result<()> {
    let mut cmd = Command::new(command);
    cmd.args(args);

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let status = cmd.status()?;

    if status.success() {
        Ok(())
    } else {
        Err(CliError::ProcessError(format!(
            "Command '{}' failed with exit code: {}",
            command,
            status.code().unwrap_or(-1)
        )))
    }
}

pub fn run_command_with_output(
    command: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<std::process::Output> {
    let mut cmd = Command::new(command);
    cmd.args(args);

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let output = cmd.output()?;

    if output.status.success() {
        Ok(output)
    } else {
        Err(CliError::ProcessError(format!(
            "Command '{}' failed: {}",
            command,
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

pub fn run_scenario_command(
    scenario: &Path,
    bitcoind: &Path,
    env_vars: &[(&str, &str)],
) -> Result<()> {
    let mut cmd = Command::new(scenario);
    cmd.arg(bitcoind);

    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    cmd.stdout(Stdio::null()).stderr(Stdio::null());

    let status = cmd.status()?;

    if status.success() {
        Ok(())
    } else {
        Err(CliError::ProcessError("Scenario failed to run".to_string()))
    }
}

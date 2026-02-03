use crate::error::{CliError, Result};
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::process::Command;
pub struct BugReproCommand;

impl BugReproCommand {
    pub fn execute(yaml: &PathBuf) -> Result<()> {
        if !yaml.is_file() {
            return Err(CliError::InvalidInput(format!(
                "{} doesn't exist",
                yaml.display()
            )));
        }

        let file = File::open(yaml)?;
        let parsed: HashMap<String, String> = serde_yaml_ng::from_reader(file)
            .map_err(|_| CliError::InvalidInput("Failed to parse yaml file".to_string()))?;

        let commit = parsed
            .get("commit")
            .ok_or_else(|| CliError::InvalidInput("Failed to parse yaml file".to_string()))?;
        let name = parsed
            .get("name")
            .ok_or_else(|| CliError::InvalidInput("Failed to parse yaml file".to_string()))?;

        let differential = parsed
            .get("differential")
            .ok_or_else(|| CliError::InvalidInput("Failed to parse yaml file".to_string()))?;
        let differential = if differential == "1" { 1 } else { 0 };

        let tag = format!("fuzzamoto-libafl-{}", name);

        println!("Building {}", tag);
        let _ = Command::new("docker")
            .args([
                "build",
                "-f",
                "Dockerfile.bug",
                "-t",
                &tag,
                ".",
                "--build-arg",
                &format!("BITCOIN_COMMIT={}", commit),
                "--build-arg",
                &format!("BUG_PATCH={}", name),
                "--build-arg",
                &format!("DIFFERENTIAL={}", differential),
            ])
            .status()
            .map_err(|_| CliError::InvalidInput("Failed to run docer build".to_string()))?;

        Ok(())
    }
}

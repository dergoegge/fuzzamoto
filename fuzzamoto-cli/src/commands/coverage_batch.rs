use crate::error::{CliError, Result};
use crate::utils::file_ops;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
pub struct CoverageBatchCommand;

impl CoverageBatchCommand {
    pub fn execute(
        output: PathBuf,
        corpus: PathBuf,
        _bitcoind: PathBuf,
        _scenario: PathBuf,
        docker_image: String,
    ) -> Result<()> {
        let found = Self::check_local_docker_image(&docker_image)?;
        if !found {
            return Err(CliError::InvalidInput(
                "fuzzamoto-coverage-generic docker image not found!".to_string(),
            ));
        }
        let cpu_n = std::thread::available_parallelism()
            .map_err(|e| CliError::ProcessError(format!("Failed to get CPU parallelism: {e}")))?
            .get();
        let all_files = file_ops::read_dir_files(&corpus)?;

        // we can't have containers that do nothing
        let cpu_n = std::cmp::min(cpu_n, all_files.len());

        log::info!("Spawning {} containers", cpu_n);
        let batches = Self::split_corpus(all_files, cpu_n);
        if output.exists() {
            log::warn!(
                "Output directory {:?} already exists. Overwriting it..",
                output
            );
            fs::remove_dir_all(&output).map_err(|_| {
                CliError::InvalidInput("Failed to remove existing output directory".to_string())
            })?;
        }

        fs::create_dir(&output)
            .map_err(|_| CliError::InvalidInput("Failed to create output directory".to_string()))?;

        let mut workdirs: Vec<PathBuf> = Vec::with_capacity(cpu_n);
        for (i, batch_files) in batches.iter().enumerate() {
            let workdir = output.join(format!("workdir_{i}"));
            let w_corpus = workdir.join("corpus");
            let w_out = workdir.join("output");

            fs::create_dir_all(&w_corpus).map_err(|e| {
                CliError::InvalidInput(format!(
                    "Failed to create workdir {}: {e}",
                    w_corpus.display()
                ))
            })?;
            fs::create_dir_all(&w_out).map_err(|e| {
                CliError::InvalidInput(format!("Failed to create workdir {}: {e}", w_out.display()))
            })?;

            for src in batch_files {
                let file_name = src.file_name().ok_or_else(|| {
                    CliError::InvalidInput(format!("Invalid corpus file path: {}", src.display()))
                })?;
                let dst = w_corpus.join(file_name);
                fs::copy(src, &dst).map_err(|_| {
                    CliError::InvalidInput(format!("Failed to copy corpus file {}", src.display()))
                })?;
            }

            workdirs.push(workdir);
        }

        let mut containers: Vec<(usize, String)> = Vec::with_capacity(workdirs.len());
        for (i, workdir) in workdirs.iter().enumerate() {
            let cid = Self::docker_run_detached(
                &docker_image,
                &workdir.join("corpus"),
                &workdir.join("output"),
            )?;
            log::info!("Started batch {} as container {}", i, cid);
            containers.push((i, cid));
        }

        for (i, cid) in &containers {
            let code = Self::docker_wait(cid)?;
            if code != 0 {
                let logs = Command::new("docker").args(["logs", cid]).output().ok();
                if let Some(logs) = logs {
                    log::error!(
                        "Container {} (batch {}) logs:\n{}",
                        cid,
                        i,
                        String::from_utf8_lossy(&logs.stdout)
                    );
                    let stderr = String::from_utf8_lossy(&logs.stderr);
                    if !stderr.trim().is_empty() {
                        log::error!("Container {} (batch {}) stderr:\n{}", cid, i, stderr);
                    }
                }

                return Err(CliError::ProcessError(format!(
                    "Batch {} failed: container {} exited with code {}",
                    i, cid, code
                )));
            }
            log::info!("Batch {} finished successfully (container {})", i, cid);
        }

        for (_, cid) in &containers {
            log::info!("Cleaning up container {}", cid);
            let _ = Command::new("docker").args(["rm", "-f", cid]).status();
        }

        let mut profraws_hosts = Vec::new();
        for workdir in &workdirs {
            let profraws_dir = workdir.join("output");
            profraws_hosts.push(profraws_dir);
        }

        let final_output = output.join("final");
        fs::create_dir(&final_output).map_err(|_| {
            CliError::InvalidInput("Failed to create final output directory".to_string())
        })?;

        let merging_container =
            Self::docker_merge_detached(&docker_image, &corpus, &output, &profraws_hosts)?;
        log::info!("Started merge container {}", merging_container);
        Ok(())
    }

    fn docker_wait(container_id: &str) -> Result<i32> {
        let out = Command::new("docker")
            .args(["wait", container_id])
            .output()
            .map_err(|e| CliError::ProcessError(format!("docker wait failed: {e}")))?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(CliError::ProcessError(format!(
                "docker wait failed for {container_id}: {stderr}"
            )));
        }

        let code = String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<i32>()
            .map_err(|e| CliError::ProcessError(format!("Bad exit code from docker wait: {e}")))?;

        Ok(code)
    }

    fn docker_run_detached(image: &str, corpus_host: &Path, output_host: &Path) -> Result<String> {
        let corpus_abs = fs::canonicalize(corpus_host).map_err(|e| {
            CliError::InvalidInput(format!(
                "Failed to canonicalize {}: {e}",
                corpus_host.display()
            ))
        })?;
        let output_abs = fs::canonicalize(output_host).map_err(|e| {
            CliError::InvalidInput(format!(
                "Failed to canonicalize {}: {e}",
                output_host.display()
            ))
        })?;

        let out = Command::new("docker")
            .args([
                "run",
                "--privileged",
                "-d",
                "-v",
                &format!("{}:/mnt/corpus", corpus_abs.display()),
                "-v",
                &format!("{}:/mnt/output", output_abs.display()),
                image,
                "/fuzzamoto/target/release/fuzzamoto-cli",
                "coverage",
                "--output",
                "/mnt/output",
                "--corpus",
                "/mnt/corpus",
                "--bitcoind",
                "/bitcoin/build_fuzz_cov/bin/bitcoind",
                "--scenario",
                "/fuzzamoto/target/release/scenario-ir",
                "--run-only",
            ])
            .output()
            .map_err(|e| CliError::ProcessError(format!("docker run failed: {e}")))?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(CliError::ProcessError(format!(
                "docker run failed: {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn docker_merge_detached(
        image: &str,
        corpus_host: &Path,
        output_host: &Path,
        profraws_hosts: &[PathBuf],
    ) -> Result<String> {
        let corpus_abs = fs::canonicalize(corpus_host).map_err(|e| {
            CliError::InvalidInput(format!(
                "Failed to canonicalize {}: {e}",
                corpus_host.display()
            ))
        })?;

        let output_abs = fs::canonicalize(output_host).map_err(|e| {
            CliError::InvalidInput(format!(
                "Failed to canonicalize {}: {e}",
                output_host.display()
            ))
        })?;

        let mut profdatas_container: Vec<String> = Vec::with_capacity(profraws_hosts.len());
        for p in profraws_hosts {
            let p_abs = fs::canonicalize(p).map_err(|e| {
                CliError::InvalidInput(format!("Failed to canonicalize {}: {e}", p.display()))
            })?;

            let rel = p_abs.strip_prefix(&output_abs).map_err(|_| {
                CliError::InvalidInput(format!(
                    "Profdata path is not under output root. profdata={}, outroot={}",
                    p_abs.display(),
                    output_abs.display()
                ))
            })?;

            profdatas_container.push(format!("/mnt/output/{}", rel.to_str().unwrap()));
        }

        let mut args: Vec<String> = Vec::new();
        args.extend([
            "run".into(),
            "--privileged".into(),
            "-i".into(),
            "--rm".into(),
            "-v".into(),
            format!("{}:/mnt/corpus", corpus_abs.display()),
            "-v".into(),
            format!("{}:/mnt/output", output_abs.display()),
            image.into(),
            "/fuzzamoto/target/release/fuzzamoto-cli".into(),
            "coverage".into(),
            "--output".into(),
            "/mnt/output/final".into(),
            "--corpus".into(),
            "/mnt/corpus".into(),
            "--bitcoind".into(),
            "/bitcoin/build_fuzz_cov/bin/bitcoind".into(),
            "--scenario".into(),
            "/fuzzamoto/target/release/scenario-ir".into(),
            "--profraws".into(),
        ]);

        args.extend(profdatas_container);

        let out = Command::new("docker")
            .args(args)
            .output()
            .map_err(|e| CliError::ProcessError(format!("docker run (merge) failed: {e}")))?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(CliError::ProcessError(format!(
                "docker run (merge) failed: {stderr}"
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn split_corpus(files: Vec<PathBuf>, n: usize) -> Vec<Vec<PathBuf>> {
        let mut batches = vec![Vec::new(); n];
        for (idx, p) in files.into_iter().enumerate() {
            batches[idx % n].push(p);
        }
        batches
    }

    pub fn check_local_docker_image(image_id: &str) -> Result<bool> {
        let output = Command::new("docker")
            .args(["image", "inspect", image_id])
            .output()
            .map_err(|_| CliError::ProcessError("Failed to run docker image".to_string()))?;
        if !output.status.success() {
            return Err(CliError::ProcessError(
                "Failed to run docker image".to_string(),
            ));
        }

        let images: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| CliError::ProcessError("Failed to run docker image".to_string()))?;

        let images = images.as_array().ok_or(CliError::ProcessError(
            "Failed top parse Json result".to_string(),
        ))?;

        for image in images {
            if let Some(repo_tags) = image.get("RepoTags").and_then(|v| v.as_array()) {
                for tag in repo_tags {
                    if let Some(tag) = tag.as_str() {
                        // tag format: name:version
                        if let Some((name, _)) = tag.split_once(':') {
                            if name == "fuzzamoto-coverage-generic" {
                                return Ok(true);
                            }
                        }
                    }
                }
            }
        }

        Ok(false)
    }
}

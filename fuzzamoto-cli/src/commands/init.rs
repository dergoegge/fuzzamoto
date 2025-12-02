use crate::error::{CliError, Result};
use crate::utils::{file_ops, nyx, process};
use std::path::PathBuf;

pub struct InitCommand;

impl InitCommand {
    pub fn execute(sharedir: PathBuf, image: String, nyx_dir: Option<PathBuf>) -> Result<()> {
        file_ops::ensure_sharedir_not_exists(&sharedir)?;
        file_ops::create_dir_all(&sharedir)?;

        // Check if the Docker image exists locally
        log::info!("Checking if Docker image exists locally: {}", image);
        let image_exists =
            process::run_command_with_status("docker", &["image", "inspect", &image], None).is_ok();

        if image_exists {
            log::info!("Docker image already exists locally, skipping pull");
        } else {
            // Pull the Docker image
            log::info!("Pulling Docker image: {}", image);
            process::run_command_with_status("docker", &["pull", &image], None)?;
        }

        // Create a container from the image with a name
        let container_name = "fuzzamoto-temp-container";
        log::info!("Creating container from image: {}", image);
        process::run_command_with_status(
            "docker",
            &["create", "--name", container_name, &image],
            None,
        )?;

        // Export the container to a tar file
        let container_tar_path = sharedir.join("container.tar");
        log::info!("Exporting container to: {}", container_tar_path.display());
        process::run_command_with_status(
            "docker",
            &[
                "export",
                container_name,
                "-o",
                container_tar_path.to_str().unwrap(),
            ],
            None,
        )?;

        // Clean up: remove the container
        log::info!("Removing temporary container: {}", container_name);
        process::run_command_with_status("docker", &["rm", container_name], None)?;

        let nyx_dir = match nyx_dir {
            Some(nyx_dir) => nyx_dir,
            // If nyx dir isn't specified, try to locate the libafl_nyx path
            None => nyx::get_libafl_nyx_path()?,
        };
        nyx::compile_packer_binaries(&nyx_dir)?;
        nyx::copy_packer_binaries(&nyx_dir, &sharedir)?;
        nyx::generate_nyx_config(&nyx_dir, &sharedir)?;

        nyx::create_nyx_script(&sharedir)?;

        Ok(())
    }
}

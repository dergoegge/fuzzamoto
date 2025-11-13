use crate::error::{CliError, Result};
use crate::utils::process::run_command_with_status;
use std::path::{Path, PathBuf};

pub fn get_libafl_nyx_path() -> Result<PathBuf> {
    let output = std::process::Command::new("cargo")
        .arg("metadata")
        .arg("--format-version=1")
        .output()?;

    if !output.status.success() {
        return Err(CliError::ProcessError(
            "Failed to get cargo metadata".to_string(),
        ));
    }

    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    let packages = metadata
        .as_object()
        .and_then(|obj| obj.get("packages"))
        .and_then(|p| p.as_array())
        .ok_or_else(|| CliError::ProcessError("Invalid cargo metadata format".to_string()))?;

    let libafl_nyx_package = packages
        .iter()
        .find(|p| {
            p.as_object()
                .and_then(|obj| obj.get("name"))
                .and_then(|name| name.as_str())
                == Some("libafl_nyx")
        })
        .ok_or_else(|| CliError::ProcessError("libafl_nyx package not found".to_string()))?;

    let manifest_path = libafl_nyx_package
        .as_object()
        .and_then(|obj| obj.get("manifest_path"))
        .and_then(|path| path.as_str())
        .ok_or_else(|| CliError::ProcessError("Invalid manifest path".to_string()))?;

    let libafl_nyx_path = PathBuf::from(manifest_path)
        .parent()
        .ok_or_else(|| CliError::ProcessError("Invalid libafl_nyx path".to_string()))?
        .to_path_buf();

    log::info!("Found libafl_nyx at: {:?}", libafl_nyx_path);
    Ok(libafl_nyx_path)
}

pub fn compile_packer_binaries(nyx_path: &Path) -> Result<()> {
    log::info!("Compiling packer binaries");

    let packer_path = nyx_path.join("packer/packer/");
    let userspace_path = packer_path.join("linux_x86_64-userspace");

    run_command_with_status("bash", &["compile_64.sh"], Some(&userspace_path))?;

    Ok(())
}

pub fn copy_packer_binaries(nyx_path: &Path, dst_dir: &Path) -> Result<()> {
    let packer_path = nyx_path.join("packer/packer/");
    let userspace_path = packer_path.join("linux_x86_64-userspace");
    let binaries_path = userspace_path.join("bin64");

    crate::utils::file_ops::copy_dir_contents(&binaries_path, dst_dir)?;

    Ok(())
}

pub fn generate_nyx_config(nyx_path: &Path, sharedir: &Path) -> Result<()> {
    log::info!("Generating nyx config");

    let packer_path = nyx_path.join("packer/packer/");

    run_command_with_status(
        "python3",
        &[
            "nyx_config_gen.py",
            sharedir.to_str().unwrap(),
            "Kernel",
            "-m",
            "4096",
        ],
        Some(&packer_path),
    )?;

    Ok(())
}

pub fn create_nyx_script(sharedir: &Path) -> Result<()> {
    let mut script = Vec::new();

    script.push("chmod +x hget".to_string());
    script.push("cp hget /tmp".to_string());
    script.push("cd /tmp".to_string());
    script.push("echo 0 > /proc/sys/kernel/randomize_va_space".to_string());
    script.push("echo 0 > /proc/sys/kernel/printk".to_string());
    script.push("./hget hcat_no_pt hcat".to_string());
    script.push("./hget habort_no_pt habort".to_string());
    script.push("chmod +x ./hcat".to_string());
    script.push("chmod +x ./habort".to_string());

    script.push("./hget container.tar container.tar".to_string());

    script.push("export __AFL_DEFER_FORKSRV=1".to_string()); // TODO why is this needed again?

    // Enable networking through localhost
    script.push("ip addr add 127.0.0.1/8 dev lo".to_string());
    script.push("ip link set lo up".to_string());
    script.push("ip a | ./hcat".to_string()); // Maybe useful for debugging

    // Unpack the container
    script.push("mkdir rootfs/ && tar -xf container.tar -C /tmp/rootfs".to_string());
    script.push("mount -t proc /proc rootfs/proc/".to_string());
    script.push("mount --rbind /sys rootfs/sys/".to_string());
    script.push("mount --rbind /dev rootfs/dev/".to_string());
    // Launch the init script (init.sh is expected to exist)
    script.push("chroot /tmp/rootfs /init.sh".to_string());
    script.push("cat rootfs/init.log | ./hcat".to_string());

    script.push("./habort \"$(tail rootfs/init.log)\"".to_string());

    let script_path = sharedir.join("fuzz_no_pt.sh");
    let script_content = script.join("\n");
    std::fs::write(&script_path, script_content)?;

    log::info!("Created fuzz_no_pt.sh script");
    Ok(())
}

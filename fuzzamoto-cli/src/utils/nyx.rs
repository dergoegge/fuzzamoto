use crate::error::Result;
use crate::utils::process::run_command_with_status;
use std::path::Path;

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

pub fn create_nyx_script(
    sharedir: &Path,
    all_deps: &[String],
    binary_names: &[String],
    crash_handler_name: &str,
    scenario_name: &str,
    secondary_bitcoind: Option<&str>,
    rpc_path: Option<&str>,
) -> Result<()> {
    let mut script = Vec::new();

    script.push("chmod +x hget".to_string());
    script.push("cp hget /tmp".to_string());
    script.push("cd /tmp".to_string());
    script.push("echo 0 > /proc/sys/kernel/randomize_va_space".to_string());
    script.push("echo 0 > /proc/sys/kernel/printk".to_string());
    script.push("./hget hcat_no_pt hcat".to_string());
    script.push("./hget habort_no_pt habort".to_string());

    // Add dependencies
    for dep in all_deps {
        script.push(format!("./hget {} {}", dep, dep));
    }

    if let Some(rpc_path) = rpc_path {
        script.push(format!("./hget {} {}", rpc_path, rpc_path));
    }

    // Make executables
    for exe in &["habort", "hcat", "ld-linux-x86-64.so.2", crash_handler_name] {
        script.push(format!("chmod +x {}", exe));
    }

    for binary_name in binary_names {
        script.push(format!("chmod +x {}", binary_name));
    }

    script.push("export __AFL_DEFER_FORKSRV=1".to_string());

    // Network setup
    script.push("ip addr add 127.0.0.1/8 dev lo".to_string());
    script.push("ip link set lo up".to_string());
    script.push("ip a | ./hcat".to_string());

    // Create bitcoind proxy script
    let asan_options = [
        "detect_leaks=1",
        "detect_stack_use_after_return=1",
        "check_initialization_order=1",
        "strict_init_order=1",
        "log_path=/tmp/asan.log",
        "abort_on_error=1",
        "handle_abort=1",
    ]
    .join(":");

    #[cfg(feature = "nyx_log")]
    let primary_log = " > /tmp/primary.log 2>&1";
    #[cfg(not(feature = "nyx_log"))]
    let primary_log = "";

    #[cfg(feature = "nyx_log")]
    let secondary_log = " > /tmp/secondary.log 2>&1";
    #[cfg(not(feature = "nyx_log"))]
    let secondary_log = "";

    let asan_options = format!("ASAN_OPTIONS={}", asan_options);
    let crash_handler_preload = format!("LD_PRELOAD=./{}", crash_handler_name);
    let proxy_script = format!(
        "{} LD_LIBRARY_PATH=/tmp LD_BIND_NOW=1 {} ./bitcoind \\$@{}",
        asan_options, crash_handler_preload, primary_log
    );

    script.push("echo \"#!/bin/sh\" > ./bitcoind_proxy".to_string());
    script.push(format!("echo \"{}\" >> ./bitcoind_proxy", proxy_script));
    script.push("chmod +x ./bitcoind_proxy".to_string());

    if secondary_bitcoind.is_some() {
        let secondary_proxy_script = format!(
            "{} LD_LIBRARY_PATH=/tmp LD_BIND_NOW=1 {} ./{} \\$@{}",
            asan_options,
            crash_handler_preload,
            secondary_bitcoind.unwrap(),
            secondary_log
        );
        script.push("echo \"#!/bin/sh\" > ./bitcoind2_proxy".to_string());
        script.push(format!(
            "echo \"{}\" >> ./bitcoind2_proxy",
            secondary_proxy_script
        ));
        script.push("chmod +x ./bitcoind2_proxy".to_string());
    }

    // Run the scenario
    script.push(format!(
        "RUST_LOG=debug LD_LIBRARY_PATH=/tmp LD_BIND_NOW=1 ./{} ./bitcoind_proxy {} ./bitcoind2_proxy > log.txt 2>&1",
        scenario_name,
        rpc_path.unwrap_or(""),
    ));

    // Debug info
    script.push("cat log.txt | ./hcat".to_string());
    script.push(
        "./habort \"target has terminated without initializing the fuzzing agent ...\"".to_string(),
    );

    let script_path = sharedir.join("fuzz_no_pt.sh");
    let script_content = script.join("\n");
    std::fs::write(&script_path, script_content)?;

    log::info!("Created fuzz_no_pt.sh script");
    Ok(())
}

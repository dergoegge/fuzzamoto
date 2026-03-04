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

pub struct NyxScriptOptions<'a> {
    pub secondary_bitcoind: Option<&'a str>,
    pub rpc_path: Option<&'a str>,
    pub seedfile: Option<&'a str>,
}

pub fn create_nyx_script(
    sharedir: &Path,
    all_deps: &[String],
    binary_names: &[String],
    crash_handler_name: &str,
    scenario_name: &str,
    opts: &NyxScriptOptions<'_>,
) -> Result<()> {
    let secondary_bitcoind = opts.secondary_bitcoind;
    let rpc_path = opts.rpc_path;
    let seedfile = opts.seedfile;
    let mut script = vec![
        "chmod +x hget".to_string(),
        "cp hget /tmp".to_string(),
        "cd /tmp".to_string(),
        "echo 0 > /proc/sys/kernel/randomize_va_space".to_string(),
        "echo 0 > /proc/sys/kernel/printk".to_string(),
        "./hget hcat_no_pt hcat".to_string(),
        "./hget habort_no_pt habort".to_string(),
    ];

    // Add dependencies
    for dep in all_deps {
        script.push(format!("./hget {dep} {dep}"));
    }

    if let Some(rpc_path) = rpc_path {
        script.push(format!("./hget {rpc_path} {rpc_path}"));
    }

    if let Some(seedfile) = seedfile {
        script.push(format!("./hget {seedfile} {seedfile}"));
    }

    // Make executables
    for exe in &["habort", "hcat", "ld-linux-x86-64.so.2", crash_handler_name] {
        script.push(format!("chmod +x {exe}"));
    }

    for binary_name in binary_names {
        script.push(format!("chmod +x {binary_name}"));
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

    let asan_options = format!("ASAN_OPTIONS={asan_options}");
    let crash_handler_preload = format!("LD_PRELOAD=./{crash_handler_name}");
    let proxy_script = format!(
        "{asan_options} LD_LIBRARY_PATH=/tmp LD_BIND_NOW=1 {crash_handler_preload} ./bitcoind \\$@",
    );

    script.push("echo \"#!/bin/sh\" > ./bitcoind_proxy".to_string());
    script.push(format!("echo \"{proxy_script}\" >> ./bitcoind_proxy"));
    script.push("chmod +x ./bitcoind_proxy".to_string());

    // Build the seedfile argument for the scenario command
    let seedfile_arg = seedfile
        .map(|s| format!(" --seedfile {s}"))
        .unwrap_or_default();

    // Run the scenario
    script.push(format!(
        "RUST_LOG=debug LD_LIBRARY_PATH=/tmp LD_BIND_NOW=1 ./{} ./bitcoind_proxy {} ./{}{} > log.txt 2>&1",
        scenario_name,
        rpc_path.unwrap_or(""),
        secondary_bitcoind.unwrap_or(""),
        seedfile_arg
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

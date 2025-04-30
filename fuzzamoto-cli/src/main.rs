use clap::{Parser, Subcommand, ValueEnum};
use fuzzamoto_ir::compiler::Compiler;
use fuzzamoto_ir::{
    AdvanceTimeGenerator, Generator, InstructionContext, Program, ProgramBuilder, ProgramContext,
    SendMessageGenerator, TxGenerator, TxoGenerator,
};
use log;
use rand::Rng;
use rand::rngs::ThreadRng;
use rand::seq::SliceRandom;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new fuzzamoto fuzzing campaign with Nyx
    Init {
        #[arg(long, help = "Path to the nyx share directory that should be created")]
        sharedir: PathBuf,
        #[arg(
            long,
            help = "Path to the crash handler that should be copied into the share directory"
        )]
        crash_handler: PathBuf,
        #[arg(
            long,
            help = "Path to the bitcoind binary that should be copied into the share directory"
        )]
        bitcoind: PathBuf,
        #[arg(
            long,
            help = "Path to the fuzzamoto scenario binary that should be copied into the share directory"
        )]
        scenario: PathBuf,
    },

    /// Create a html coverage report for a given corpus
    Coverage {
        #[arg(long, help = "Path to the output directory for the coverage report")]
        output: PathBuf,
        #[arg(long, help = "Path to the input corpus directory")]
        corpus: PathBuf,
        #[arg(
            long,
            help = "Path to the bitcoind binary that should be copied into the share directory"
        )]
        bitcoind: PathBuf,
        #[arg(
            long,
            help = "Path to the fuzzamoto scenario binary that should be copied into the share directory"
        )]
        scenario: PathBuf,
    },

    /// Record test cases from a corpus of a specialized scenario to be used as seeds for the
    /// generic scenario
    Record {
        #[arg(
            long,
            help = "Path to the output directory for the recorded test cases"
        )]
        output: PathBuf,
        #[arg(long, help = "Path to the input corpus directory")]
        corpus: PathBuf,
        #[arg(long, help = "Path to the fuzzamoto scenario scenario binary")]
        scenario: PathBuf,
    },
    /// Fuzzamoto intermediate representation (IR) commands
    IR {
        #[command(subcommand)]
        command: IRCommands,
    },
}

#[derive(Subcommand)]
enum IRCommands {
    /// Generate fuzzamoto IR
    Generate {
        #[arg(long, help = "Path to the output directory for the generated IR")]
        output: PathBuf,
        #[arg(
            long,
            help = "Max number of iterations to generate each output IR program for"
        )]
        iterations: usize,
        #[arg(long, help = "Number of IR programs to generate")]
        programs: usize,
        #[arg(long, help = "Path to the program context file")]
        context: PathBuf,
        #[arg(long, help = "Path to the program context file")]
        txos: Option<PathBuf>,
    },
    /// Compile fuzzamoto IR
    Compile {
        #[arg(long, help = "Path to the input file/directory for the generated IR")]
        input: PathBuf,
        #[arg(long, help = "Path to the output file/directory for the compiled IR")]
        output: PathBuf,
    },

    /// Convert fuzzamoto corpora
    Convert {
        #[arg(long, help = "Format of the input IR", value_enum, default_value_t = CorpusFormat::Postcard)]
        from: CorpusFormat,
        #[arg(long, help = "Format of the output IR", value_enum, default_value_t = CorpusFormat::Json)]
        to: CorpusFormat,
        #[arg(long, help = "Path to the input file/directory for the generated IR")]
        input: PathBuf,
        #[arg(long, help = "Path to the output file/directory for the converted IR")]
        output: PathBuf,
    },

    /// Print human readable IR
    Print {
        #[arg(long, help = "Print IR in json format", default_value_t = false)]
        json: bool,

        #[arg(help = "Path to the input IR file ot be displayed")]
        input: PathBuf,
    },
}

#[derive(ValueEnum, Debug, Clone)]
enum CorpusFormat {
    Json,
    Postcard, // Default corpus format (https://github.com/jamesmunns/postcard)
}

fn create_sharedir(
    sharedir: PathBuf,
    crash_handler: PathBuf,
    bitcoind: PathBuf,
    scenario: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    if sharedir.exists() {
        return Err("Share directory already exists".into());
    }
    std::fs::create_dir_all(&sharedir)?;

    if !crash_handler.exists() {
        return Err("Crash handler does not exist".into());
    }

    let mut all_deps = Vec::new();
    let mut binary_names = Vec::new();

    // Copy each binary and its dependencies
    let binaries = vec![bitcoind, scenario.clone()];
    for binary in &binaries {
        // Copy the binary itself
        let binary_name = binary
            .file_name()
            .ok_or("Invalid binary path")?
            .to_str()
            .ok_or("Invalid binary name")?;
        std::fs::copy(binary, sharedir.join(binary_name))?;
        log::info!("Copied binary: {}", binary_name);
        all_deps.push(binary_name.to_string());
        binary_names.push(binary_name.to_string());
        // Get and copy dependencies using lddtree
        let output = std::process::Command::new("lddtree").arg(binary).output()?;

        if !output.status.success() {
            return Err(
                format!("lddtree error: {}", String::from_utf8_lossy(&output.stderr)).into(),
            );
        }

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
        .ok_or("Invalid crash handler path")?
        .to_str()
        .ok_or("Invalid crash handler name")?
        .to_string();
    std::fs::copy(&crash_handler, sharedir.join(&crash_handler_name))?;
    log::info!("Copied crash handler: {}", crash_handler_name);
    all_deps.push(crash_handler_name.clone());
    all_deps.sort();
    all_deps.dedup();

    // Create fuzz_no_pt.sh script
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

    // Make executables
    for exe in &[
        "habort",
        "hcat",
        "ld-linux-x86-64.so.2",
        &crash_handler_name,
    ] {
        script.push(format!("chmod +x {}\n", exe));
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

    let asan_options = format!("ASAN_OPTIONS={}", asan_options);
    let crash_handler_preload = format!("LD_PRELOAD=./{}", crash_handler_name);
    let proxy_script = format!(
        "{} LD_LIBRARY_PATH=/tmp LD_BIND_NOW=1 {} ./bitcoind \\$@",
        asan_options, crash_handler_preload,
    );

    script.push(format!("echo \"#!/bin/sh\" > ./bitcoind_proxy"));
    script.push(format!("echo \"{}\" >> ./bitcoind_proxy", proxy_script));
    script.push("chmod +x ./bitcoind_proxy".to_string());

    // Run the scenario
    let scenario_name = scenario
        .file_name()
        .ok_or("Invalid scenario path")?
        .to_str()
        .ok_or("Invalid scenario name")?;
    script.push(format!(
        "RUST_LOG=debug LD_LIBRARY_PATH=/tmp LD_BIND_NOW=1 ./{} ./bitcoind_proxy > log.txt 2>&1",
        scenario_name
    ));

    // Debug info
    script.push("cat log.txt | ./hcat".to_string());
    script.push(
        "./habort \"target has terminated without initializing the fuzzing agent ...\"".to_string(),
    );

    let mut file = File::create(sharedir.join("fuzz_no_pt.sh"))?;
    for line in script {
        writeln!(file, "{}", line)?;
    }

    log::info!("Created share directory: {}", sharedir.display());

    Ok(())
}

fn run_one_input(
    output: PathBuf,
    input: PathBuf,
    bitcoind: PathBuf,
    scenario: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Running scenario with input: {}", input.display());
    let profraw_file = output.join(format!(
        "{}.coverage.profraw.%p",
        input.file_name().unwrap().to_str().unwrap()
    ));
    let status = std::process::Command::new(scenario)
        .arg(bitcoind)
        .env("LLVM_PROFILE_FILE", &profraw_file)
        .env("FUZZAMOTO_INPUT", input)
        .env("RUST_LOG", "debug")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err("Scenario failed to run".into())
    }
}

fn record_one_input(
    input: PathBuf,
    output: PathBuf,
    scenario: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Recording input: {}", input.display());
    let status = std::process::Command::new(scenario)
        .arg("./foobar")
        .env("FUZZAMOTO_RECORD_FILE", output)
        .env("FUZZAMOTO_INPUT", input)
        .env("RUST_LOG", "debug")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err("Scenario failed to run".into())
    }
}

fn get_llvm_command(base: &str) -> String {
    match std::env::var("LLVM_V") {
        Ok(version) => format!("{}-{}", base, version),
        Err(_) => base.to_string(),
    }
}

fn create_coverage_report(
    output: PathBuf,
    corpus: PathBuf,
    bitcoind: PathBuf,
    scenario: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in corpus.read_dir()? {
        let path = entry?.path();
        if let Err(e) = run_one_input(
            output.clone(),
            path.clone(),
            bitcoind.clone(),
            scenario.clone(),
        ) {
            log::error!("Failed to run input ({:?}): {}", path, e);
        }
    }

    let mut profraw_files = Vec::new();
    for entry in output.read_dir()? {
        let path = entry?.path();
        // Check if file name starts with "coverage.profraw"
        if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
            if file_name.contains("coverage.profraw") {
                profraw_files.push(path);
            }
        }
    }

    let merge_status = std::process::Command::new(get_llvm_command("llvm-profdata"))
        .arg("merge")
        .arg("-sparse")
        .args(&profraw_files)
        .arg("-o")
        .arg(output.join("coverage.profdata"))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !merge_status.success() {
        return Err("Failed to merge profraw files".into());
    }

    let mut cmd = std::process::Command::new(get_llvm_command("llvm-cov"));
    cmd.args([
        "show",
        bitcoind.to_str().unwrap(),
        &format!(
            "-instr-profile={}",
            output.join("coverage.profdata").to_str().unwrap()
        ),
        "-format=html",
        "-show-directory-coverage",
        "-show-branches=count",
        &format!(
            "-output-dir={}",
            output.join("coverage-report").to_str().unwrap()
        ),
        "-Xdemangler=c++filt",
    ]);

    let status = cmd.status()?;

    if !status.success() {
        return Err("Failed to generate HTML report".into());
    }

    Ok(())
}

fn record_test_cases(
    output: PathBuf,
    corpus: PathBuf,
    scenario: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in corpus.read_dir()? {
        let path = entry?.path();
        if let Err(e) = record_one_input(
            path.clone(),
            output.join(format!(
                "{}.recording.bin",
                path.file_name().unwrap().to_str().unwrap()
            )),
            scenario.clone(),
        ) {
            log::error!("Failed to record input ({:?}): {}", path, e);
        }
    }

    Ok(())
}

fn generate_ir(
    output: &PathBuf,
    iterations: usize,
    programs: usize,
    context: &PathBuf,
    txos: &Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let txos = if let Some(txos_file) = txos {
        let bytes = std::fs::read(txos_file).unwrap();
        let txos: Vec<fuzzamoto_ir::Txo> = postcard::from_bytes(&bytes).unwrap();
        txos
    } else {
        vec![]
    };

    let mut rng = rand::thread_rng();
    let generators: Vec<Box<dyn Generator<ThreadRng>>> = vec![
        Box::new(SendMessageGenerator::default()),
        Box::new(AdvanceTimeGenerator::default()),
        Box::new(TxoGenerator::new(txos)),
        //Box::new(WitnessGenerator::new()),
        Box::new(TxGenerator),
    ];

    let context = std::fs::read(context.clone())?;
    let context: ProgramContext = postcard::from_bytes(&context)?;

    for _ in 0..programs {
        let mut program = Program::unchecked_new(context.clone(), vec![]);

        let mut insertion_index = 0;
        for _i in 0..rng.gen_range(1..iterations) {
            let mut builder = ProgramBuilder::new(program.context.clone());
            if !program.instructions.is_empty() {
                let instrs = &program.instructions[..insertion_index];
                builder.append_all(instrs.iter().cloned()).unwrap();
            }

            let variable_threshold = builder.variable_count();

            if let Err(_) = generators
                .choose(&mut rng)
                .unwrap()
                .generate(&mut builder, &mut rng)
            {
                continue;
            }

            let second_half = Program::unchecked_new(
                builder.context().clone(),
                program.instructions[insertion_index..]
                    .iter()
                    .cloned()
                    .collect(),
            );

            builder
                .append_program(
                    &second_half,
                    variable_threshold,
                    builder.variable_count() - variable_threshold,
                )
                .unwrap();

            program = builder.finalize().unwrap();
            insertion_index = builder
                .get_random_instruction_index(&mut rng, InstructionContext::Global)
                .unwrap()
                .max(1);
        }

        let file_name = output.join(format!("{:8x}.ir", rng.r#gen::<u64>()));
        let bytes = postcard::to_allocvec(&program)?;
        std::fs::write(&file_name, &bytes)?;

        log::info!("Generated IR: {}", file_name.display());
    }

    Ok(())
}

fn compile_ir_file(input: &PathBuf, output: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    assert!(input.is_file());

    let bytes = std::fs::read(input)?;
    let program: Program = postcard::from_bytes(&bytes)?;

    let compiler = Compiler::new();
    let compiled = compiler.compile(&program).unwrap();

    let bytes = postcard::to_allocvec(&compiled)?;
    std::fs::write(output, &bytes)?;

    Ok(())
}

fn compile_ir_dir(input: &PathBuf, output: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    for entry in input.read_dir()? {
        let path = entry?.path();
        if path.is_file() && !path.file_name().unwrap().to_str().unwrap().starts_with(".") {
            log::trace!("Compiling {:?}", path);
            compile_ir_file(
                &path,
                &output
                    .join(path.file_name().unwrap())
                    .with_extension("prog"),
            )?;
        }
    }

    Ok(())
}

fn compile_ir(input: &PathBuf, output: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if input.is_file() {
        compile_ir_file(input, output)?;
    } else if input.is_dir() && output.is_dir() {
        compile_ir_dir(input, output)?;
    } else {
        return Err("Invalid input or output".into());
    }

    Ok(())
}

fn print_ir(input: &PathBuf, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(input)?;
    let program: Program = postcard::from_bytes(&bytes)?;

    if json {
        println!("{}", serde_json::to_string(&program)?);
    } else {
        println!("{}", program);
    }
    Ok(())
}

fn convert_ir_dir(
    from: &CorpusFormat,
    to: &CorpusFormat,
    input: &PathBuf,
    output: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in input.read_dir()? {
        let path = entry?.path();
        if path.is_file() && !path.file_name().unwrap().to_str().unwrap().starts_with(".") {
            let mut new_path = output.join(path.file_name().unwrap().to_str().unwrap());

            match *to {
                CorpusFormat::Postcard => {
                    new_path.set_extension("ir");
                }
                CorpusFormat::Json => {
                    new_path.set_extension("json");
                }
            }

            convert_ir_file(from, to, &path, &new_path)?;
        }
    }

    Ok(())
}

fn convert_ir_file(
    from: &CorpusFormat,
    to: &CorpusFormat,
    input: &PathBuf,
    output: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(input)?;
    let program: Program = match *from {
        CorpusFormat::Postcard => postcard::from_bytes(&bytes)?,
        CorpusFormat::Json => serde_json::from_slice(&bytes)?,
    };

    let bytes = match *to {
        CorpusFormat::Postcard => postcard::to_allocvec(&program)?,
        CorpusFormat::Json => serde_json::to_vec(&program)?,
    };
    std::fs::write(output, &bytes)?;

    Ok(())
}

fn convert_ir(
    from: &CorpusFormat,
    to: &CorpusFormat,
    input: &PathBuf,
    output: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    if input.is_file() {
        convert_ir_file(from, to, input, output)?;
    } else if input.is_dir() && output.is_dir() {
        convert_ir_dir(from, to, input, output)?;
    } else {
        return Err("Invalid input or output".into());
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Init {
            sharedir,
            crash_handler,
            bitcoind,
            scenario,
        } => {
            create_sharedir(
                sharedir.clone(),
                crash_handler.clone(),
                bitcoind.clone(),
                scenario.clone(),
            )?;
        }
        Commands::Coverage {
            output,
            corpus,
            bitcoind,
            scenario,
        } => {
            create_coverage_report(
                output.clone(),
                corpus.clone(),
                bitcoind.clone(),
                scenario.clone(),
            )?;
        }
        Commands::Record {
            output,
            corpus,
            scenario,
        } => {
            record_test_cases(output.clone(), corpus.clone(), scenario.clone())?;
        }
        Commands::IR { command } => match command {
            IRCommands::Generate {
                output,
                iterations,
                programs,
                context,
                txos,
            } => {
                generate_ir(output, *iterations, *programs, context, txos)?;
            }
            IRCommands::Compile { input, output } => {
                compile_ir(input, output)?;
            }
            IRCommands::Print { input, json } => {
                print_ir(input, *json)?;
            }
            IRCommands::Convert {
                from,
                to,
                input,
                output,
            } => {
                convert_ir(from, to, input, output)?;
            }
        },
    }

    Ok(())
}

use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;

use fuzzamoto_ir::compiler::Compiler;
use fuzzamoto_ir::{
    AdvanceTimeGenerator, BlockGenerator, FullProgramContext, Generator, HeaderGenerator,
    InstructionContext, Program, ProgramBuilder,
};

use rand::Rng;
use rand::rngs::ThreadRng;
use rand::seq::SliceRandom;

use crate::error::{CliError, Result};

pub struct IrCommand;

impl IrCommand {
    pub fn execute(command: &IRCommands) -> Result<()> {
        match command {
            IRCommands::Generate {
                output,
                iterations,
                programs,
                context,
            } => generate_ir(output, *iterations, *programs, context),
            IRCommands::Compile { input, output } => compile_ir(input, output),
            IRCommands::Print { input, json } => print_ir(input, *json),
            IRCommands::Convert {
                from,
                to,
                input,
                output,
            } => convert_ir(from, to, input, output),
            IRCommands::Analyze { input } => analyze_ir(input),
        }
    }
}

#[derive(Subcommand)]
pub enum IRCommands {
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

    /// Analyze IR corpus statistics
    Analyze {
        #[arg(help = "Path to the input IR directory to analyze")]
        input: PathBuf,
    },
}

#[derive(ValueEnum, Debug, Clone)]
pub enum CorpusFormat {
    Json,
    Postcard, // Default corpus format (https://github.com/jamesmunns/postcard)
}

pub fn generate_ir(
    output: &PathBuf,
    iterations: usize,
    programs: usize,
    context: &PathBuf,
) -> Result<()> {
    let context = std::fs::read(context.clone())?;
    let context: FullProgramContext = postcard::from_bytes(&context)?;

    let mut rng = rand::thread_rng();
    let generators: Vec<Box<dyn Generator<ThreadRng>>> = vec![
        //Box::new(SendMessageGenerator::default()),
        Box::new(AdvanceTimeGenerator::default()),
        //Box::new(TxoGenerator::new(context.txos.clone())),
        //Box::new(SingleTxGenerator),
        Box::new(HeaderGenerator::new(context.headers.clone())),
        Box::new(BlockGenerator::default()),
    ];

    for _ in 0..programs {
        let mut program = Program::unchecked_new(context.context.clone(), vec![]);

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
                    second_half,
                    variable_threshold,
                    builder.variable_count() - variable_threshold,
                )
                .unwrap();

            program = builder.finalize().unwrap();
            insertion_index = program
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

fn compile_ir_file(input: &PathBuf, output: &PathBuf) -> Result<()> {
    assert!(input.is_file());

    let bytes = std::fs::read(input)?;
    let program: Program = postcard::from_bytes(&bytes)?;

    let mut compiler = Compiler::new();
    let compiled = compiler.compile(&program).unwrap();

    let bytes = postcard::to_allocvec(&compiled)?;
    std::fs::write(output, &bytes)?;

    Ok(())
}

fn compile_ir_dir(input: &PathBuf, output: &PathBuf) -> Result<()> {
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

pub fn compile_ir(input: &PathBuf, output: &PathBuf) -> Result<()> {
    if input.is_file() {
        compile_ir_file(input, output)?;
    } else if input.is_dir() && output.is_dir() {
        compile_ir_dir(input, output)?;
    } else {
        return Err(CliError::InvalidInput(
            "Invalid input or output".to_string(),
        ));
    }

    Ok(())
}

pub fn print_ir(input: &PathBuf, json: bool) -> Result<()> {
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
) -> Result<()> {
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

            if let Err(e) = convert_ir_file(from, to, &path, &new_path) {
                log::warn!("Failed to convert from {:?} to {:?}: {}", path, new_path, e);
            }
        }
    }

    Ok(())
}

fn convert_ir_file(
    from: &CorpusFormat,
    to: &CorpusFormat,
    input: &PathBuf,
    output: &PathBuf,
) -> Result<()> {
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

pub fn convert_ir(
    from: &CorpusFormat,
    to: &CorpusFormat,
    input: &PathBuf,
    output: &PathBuf,
) -> Result<()> {
    if input.is_file() {
        convert_ir_file(from, to, input, output)?;
    } else if input.is_dir() && output.is_dir() {
        convert_ir_dir(from, to, input, output)?;
    } else {
        return Err(CliError::InvalidInput(
            "Invalid input or output".to_string(),
        ));
    }

    Ok(())
}

struct Point {
    ir_size: usize,
    compiled_size: usize,
}

fn print_size_scatter_plot(points: &[Point]) {
    const PLOT_WIDTH: usize = 100;
    const PLOT_HEIGHT: usize = 40;

    if points.is_empty() {
        println!("No data points to plot");
        return;
    }

    // Find ranges
    let max_ir = points.iter().map(|p| p.ir_size).max().unwrap();
    let max_compiled = points.iter().map(|p| p.compiled_size).max().unwrap();

    // Create grid for density calculation
    let mut grid = vec![vec![0usize; PLOT_WIDTH]; PLOT_HEIGHT];

    // Map points to grid
    for point in points {
        let x =
            (point.compiled_size as f64 / max_compiled as f64 * (PLOT_WIDTH - 1) as f64) as usize;
        let y = (point.ir_size as f64 / max_ir as f64 * (PLOT_HEIGHT - 1) as f64) as usize;
        if x < PLOT_WIDTH && y < PLOT_HEIGHT {
            grid[PLOT_HEIGHT - 1 - y][x] += 1;
        }
    }

    // Print Y-axis labels and plot
    for i in 0..PLOT_HEIGHT {
        let y_value = max_ir as f64 * (PLOT_HEIGHT - 1 - i) as f64 / (PLOT_HEIGHT - 1) as f64;
        print!("{:>5}K ┤", (y_value / 1024.0).round());

        // Print row
        for count in &grid[i] {
            match count {
                0 => print!(" "),
                1 => print!("·"),
                2..=3 => print!(":"),
                4..=5 => print!("⁘"),
                _ => print!("⬢"),
            }
        }
        println!();
    }

    // Print X-axis
    print!("      └");
    for _ in 0..PLOT_WIDTH {
        print!("─");
    }
    println!();

    // Print X-axis labels
    print!("       ");
    for i in 0..=5 {
        let x_value = max_compiled as f64 * i as f64 / 5.0;
        let label = if x_value >= 1024.0 * 1024.0 {
            format!("{:.1}M", x_value / (1024.0 * 1024.0))
        } else {
            format!("{}K", (x_value / 1024.0).round())
        };
        print!("{:>12}", label);
    }
    println!();

    // Print legend
    println!("\nDensity: · (1 program)  : (2-3)  ⁘ (4-5)  ⬢ (>5 programs)");
}

pub fn analyze_ir(input: &PathBuf) -> Result<()> {
    const IR_BUCKET_SIZE: usize = 256;
    const COMPILED_BUCKET_SIZE: usize = 1024 * 75;
    const SENDS_BUCKET_SIZE: usize = 1;
    const INSTRUCTIONS_BUCKET_SIZE: usize = 30;
    assert!(input.is_dir());

    // Initialize histograms
    let mut ir_size_hist = vec![];
    let mut compiled_size_hist = vec![];
    let mut scatter_points = vec![];
    let mut sends_per_program_hist = vec![];
    let mut instructions_hist = vec![];

    // Process each file
    for entry in input.read_dir()? {
        let path = entry?.path();
        if path.is_file() && !path.file_name().unwrap().to_str().unwrap().starts_with(".") {
            // Read and parse the IR file
            let bytes = std::fs::read(&path)?;
            if let Ok(program) = postcard::from_bytes::<fuzzamoto_ir::Program>(&bytes) {
                // Count instructions
                let instr_count = program.instructions.len();
                let bucket = instr_count / INSTRUCTIONS_BUCKET_SIZE;
                instructions_hist.resize(instructions_hist.len().max(bucket + 1), 0);
                instructions_hist[bucket] += 1;

                // Compile the program
                let mut compiler = fuzzamoto_ir::compiler::Compiler::new();
                if let Ok(compiled) = compiler.compile(&program) {
                    // Count total sends in this program
                    let sends = compiled
                        .actions
                        .iter()
                        .filter(|action| {
                            matches!(
                                action,
                                fuzzamoto_ir::compiler::CompiledAction::SendRawMessage(..)
                            )
                        })
                        .count()
                        / SENDS_BUCKET_SIZE;

                    // Update histogram
                    sends_per_program_hist.resize(sends_per_program_hist.len().max(sends + 1), 0);
                    sends_per_program_hist[sends] += 1;

                    // Get compiled size
                    let compiled_bytes = postcard::to_allocvec(&compiled)?;
                    let compiled_size = compiled_bytes.len();
                    let bucket = compiled_size / COMPILED_BUCKET_SIZE;
                    compiled_size_hist.resize(compiled_size_hist.len().max(bucket + 1), 0);
                    compiled_size_hist[bucket] += 1;

                    scatter_points.push(Point {
                        ir_size: bytes.len(),
                        compiled_size,
                    });
                }
            }

            // Get IR file size for histogram
            let bucket = bytes.len() / IR_BUCKET_SIZE;
            ir_size_hist.resize(ir_size_hist.len().max(bucket + 1), 0);
            ir_size_hist[bucket] += 1;
        }
    }

    println!("\nNumber of Send Operations per Program");
    println!("-----------------------------------");
    print_histogram(&sends_per_program_hist, SENDS_BUCKET_SIZE, "sends");

    println!("\nIR Size vs Compiled Size Distribution");
    println!("------------------------------------");
    print_size_scatter_plot(&scatter_points);

    println!(
        "\nIR Instruction Count Distribution (bucket size: {} instructions)",
        INSTRUCTIONS_BUCKET_SIZE
    );
    println!("----------------------------------------------------");
    print_histogram(&instructions_hist, INSTRUCTIONS_BUCKET_SIZE, "instructions");

    println!(
        "\nIR File Size Distribution (bucket size: {} bytes)",
        IR_BUCKET_SIZE
    );
    println!("------------------------------------------------");
    print_histogram(&ir_size_hist, IR_BUCKET_SIZE, "bytes");

    println!(
        "\nCompiled Size Distribution (bucket size: {} bytes)",
        COMPILED_BUCKET_SIZE
    );
    println!("-------------------------------------------------");
    print_histogram(&compiled_size_hist, COMPILED_BUCKET_SIZE, "bytes");

    Ok(())
}

fn print_histogram(hist: &[i32], bucket_size: usize, label: &str) {
    let max_count = hist.iter().max().unwrap_or(&0);
    if *max_count == 0 {
        println!("No data points");
        return;
    }

    const WIDTH: usize = 60;
    // Find the width needed for the largest number to ensure proper alignment
    let max_width = (hist.len() * bucket_size).to_string().len();

    for (i, count) in hist.iter().enumerate() {
        if *count > 0 {
            let bar_width = (*count as f64 / *max_count as f64 * WIDTH as f64) as usize;
            println!(
                "{:>width$}-{:<width$} {} {:<5}[{:>4}]: {}",
                i * bucket_size,
                (i + 1) * bucket_size - 1,
                label,
                "", // Padding before bracket
                count,
                "█".repeat(bar_width),
                width = max_width
            );
        }
    }
}

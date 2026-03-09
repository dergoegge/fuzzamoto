use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use fuzzamoto_ir::{Operation, Program, ProgramBuilder, ProgramContext, VariableLookup};

const NUM_INSTRUCTIONS: usize = 100_000;

fn test_context() -> ProgramContext {
    ProgramContext {
        num_nodes: 2,
        num_connections: 4,
        timestamp: 1_700_000_000,
    }
}

fn create_test_program() -> Program {
    let operations = [
        Operation::LoadBytes(vec![0u8; 32]),
        Operation::LoadAmount(1000),
        Operation::LoadTime(1_700_000_000),
        Operation::LoadNode(0),
        Operation::LoadConnection(0),
        Operation::LoadSize(100),
        Operation::LoadTxVersion(2),
        Operation::LoadLockTime(0),
        Operation::LoadSequence(0xffffffff),
        Operation::LoadBlockHeight(100),
    ];

    let mut builder = ProgramBuilder::new(test_context());
    for i in 0..NUM_INSTRUCTIONS {
        builder.force_append(vec![], &operations[i % operations.len()]);
    }
    builder.finalize().expect("Program should be valid")
}

fn bench_large_program(c: &mut Criterion) {
    let program = create_test_program();

    c.bench_function("program_builder_100k", |b| {
        b.iter(|| {
            let rebuilt = ProgramBuilder::from_program(Program::unchecked_new(
                program.context.clone(),
                program.instructions.clone(),
            ))
            .unwrap();
            black_box(rebuilt.variable_count())
        })
    });

    c.bench_function("variable_lookup_100k", |b| {
        b.iter(|| {
            let lookup = VariableLookup::from_instructions(&program.instructions);
            black_box(lookup.variable_count())
        })
    });
}

criterion_group!(benches, bench_large_program);
criterion_main!(benches);

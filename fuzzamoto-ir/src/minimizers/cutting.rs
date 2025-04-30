use crate::Program;

use super::Minimizer;

pub struct CuttingMinimizer {
    original: Program,

    current: usize,
    chopped: usize,
}

impl Minimizer for CuttingMinimizer {
    fn new(program: Program) -> Self {
        Self {
            original: program.clone(),
            current: (program.instructions.len() as f64 / 2.0) as usize,
            chopped: program.instructions.len(),
        }
    }

    fn success(&mut self) {
        self.chopped = self.current;
        self.current = (self.current as f64 / 2.0) as usize;
    }

    fn failure(&mut self) {
        self.current += (((self.chopped - self.current) as f64 / 2.0) as usize).max(1);
    }
}

impl Iterator for CuttingMinimizer {
    type Item = Program;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current == self.chopped || self.current > self.original.instructions.len() {
            return None;
        }

        Some(Program::unchecked_new(
            self.original.context.clone(),
            self.original.instructions[..self.current].to_vec(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::{Instruction, InstructionContext, Operation};
    use rand::Rng;

    fn create_test_program(size: usize) -> Program {
        let context = crate::ProgramContext {
            num_nodes: 1,
            num_connections: 1,
            timestamp: 0,
        };
        let instructions = vec![
            Instruction {
                inputs: vec![],
                operation: Operation::Nop {
                    outputs: 0,
                    inner_outputs: 0
                }
            };
            size
        ];
        Program::unchecked_new(context, instructions)
    }

    #[test]
    fn test_rnd() {
        let program = create_test_program(10000);
        let mut minimizer = CuttingMinimizer::new(program.clone());

        let mut rng = rand::thread_rng();
        let mut set = HashMap::new();
        while let Some(program) = minimizer.next() {
            println!(
                "current={} chopped={}",
                minimizer.current, minimizer.chopped
            );
            if !set.contains_key(&minimizer.current) {
                set.insert(minimizer.current, rng.gen_bool(0.5));
            }
            if *set.get(&minimizer.current).unwrap() {
                minimizer.success();
            } else {
                minimizer.failure();
            }
        }
    }
}

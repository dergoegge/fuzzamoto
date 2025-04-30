pub mod builder;
pub mod compiler;
pub mod errors;
pub mod generators;
pub mod instruction;
pub mod minimizers;
pub mod mutators;
pub mod operation;
pub mod variable;

use crate::errors::*;
pub use builder::*;
pub use generators::*;
pub use instruction::*;
pub use minimizers::*;
pub use mutators::*;
pub use operation::*;
pub use variable::*;

use std::{fmt, hash::Hash};

/// Program represent a sequence of operations to perform on target nodes.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Hash)]
pub struct Program {
    pub instructions: Vec<Instruction>,
    pub context: ProgramContext,
}

/// ProgramContext represents the context in which a program is executed. This mostly describes the
/// snapshot state of the target nodes.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Hash)]
pub struct ProgramContext {
    pub num_nodes: usize,
    pub num_connections: usize,
    pub timestamp: u64,
}

impl Program {
    pub fn unchecked_new(context: ProgramContext, instructions: Vec<Instruction>) -> Self {
        Self {
            instructions,
            context,
        }
    }

    pub fn is_statically_valid(&self) -> bool {
        ProgramBuilder::from_program(self).is_ok()
    }

    pub fn to_builder(&self) -> Option<ProgramBuilder> {
        match ProgramBuilder::from_program(self) {
            Ok(prog) => Some(prog),
            Err(_) => None,
        }
    }

    pub fn remove_nops(&mut self) {
        debug_assert!(self.is_statically_valid());
        self.instructions = self
            .instructions
            .drain(..)
            .filter(|instr| !matches!(&instr.operation, Operation::Nop { .. }))
            .collect();
        debug_assert!(self.is_statically_valid());
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "// Context: nodes={} connections={} timestamp={}\n",
            self.context.num_nodes, self.context.num_connections, self.context.timestamp
        )?;
        let mut var_counter = 0;
        let mut indent_counter = 0;

        for instruction in &self.instructions {
            if indent_counter > 0 {
                let offset = if instruction.operation.is_block_end() {
                    1
                } else {
                    0
                };
                write!(f, "{}", "  ".repeat(indent_counter - offset))?;
            }

            if instruction.operation.num_outputs() > 0 {
                for _ in 0..(instruction.operation.num_outputs() - 1) {
                    write!(f, "v{}, ", var_counter)?;
                    var_counter += 1;
                }
                write!(f, "v{}", var_counter)?;
                var_counter += 1;
                write!(f, " <- ")?;
            }
            write!(f, "{}", instruction.operation)?;

            if instruction.operation.num_inputs() > 0 {
                write!(f, "(")?;
                for input in &instruction.inputs[..instruction.operation.num_inputs() - 1] {
                    write!(f, "v{}, ", input)?;
                }
                write!(
                    f,
                    "v{}",
                    instruction.inputs[instruction.operation.num_inputs() - 1]
                )?;
                write!(f, ")")?;
            }

            if instruction.operation.num_inner_outputs() > 0 {
                write!(f, " -> ")?;
                for _ in 0..(instruction.operation.num_inner_outputs() - 1) {
                    write!(f, "v{}, ", var_counter)?;
                    var_counter += 1;
                }
                write!(f, "v{}", var_counter)?;
                var_counter += 1;
            }
            write!(f, "\n")?;

            if instruction.operation.is_block_begin() {
                indent_counter += 1;
            }
            if instruction.operation.is_block_end() {
                indent_counter -= 1;
            }
        }
        Ok(())
    }
}

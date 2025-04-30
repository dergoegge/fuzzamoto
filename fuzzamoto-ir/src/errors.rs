use crate::{Operation, ProgramContext, Variable};

#[derive(Debug, Clone)]
pub enum ProgramValidationError {
    VariableNotDefined(usize),
    NodeNotFound(usize),
    ConnectionNotFound(usize),
    InvalidConnectionType(String),
    InvalidVariableType {
        is: Option<Variable>,
        expected: Variable,
    },
    InvalidNumberOfInputs {
        is: usize,
        expected: usize,
    },
    InvalidBlockEnd {
        begin: Operation,
        end: Operation,
    },
    ScopeStillOpen,
}

#[derive(Debug, Clone)]
pub enum ProgramSpliceError {
    InvalidIndex(usize),
    ContextMismatch {
        expected: ProgramContext,
        actual: ProgramContext,
    },
}

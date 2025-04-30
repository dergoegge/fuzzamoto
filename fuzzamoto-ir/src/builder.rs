use std::collections::HashSet;

use rand::{Rng, RngCore, seq::IteratorRandom};

use crate::{
    Instruction, InstructionContext, Operation, Program, ProgramContext, ProgramValidationError,
    Variable,
};

pub struct Scope {
    begin: Option<usize>, // Index of begin op
    id: usize,            // Scope index

    // Context that the begin instruction entered
    context: InstructionContext,
}

/// Variable and its containing scope id
#[derive(Debug)]
pub struct ScopedVariable {
    pub var: Variable,
    pub scope_id: usize,
}

/// Variable and its index
#[derive(Debug, Clone)]
pub struct IndexedVariable {
    pub var: Variable,
    pub index: usize,
}

pub struct ProgramBuilder {
    // Context of the program to be created
    context: ProgramContext,

    // Active scopes (only variables in an active scope are usable by instructions)
    active_scopes: Vec<Scope>,         // stack of active scopes
    active_scopes_set: HashSet<usize>, // set of active scope ids (for quick lookups)
    // Monotonically increasing counter for unique scope ids
    scope_counter: usize,

    // All variables created by `instructions`
    variables: Vec<ScopedVariable>,
    // Instruction in the program
    pub instructions: Vec<Instruction>,

    contexts: Vec<InstructionContext>,
}

impl ProgramBuilder {
    pub fn new(context: ProgramContext) -> Self {
        let mut builder = Self {
            context,
            active_scopes: Vec::new(),
            active_scopes_set: HashSet::new(),
            scope_counter: 0usize,
            variables: Vec::new(),
            instructions: Vec::new(),
            contexts: Vec::new(),
        };

        // Enter outer/global scope of the program (never exited)
        builder.enter_scope(None, InstructionContext::Global);

        builder
    }

    pub fn from_program(program: &Program) -> Result<ProgramBuilder, ProgramValidationError> {
        let mut builder = Self::new(program.context.clone());

        builder.append_program(program, 0usize, 0usize)?;

        Ok(builder)
    }

    pub fn context(&self) -> &ProgramContext {
        &self.context
    }

    pub fn variable_count(&self) -> usize {
        self.variables.len()
    }

    fn is_variable_in_scope(&self, variable_index: usize) -> bool {
        let ScopedVariable { var: _, scope_id } = &self.variables[variable_index];
        self.is_scope_active(*scope_id)
    }
    fn is_scope_active(&self, scope_id: usize) -> bool {
        self.active_scopes_set.contains(&scope_id)
    }

    fn enter_scope(&mut self, begin: Option<usize>, context: InstructionContext) {
        self.scope_counter += 1;
        self.active_scopes.push(Scope {
            begin,
            id: self.scope_counter,
            context: context.clone(),
        });
        self.active_scopes_set.insert(self.scope_counter);
    }

    fn exit_scope(&mut self) -> Scope {
        let exited = self
            .active_scopes
            .pop()
            .expect("There must always be an active scope");

        assert!(self.active_scopes_set.remove(&exited.id));

        exited
    }

    fn current_scope(&self) -> &Scope {
        self.active_scopes
            .last()
            .expect("There must always be an active scope")
    }

    /// Append a single instruction
    ///
    /// Checks static signle assignment form and instruction input type correctness.
    pub fn append(
        &mut self,
        instruction: Instruction,
    ) -> Result<Vec<IndexedVariable>, ProgramValidationError> {
        // Check number of inputs first
        if instruction.operation.num_inputs() != instruction.inputs.len() {
            return Err(ProgramValidationError::InvalidNumberOfInputs {
                is: instruction.inputs.len(),
                expected: instruction.operation.num_inputs(),
            });
        }

        // Collect input variable types
        let mut input_vars = Vec::with_capacity(instruction.inputs.len());

        for input_idx in &instruction.inputs {
            if *input_idx >= self.variables.len() {
                return Err(ProgramValidationError::VariableNotDefined(*input_idx));
            }

            let ScopedVariable { var, scope_id } = &self.variables[*input_idx];
            if self.is_scope_active(*scope_id) {
                input_vars.push(var.clone());
            } else {
                // Variable is not defined in any of the active scopes
                return Err(ProgramValidationError::VariableNotDefined(*input_idx));
            }
        }

        // Check input types for the operation
        instruction.operation.check_input_types(&input_vars)?;

        match &instruction.operation {
            Operation::LoadNode(idx) => {
                if *idx >= self.context.num_nodes {
                    return Err(ProgramValidationError::NodeNotFound(*idx));
                }
            }
            Operation::LoadConnection(idx) => {
                if *idx >= self.context.num_connections {
                    return Err(ProgramValidationError::ConnectionNotFound(*idx));
                }
            }
            Operation::LoadConnectionType(connection_type) => match connection_type.as_str() {
                "outbound" | "inbound" => {}
                _ => {
                    return Err(ProgramValidationError::InvalidConnectionType(
                        connection_type.clone(),
                    ));
                }
            },

            _ => {}
        }

        // The instruction context prior to a block beginning or ending is used as the context for
        // the block instruction.
        self.contexts.push(self.current_scope().context.clone());

        if instruction.operation.is_block_end() {
            let last_scope = self.exit_scope();
            if !instruction
                .operation
                .is_matching_block_begin(&self.instructions[last_scope.begin.unwrap()].operation)
            {
                return Err(ProgramValidationError::InvalidBlockEnd {
                    begin: self.instructions[last_scope.begin.unwrap()]
                        .operation
                        .clone(),
                    end: instruction.operation.clone(),
                });
            }
        }

        let prev_variable_count = self.variables.len();

        let current_scope_id = match instruction.operation {
            Operation::Nop { .. } => 0usize, // All nop vars are out of scope
            _ => self.current_scope().id,
        };
        self.variables.extend(
            instruction
                .operation
                .get_output_variables()
                .iter()
                .map(|v| ScopedVariable {
                    var: v.clone(),
                    scope_id: current_scope_id,
                }),
        );

        if instruction.operation.is_block_begin() {
            self.enter_scope(
                Some(self.instructions.len()),
                // Unwrap as this is guaranteed to be a block beginning
                instruction.entered_context_after_execution().unwrap(),
            );

            let scope_id = match instruction.operation {
                Operation::Nop { .. } => 0usize, // All nop vars are out of scope
                _ => self.scope_counter,
            };

            self.variables.extend(
                instruction
                    .operation
                    .get_inner_output_variables()
                    .iter()
                    .map(|v| ScopedVariable {
                        var: v.clone(),
                        scope_id,
                    }),
            );
        }
        self.instructions.push(instruction.clone());

        Ok(self.variables[prev_variable_count..]
            .iter()
            .enumerate()
            .map(|(i, ScopedVariable { var, scope_id: _ })| IndexedVariable {
                var: var.clone(),
                index: prev_variable_count + i,
            })
            .collect())
    }

    /// Append a sequence of instructions
    pub fn append_all<'a>(
        &mut self,
        instructions: impl Iterator<Item = Instruction>,
    ) -> Result<Vec<IndexedVariable>, ProgramValidationError> {
        let mut variables = Vec::new();
        for instruction in instructions {
            variables.extend(self.append(instruction)?.drain(..));
        }

        // return all variables that are still in scope after appending the instructions
        Ok(variables
            .drain(..)
            .filter(|IndexedVariable { var: _, index }| self.is_variable_in_scope(*index))
            .collect())
    }

    /// Append an entire program and remap input variables to ensure correctness.
    ///
    /// Instruction input variable indecies above the `variable_threshold` are remapped to be
    /// offset by `variable_offset`.
    pub fn append_program(
        &mut self,
        program: &Program,
        variable_threshold: usize,
        variable_offset: usize,
    ) -> Result<(), ProgramValidationError> {
        assert!(program.context == self.context);

        let mapped_instructions = program.instructions.iter().map(|i| {
            let mut cloned_instruction = i.clone();
            for input in &mut cloned_instruction.inputs {
                if *input >= variable_threshold {
                    *input += variable_offset;
                }
            }
            cloned_instruction
        });

        self.append_all(mapped_instructions)?;

        Ok(())
    }
    pub fn append_program_without_threshold(
        &mut self,
        program: &Program,
        variable_offset: usize,
    ) -> Result<(), ProgramValidationError> {
        self.append_program(program, 0usize, variable_offset)
    }

    pub fn force_append(
        &mut self,
        inputs: Vec<usize>,
        operation: Operation,
    ) -> Vec<IndexedVariable> {
        self.append(Instruction {
            inputs,
            operation: operation.clone(),
        })
        .expect(&format!(
            "Force append should not faile for {:?}",
            operation
        ))
    }

    pub fn force_append_expect_output(
        &mut self,
        inputs: Vec<usize>,
        operation: Operation,
    ) -> IndexedVariable {
        self.force_append(inputs, operation.clone())
            .pop()
            .expect(&format!(
                "One new output var should have been created for {:?}",
                operation
            ))
    }

    /// Construct a `Program` from the builder
    pub fn finalize(&self) -> Result<Program, ProgramValidationError> {
        assert!(
            self.active_scopes.len() == self.active_scopes_set.len(),
            "Internal program scope accounting bug"
        );

        if self.active_scopes.len() != 1 {
            return Err(ProgramValidationError::ScopeStillOpen);
        }

        Ok(Program::unchecked_new(
            self.context.clone(),
            self.instructions.clone(),
        ))
    }

    pub fn get_variable(&self, index: usize) -> Option<IndexedVariable> {
        let scoped_variable = self.variables.get(index)?;
        if self.is_scope_active(scoped_variable.scope_id) {
            Some(IndexedVariable {
                var: scoped_variable.var.clone(),
                index,
            })
        } else {
            None
        }
    }

    /// Get the nearest (searched in reverse) available (in the current scope) variable of a given
    /// type
    pub fn get_nearest_variable(&self, find: Variable) -> Option<IndexedVariable> {
        self.variables
            .iter()
            .enumerate()
            .filter(|(_, ScopedVariable { var, scope_id })| {
                self.is_scope_active(*scope_id) && *var == find
            })
            .map(
                |(index, ScopedVariable { var, scope_id: _ })| IndexedVariable {
                    var: var.clone(),
                    index,
                },
            )
            .last()
    }

    pub fn get_random_instruction_index<R: RngCore>(
        &self,
        rng: &mut R,
        context: InstructionContext,
    ) -> Option<usize> {
        if self.contexts.is_empty() && matches!(context, InstructionContext::Global) {
            // Empty program
            return Some(0);
        }

        self.contexts
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == context)
            .map(|(i, _)| i)
            .choose(rng)
    }

    pub fn get_or_create_random_connection<R: RngCore>(&mut self, rng: &mut R) -> IndexedVariable {
        match self.get_random_variable(rng, Variable::Connection) {
            Some(v) => v,
            None => self.force_append_expect_output(
                vec![],
                Operation::LoadConnection(rng.gen_range(0..self.context.num_connections)),
            ),
        }
    }

    /// Get a random available (in the current scope) variable of a given type
    pub fn get_random_variable<R: RngCore>(
        &self,
        rng: &mut R,
        find: Variable,
    ) -> Option<IndexedVariable> {
        self.variables
            .iter()
            .enumerate()
            .filter(|(_, ScopedVariable { var, scope_id })| {
                self.is_scope_active(*scope_id) && *var == find
            })
            .map(
                |(index, ScopedVariable { var, scope_id: _ })| IndexedVariable {
                    var: var.clone(),
                    index,
                },
            )
            .choose(rng)
    }

    /// Get some random available (in the current scope) variables of a given type
    pub fn get_random_variables<R: RngCore>(
        &self,
        rng: &mut R,
        find: Variable,
    ) -> Vec<IndexedVariable> {
        let available = self
            .variables
            .iter()
            .enumerate()
            .filter(|(_, ScopedVariable { var, scope_id })| {
                self.is_scope_active(*scope_id) && *var == find
            })
            .map(
                |(index, ScopedVariable { var, scope_id: _ })| IndexedVariable {
                    var: var.clone(),
                    index,
                },
            );

        let n = rng.gen_range(0..available.clone().count()); // TODO maybe use size_hint instead?
        available.choose_multiple(rng, n + 1)
    }

    /// Get a random set of unspend transaction outputs
    pub fn get_random_utxos<R: RngCore>(&self, rng: &mut R) -> Vec<IndexedVariable> {
        let mut utxos = HashSet::new();

        let mut var_count = 0;
        for instruction in self.instructions.iter() {
            match instruction.operation {
                Operation::TakeTxo | Operation::LoadTxo { .. } => {
                    utxos.insert(var_count);
                }
                Operation::AddTxInput => {
                    if !utxos.remove(&instruction.inputs[1]) {
                        continue;
                    }
                    // AddTxInput instructions have no output variables so we can remove them and
                    // use `variable_count` above without issue
                }
                _ => {}
            }

            var_count += instruction.operation.num_outputs();
            var_count += instruction.operation.num_inner_outputs();
        }

        let all_utxos = utxos
            .iter()
            .filter(|index| self.is_variable_in_scope(**index))
            .map(|index| {
                let var = self.variables[*index].var.clone();
                assert!(matches!(var, Variable::Txo));
                IndexedVariable { var, index: *index }
            });

        let num_utxos = all_utxos.clone().count();
        if num_utxos == 0 {
            return Vec::new();
        }

        let n = rng.gen_range(0..num_utxos);
        all_utxos.choose_multiple(rng, n + 1)
    }
}

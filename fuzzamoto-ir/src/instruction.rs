use crate::Operation;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Hash)]
pub struct Instruction {
    pub inputs: Vec<usize>,
    pub operation: Operation,
}

impl Instruction {
    pub fn is_input_mutable(&self) -> bool {
        assert!(self.inputs.len() == self.operation.num_inputs());

        match self.operation {
            Operation::EndBuildTx
            | Operation::BeginBuildTxInputs
            | Operation::BeginBuildTxOutputs
            | Operation::EndBuildTxInputs
            | Operation::EndBuildTxOutputs
            | Operation::TakeTxo => false,
            _ => self.inputs.len() > 0,
        }
    }

    pub fn is_operation_mutable(&self) -> bool {
        match self.operation {
            Operation::LoadAmount(_)
            | Operation::LoadTxVersion(_)
            | Operation::LoadSequence(_)
            | Operation::LoadLockTime(_)
            | Operation::LoadNode(_)
            | Operation::LoadConnection(_)
            | Operation::LoadConnectionType(_)
            | Operation::LoadDuration(_)
            | Operation::LoadTime(_)
            | Operation::LoadSize(_)
            | Operation::LoadTxo { .. }
            | Operation::SendTxInv { .. }
            | Operation::LoadBytes(_) => true,
            _ => false,
        }
    }

    /// If the instruction is a block beginning, return the context that is entered after the
    /// instruction is executed.
    pub fn entered_context_after_execution(&self) -> Option<InstructionContext> {
        if self.operation.is_block_begin() {
            return match self.operation {
                Operation::BeginBuildTx => Some(InstructionContext::BuildTx),
                Operation::BeginBuildTxInputs => Some(InstructionContext::BuildTxInputs),
                Operation::BeginBuildTxOutputs => Some(InstructionContext::BuildTxOutputs),
                Operation::BeginWitnessStack => Some(InstructionContext::WitnessStack),
                _ => unimplemented!("Every block begin enters a context"),
            };
        }

        None
    }
}

/// `InstructionContext` describes the context in which an `Instruction` is executed
#[derive(Debug, Clone, PartialEq)]
pub enum InstructionContext {
    /// The instruction is executed in the global context
    Global,
    BuildTx,
    BuildTxInputs,
    BuildTxOutputs,
    WitnessStack,
}

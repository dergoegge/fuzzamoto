use crate::{ProgramValidationError, Variable};

use std::{fmt, time::Duration};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Hash)]
pub enum Operation {
    /// No operation
    Nop {
        outputs: usize,
        inner_outputs: usize,
    },

    /// `Load*` operations load data from the program's context
    LoadBytes(Vec<u8>),
    LoadMsgType([char; 12]),
    LoadNode(usize),
    LoadConnection(usize),
    LoadConnectionType(String),
    LoadDuration(Duration),
    LoadTime(u64),
    LoadTxo {
        outpoint: ([u8; 32], u32),
        value: u64,
        script_pubkey: Vec<u8>,

        spending_script_sig: Vec<u8>,
        spending_witness: Vec<Vec<u8>>,
    },
    LoadAmount(u64),
    LoadSize(usize),

    LoadTxVersion(u32),
    LoadLockTime(u32),
    LoadSequence(u32),

    /// Send a message given a connection, message type and bytes
    SendRawMessage,
    /// Advance a time variable by a given duration
    AdvanceTime,
    /// Set mock time
    SetTime,

    /// Script building operations
    BuildRawScripts,
    BuildPayToWitnessScriptHash,
    BuildOpReturnScripts,
    BeginWitnessStack,
    EndWitnessStack,
    AddWitness,

    /// Transaction building operations
    BeginBuildTx,
    EndBuildTx,
    BeginBuildTxInputs,
    EndBuildTxInputs,
    BeginBuildTxOutputs,
    EndBuildTxOutputs,
    AddTxOutput,
    AddTxInput,
    TakeTxo,

    SendTxInv {
        wtxid: bool,
    },
    SendTx,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::Nop { .. } => write!(f, "Nop"),
            Operation::LoadBytes(bytes) => write!(
                f,
                "LoadBytes(\"{}\")",
                bytes
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>()
            ), // as hex
            Operation::LoadMsgType(msg_type) => write!(
                f,
                "LoadMsgType(\"{}\")",
                msg_type.iter().map(|c| *c as char).collect::<String>()
            ),
            Operation::LoadNode(index) => write!(f, "LoadNode({})", index),
            Operation::LoadConnection(index) => write!(f, "LoadConnection({})", index),
            Operation::LoadConnectionType(connection_type) => {
                write!(f, "LoadConnectionType(\"{}\")", connection_type)
            }
            Operation::LoadDuration(duration) => write!(f, "LoadDuration({})", duration.as_secs()),
            Operation::SendRawMessage => write!(f, "SendRawMessage"),
            Operation::AdvanceTime => write!(f, "AdvanceTime"),
            Operation::LoadTime(time) => write!(f, "LoadTime({})", time),
            Operation::SetTime => write!(f, "SetTime"),
            Operation::BuildRawScripts => write!(f, "BuildRawScripts"),
            Operation::BuildPayToWitnessScriptHash => write!(f, "BuildPayToWitnessScriptHash"),
            Operation::BuildOpReturnScripts => write!(f, "BuildOpReturnScripts"),
            Operation::LoadTxo {
                outpoint,
                value,
                script_pubkey,
                spending_script_sig,
                spending_witness,
            } => write!(
                f,
                "LoadTxo({}:{}, {}, {}, {}, {})",
                hex_string(&outpoint.0),
                outpoint.1,
                value,
                hex_string(&script_pubkey),
                hex_string(&spending_script_sig),
                hex_witness_stack(&spending_witness),
            ),
            Operation::LoadAmount(amount) => write!(f, "LoadAmount({})", amount),
            Operation::LoadTxVersion(version) => write!(f, "LoadTxVersion({})", version),
            Operation::LoadLockTime(lock_time) => write!(f, "LoadLockTime({})", lock_time),
            Operation::LoadSequence(sequence) => write!(f, "LoadSequence({})", sequence),
            Operation::LoadSize(size) => write!(f, "LoadSize({})", size),
            Operation::BeginBuildTx => write!(f, "BeginBuildTx"),
            Operation::EndBuildTx => write!(f, "EndBuildTx"),
            Operation::BeginBuildTxInputs => write!(f, "BeginBuildTxInputs"),
            Operation::EndBuildTxInputs => write!(f, "EndBuildTxInputs"),
            Operation::BeginBuildTxOutputs => write!(f, "BeginBuildTxOutputs"),
            Operation::EndBuildTxOutputs => write!(f, "EndBuildTxOutputs"),
            Operation::AddTxInput => write!(f, "AddTxInput"),
            Operation::AddTxOutput => write!(f, "AddTxOutput"),
            Operation::TakeTxo => write!(f, "TakeTxo"),
            Operation::BeginWitnessStack => write!(f, "BeginWitnessStack"),
            Operation::EndWitnessStack => write!(f, "EndWitnessStack"),
            Operation::AddWitness => write!(f, "AddWitness"),
            Operation::SendTx => write!(f, "SendTx"),
            Operation::SendTxInv { wtxid } => {
                if *wtxid {
                    write!(f, "SendWtxidInv")
                } else {
                    write!(f, "SendTxidInv")
                }
            }
        }
    }
}

fn hex_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

fn hex_witness_stack(witness: &[Vec<u8>]) -> String {
    witness.iter().map(|b| hex_string(b)).collect::<String>()
}

impl Operation {
    pub fn mutates_nth_input(&self, index: usize) -> bool {
        match self {
            Operation::AddTxInput if index == 0 => true,
            Operation::AddTxOutput if index == 0 => true,
            Operation::TakeTxo if index == 0 => true,
            Operation::AddWitness if index == 0 => true,
            _ => false,
        }
    }

    pub fn is_block_begin(&self) -> bool {
        match self {
            Operation::BeginBuildTx
            | Operation::BeginBuildTxInputs
            | Operation::BeginBuildTxOutputs
            | Operation::BeginWitnessStack => true,
            // Exhaustive match to fail when new ops are added
            Operation::Nop { .. }
            | Operation::LoadBytes(_)
            | Operation::LoadMsgType(_)
            | Operation::LoadNode(_)
            | Operation::LoadConnection(_)
            | Operation::LoadConnectionType(_)
            | Operation::LoadDuration(_)
            | Operation::SendRawMessage
            | Operation::AdvanceTime
            | Operation::LoadTime(_)
            | Operation::LoadSize(_)
            | Operation::SetTime
            | Operation::BuildPayToWitnessScriptHash
            | Operation::BuildRawScripts
            | Operation::BuildOpReturnScripts
            | Operation::LoadTxo { .. }
            | Operation::LoadAmount(..)
            | Operation::LoadTxVersion(..)
            | Operation::LoadLockTime(..)
            | Operation::LoadSequence(..)
            | Operation::EndBuildTx
            | Operation::EndBuildTxInputs
            | Operation::EndBuildTxOutputs
            | Operation::AddTxInput
            | Operation::AddTxOutput
            | Operation::TakeTxo
            | Operation::EndWitnessStack
            | Operation::AddWitness
            | Operation::SendTxInv { .. }
            | Operation::SendTx => false,
        }
    }

    pub fn allow_insertion_in_block(&self) -> bool {
        if self.is_block_begin() {
            return false;
        }
        true
    }

    pub fn is_matching_block_begin(&self, other: &Operation) -> bool {
        match (other, self) {
            (Operation::BeginBuildTx, Operation::EndBuildTx)
            | (Operation::BeginBuildTxInputs, Operation::EndBuildTxInputs)
            | (Operation::BeginBuildTxOutputs, Operation::EndBuildTxOutputs)
            | (Operation::BeginWitnessStack, Operation::EndWitnessStack) => true,
            _ => false,
        }
    }

    pub fn is_block_end(&self) -> bool {
        match self {
            Operation::EndBuildTx
            | Operation::EndBuildTxInputs
            | Operation::EndBuildTxOutputs
            | Operation::EndWitnessStack => true,
            // Exhaustive match to fail when new ops are added
            Operation::Nop { .. }
            | Operation::LoadBytes(_)
            | Operation::LoadMsgType(_)
            | Operation::LoadNode(_)
            | Operation::LoadConnection(_)
            | Operation::LoadConnectionType(_)
            | Operation::LoadDuration(_)
            | Operation::SendRawMessage
            | Operation::AdvanceTime
            | Operation::LoadTime(_)
            | Operation::LoadSize(_)
            | Operation::SetTime
            | Operation::BuildPayToWitnessScriptHash
            | Operation::BuildRawScripts
            | Operation::BuildOpReturnScripts
            | Operation::LoadTxo { .. }
            | Operation::LoadAmount(..)
            | Operation::LoadTxVersion(..)
            | Operation::LoadLockTime(..)
            | Operation::LoadSequence(..)
            | Operation::BeginBuildTx
            | Operation::BeginBuildTxInputs
            | Operation::BeginBuildTxOutputs
            | Operation::AddTxInput
            | Operation::AddTxOutput
            | Operation::TakeTxo
            | Operation::BeginWitnessStack
            | Operation::AddWitness
            | Operation::SendTxInv { .. }
            | Operation::SendTx => false,
        }
    }

    pub fn num_inner_outputs(&self) -> usize {
        match self {
            Operation::BeginBuildTx
            | Operation::BeginBuildTxInputs
            | Operation::BeginBuildTxOutputs
            | Operation::BeginWitnessStack => 1,
            Operation::Nop {
                outputs: _,
                inner_outputs,
            } => *inner_outputs,
            // Exhaustive match to fail when new ops are added
            Operation::LoadBytes(_)
            | Operation::LoadMsgType(_)
            | Operation::LoadNode(_)
            | Operation::LoadConnection(_)
            | Operation::LoadConnectionType(_)
            | Operation::LoadDuration(_)
            | Operation::SendRawMessage
            | Operation::AdvanceTime
            | Operation::LoadTime(_)
            | Operation::LoadSize(_)
            | Operation::SetTime
            | Operation::BuildPayToWitnessScriptHash
            | Operation::BuildRawScripts
            | Operation::BuildOpReturnScripts
            | Operation::LoadTxo { .. }
            | Operation::LoadAmount(..)
            | Operation::LoadTxVersion(..)
            | Operation::LoadLockTime(..)
            | Operation::LoadSequence(..)
            | Operation::EndBuildTx
            | Operation::EndBuildTxInputs
            | Operation::EndBuildTxOutputs
            | Operation::AddTxInput
            | Operation::AddTxOutput
            | Operation::TakeTxo
            | Operation::EndWitnessStack
            | Operation::AddWitness
            | Operation::SendTxInv { .. }
            | Operation::SendTx => 0,
        }
    }

    pub fn num_outputs(&self) -> usize {
        match self {
            Operation::Nop { outputs, .. } => *outputs,
            Operation::LoadBytes(_) => 1,
            Operation::LoadMsgType(_) => 1,
            Operation::LoadNode(_) => 1,
            Operation::LoadConnection(_) => 1,
            Operation::LoadConnectionType(_) => 1,
            Operation::LoadDuration(_) => 1,
            Operation::SendRawMessage => 0,
            Operation::AdvanceTime => 1,
            Operation::LoadTime(_) => 1,
            Operation::LoadSize(_) => 1,
            Operation::SetTime => 0,
            Operation::BuildPayToWitnessScriptHash => 1,
            Operation::BuildRawScripts => 1,
            Operation::BuildOpReturnScripts => 1,
            Operation::LoadTxo { .. } => 1,
            Operation::LoadAmount(..) => 1,
            Operation::LoadTxVersion(..) => 1,
            Operation::LoadLockTime(..) => 1,
            Operation::LoadSequence(..) => 1,
            Operation::BeginBuildTx => 0,
            Operation::EndBuildTx => 1,
            Operation::BeginBuildTxInputs => 0,
            Operation::EndBuildTxInputs => 1,
            Operation::BeginBuildTxOutputs => 0,
            Operation::EndBuildTxOutputs => 1,
            Operation::AddTxInput => 0,
            Operation::AddTxOutput => 0,
            Operation::TakeTxo => 1,
            Operation::AddWitness => 0,
            Operation::BeginWitnessStack => 0,
            Operation::EndWitnessStack => 1,

            Operation::SendTxInv { .. } => 0,
            Operation::SendTx => 0,
        }
    }

    pub fn num_inputs(&self) -> usize {
        match self {
            Operation::Nop { .. } => 0,
            Operation::LoadBytes(_) => 0,
            Operation::LoadMsgType(_) => 0,
            Operation::LoadNode(_) => 0,
            Operation::LoadConnection(_) => 0,
            Operation::LoadConnectionType(_) => 0,
            Operation::LoadDuration(_) => 0,
            Operation::SendRawMessage => 3,
            Operation::AdvanceTime => 2,
            Operation::LoadTime(_) => 0,
            Operation::LoadSize(_) => 0,
            Operation::SetTime => 1,
            Operation::BuildPayToWitnessScriptHash => 2,
            Operation::BuildRawScripts => 3,
            Operation::BuildOpReturnScripts => 1,
            Operation::LoadTxo { .. } => 0,
            Operation::LoadAmount(..) => 0,
            Operation::LoadTxVersion(..) => 0,
            Operation::LoadLockTime(..) => 0,
            Operation::LoadSequence(..) => 0,

            Operation::BeginWitnessStack => 0,
            Operation::EndWitnessStack => 1,
            Operation::AddWitness => 2,

            Operation::BeginBuildTx => 2,
            Operation::EndBuildTx => 3,
            Operation::BeginBuildTxInputs => 0,
            Operation::EndBuildTxInputs => 1,
            Operation::BeginBuildTxOutputs => 1,
            Operation::EndBuildTxOutputs => 1,
            Operation::AddTxInput => 3,
            Operation::AddTxOutput => 3,
            Operation::TakeTxo => 1,

            Operation::SendTxInv { .. } => 2,
            Operation::SendTx => 2,
        }
    }

    pub fn check_input_types(&self, variables: &[Variable]) -> Result<(), ProgramValidationError> {
        let check_expected =
            |got: &[Variable], expected: &[Variable]| -> Result<(), ProgramValidationError> {
                assert!(self.num_inputs() == got.len());
                if got.len() != expected.len() {
                    return Err(ProgramValidationError::InvalidNumberOfInputs {
                        is: got.len(),
                        expected: expected.len(),
                    });
                }

                for (got, expected) in got.iter().zip(expected.iter()) {
                    if got != expected {
                        return Err(ProgramValidationError::InvalidVariableType {
                            is: Some(got.clone()),
                            expected: expected.clone(),
                        });
                    }
                }
                Ok(())
            };

        match self {
            Operation::SendRawMessage => check_expected(
                variables,
                &[Variable::Connection, Variable::MsgType, Variable::Bytes],
            ),
            Operation::AdvanceTime => {
                check_expected(variables, &[Variable::Time, Variable::Duration])
            }
            Operation::SetTime => check_expected(variables, &[Variable::Time]),
            Operation::BuildPayToWitnessScriptHash => {
                // Script to be wrapped and additional witness stack
                check_expected(variables, &[Variable::Bytes, Variable::ConstWitnessStack])
            }
            Operation::BuildRawScripts => check_expected(
                variables,
                &[
                    Variable::Bytes,
                    Variable::Bytes,
                    Variable::ConstWitnessStack,
                ],
            ),
            Operation::BuildOpReturnScripts => check_expected(
                variables,
                &[Variable::Size],
            ),
            Operation::BeginBuildTx => {
                check_expected(variables, &[Variable::TxVersion, Variable::LockTime])
            }
            Operation::EndBuildTx => check_expected(
                variables,
                &[
                    Variable::MutTx,
                    Variable::ConstTxInputs,
                    Variable::ConstTxOutputs,
                ],
            ),
            Operation::EndBuildTxInputs => check_expected(variables, &[Variable::MutTxInputs]),
            Operation::EndBuildTxOutputs => check_expected(variables, &[Variable::MutTxOutputs]),
            Operation::AddTxInput => check_expected(
                variables,
                &[Variable::MutTxInputs, Variable::Txo, Variable::Sequence],
            ),
            Operation::AddTxOutput => check_expected(
                variables,
                &[
                    Variable::MutTxOutputs,
                    Variable::Scripts,
                    Variable::ConstAmount,
                ],
            ),
            Operation::BeginBuildTxOutputs => check_expected(variables, &[Variable::ConstTxInputs]),
            Operation::TakeTxo => check_expected(variables, &[Variable::ConstTx]),
            Operation::AddWitness => {
                check_expected(variables, &[Variable::MutWitnessStack, Variable::Bytes])
            }
            Operation::EndWitnessStack => check_expected(variables, &[Variable::MutWitnessStack]),
            Operation::SendTxInv { .. } => {
                check_expected(variables, &[Variable::Connection, Variable::ConstTx])
            }
            Operation::SendTx => {
                check_expected(variables, &[Variable::Connection, Variable::ConstTx])
            }
            // Exhaustive match to fail when new ops are added
            Operation::Nop { .. }
            | Operation::LoadBytes(_)
            | Operation::LoadMsgType(_)
            | Operation::LoadNode(_)
            | Operation::LoadConnection(_)
            | Operation::LoadConnectionType(_)
            | Operation::LoadDuration(_)
            | Operation::LoadTime(_)
            | Operation::LoadTxo { .. }
            | Operation::LoadAmount(..)
            | Operation::LoadTxVersion(..)
            | Operation::LoadLockTime(..)
            | Operation::LoadSequence(..)
            | Operation::LoadSize(_)
            | Operation::BeginBuildTxInputs
            | Operation::BeginWitnessStack => Ok(()),
        }
    }

    pub fn get_output_variables(&self) -> Vec<Variable> {
        match self {
            Operation::LoadBytes(_) => vec![Variable::Bytes],
            Operation::LoadMsgType(_) => vec![Variable::MsgType],
            Operation::LoadNode(_) => vec![Variable::Node],
            Operation::LoadConnection(_) => vec![Variable::Connection],
            Operation::LoadConnectionType(_) => vec![Variable::ConnectionType],
            Operation::LoadDuration(_) => vec![Variable::Duration],
            Operation::SendRawMessage => vec![],
            Operation::AdvanceTime => vec![Variable::Time],
            Operation::LoadTime(_) => vec![Variable::Time],
            Operation::SetTime => vec![],
            Operation::Nop { outputs, .. } => vec![Variable::Nop; *outputs],
            Operation::BuildPayToWitnessScriptHash => vec![Variable::Scripts],
            Operation::BuildRawScripts => vec![Variable::Scripts],
            Operation::BuildOpReturnScripts => vec![Variable::Scripts],
            Operation::LoadTxo { .. } => vec![Variable::Txo],
            Operation::LoadAmount(..) => vec![Variable::ConstAmount],
            Operation::LoadTxVersion(..) => vec![Variable::TxVersion],
            Operation::LoadLockTime(..) => vec![Variable::LockTime],
            Operation::LoadSequence(..) => vec![Variable::Sequence],
            Operation::LoadSize(..) => vec![Variable::Size],
            Operation::TakeTxo => vec![Variable::Txo],
            Operation::BeginBuildTx => vec![],
            Operation::EndBuildTx => vec![Variable::ConstTx],
            Operation::BeginBuildTxInputs => vec![],
            Operation::EndBuildTxInputs => vec![Variable::ConstTxInputs],
            Operation::BeginBuildTxOutputs => vec![],
            Operation::EndBuildTxOutputs => vec![Variable::ConstTxOutputs],
            Operation::AddTxInput => vec![],
            Operation::AddTxOutput => vec![],

            Operation::BeginWitnessStack => vec![],
            Operation::EndWitnessStack => vec![Variable::ConstWitnessStack],
            Operation::AddWitness => vec![],

            Operation::SendTxInv { .. } => vec![],
            Operation::SendTx => vec![],
        }
    }

    pub fn get_inner_output_variables(&self) -> Vec<Variable> {
        match self {
            Operation::BeginBuildTx => vec![Variable::MutTx],
            Operation::BeginBuildTxInputs => vec![Variable::MutTxInputs],
            Operation::BeginBuildTxOutputs => vec![Variable::MutTxOutputs],
            Operation::BeginWitnessStack => vec![Variable::MutWitnessStack],
            Operation::Nop {
                outputs: _,
                inner_outputs,
            } => vec![Variable::Nop; *inner_outputs],
            // Exhaustive match to fail when new ops are added
            Operation::LoadBytes(_)
            | Operation::LoadMsgType(_)
            | Operation::LoadNode(_)
            | Operation::LoadConnection(_)
            | Operation::LoadConnectionType(_)
            | Operation::LoadDuration(_)
            | Operation::SendRawMessage
            | Operation::AdvanceTime
            | Operation::LoadTime(_)
            | Operation::SetTime
            | Operation::BuildPayToWitnessScriptHash
            | Operation::BuildRawScripts
            | Operation::BuildOpReturnScripts
            | Operation::LoadTxo { .. }
            | Operation::LoadAmount(..)
            | Operation::LoadTxVersion(..)
            | Operation::LoadLockTime(..)
            | Operation::LoadSequence(..)
            | Operation::LoadSize(..)
            | Operation::EndBuildTx
            | Operation::EndBuildTxInputs
            | Operation::EndBuildTxOutputs
            | Operation::AddTxInput
            | Operation::AddTxOutput
            | Operation::TakeTxo
            | Operation::EndWitnessStack
            | Operation::AddWitness
            | Operation::SendTxInv { .. }
            | Operation::SendTx => vec![],
        }
    }
}

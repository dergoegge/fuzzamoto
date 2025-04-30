use std::{any::Any, time::Duration};

use bitcoin::{
    Amount, OutPoint, Script, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid,
    absolute::LockTime,
    hashes::{Hash, serde_macros::serde_details::SerdeHash, sha256},
    opcodes::{OP_0, all::OP_RETURN},
    p2p::message_blockdata::Inventory,
    script::PushBytesBuf,
    transaction::Version,
};

use crate::{Operation, Program};

/// `Compiler` is responsible for compiling IR into a sequence of low-level actions to be performed
/// on a node (i.e. mapping `fuzzamoto_ir::Program` -> `CompiledProgram`).
pub struct Compiler;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum CompiledAction {
    /// Create a new connection
    Connect(usize, String),
    /// Send a message on one of the connections
    SendRawMessage(usize, String, Vec<u8>),
    /// Set mock time for all nodes in the test
    SetTime(u64),
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct CompiledProgram {
    pub actions: Vec<CompiledAction>,
}

#[derive(Debug)]
pub enum CompilerError {
    MiscError(String),
    IncorrectNumberOfInputs,
    VariableNotFound,
    IncorrectVariableType,
}

pub type CompilerResult = Result<CompiledProgram, CompilerError>;

struct Node {
    _index: usize,
}

struct Connection {
    index: usize,
}

#[derive(Clone, Debug)]
struct Scripts {
    script_pubkey: Vec<u8>,
    script_sig: Vec<u8>,
    witness: Witness,
}

#[derive(Debug, Clone)]
struct Witness {
    stack: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct Txo {
    prev_out: ([u8; 32], u32),
    scripts: Scripts,
    value: u64,
}

#[derive(Clone)]
struct TxOutputs {
    outputs: Vec<(Scripts, u64)>,
    fees: u64,
}

#[derive(Clone)]
struct TxInputs {
    inputs: Vec<TxIn>,
    total_value: u64,
}

#[derive(Clone, Debug)]
struct Tx {
    tx: Transaction,
    txos: Vec<Txo>,
    output_selector: usize,
}

struct Nop;

impl Compiler {
    pub fn compile(&self, ir: &Program) -> CompilerResult {
        let mut output = CompiledProgram {
            actions: Vec::new(),
        };

        let mut variables: Vec<Box<dyn Any>> = Vec::new();

        for instruction in &ir.instructions {
            match instruction.operation.clone() {
                Operation::Nop {
                    outputs,
                    inner_outputs,
                } => {
                    for _ in 0..outputs {
                        variables.push(Box::new(Nop));
                    }
                    for _ in 0..inner_outputs {
                        variables.push(Box::new(Nop));
                    }
                }
                Operation::LoadNode(index) => {
                    variables.push(Box::new(Node { _index: index }));
                }
                Operation::LoadConnection(index) => {
                    variables.push(Box::new(Connection { index }));
                }
                Operation::LoadConnectionType(connection_type) => {
                    variables.push(Box::new(connection_type));
                }
                Operation::LoadDuration(duration) => {
                    variables.push(Box::new(duration));
                }
                Operation::LoadAmount(amount) => {
                    variables.push(Box::new(amount));
                }
                Operation::LoadTxVersion(version) => {
                    variables.push(Box::new(version));
                }
                Operation::LoadLockTime(lock_time) => {
                    variables.push(Box::new(lock_time));
                }
                Operation::LoadSequence(sequence) => {
                    variables.push(Box::new(sequence));
                }
                Operation::LoadTime(time) => {
                    variables.push(Box::new(time));
                }
                Operation::LoadMsgType(message_type) => {
                    variables.push(Box::new(message_type));
                }
                Operation::LoadBytes(bytes) => {
                    variables.push(Box::new(bytes));
                }
                Operation::LoadSize(size) => {
                    variables.push(Box::new(size));
                }
                Operation::LoadTxo {
                    outpoint,
                    value,
                    script_pubkey,
                    spending_script_sig,
                    spending_witness,
                } => {
                    variables.push(Box::new(Txo {
                        prev_out: outpoint,
                        value,
                        scripts: Scripts {
                            script_pubkey,
                            script_sig: spending_script_sig,
                            witness: Witness {
                                stack: spending_witness,
                            },
                        },
                    }));
                }

                Operation::BeginWitnessStack => {
                    variables.push(Box::new(Witness { stack: Vec::new() }));
                }
                Operation::AddWitness => {
                    let bytes_var =
                        get_nth_variable::<Vec<u8>>(&variables, &instruction.inputs, 1)?.clone();
                    let witness_var =
                        get_nth_variable_mut::<Witness>(&mut variables, &instruction.inputs, 0)?;

                    witness_var.stack.push(bytes_var);
                }
                Operation::EndWitnessStack => {
                    let witness_var =
                        get_nth_variable::<Witness>(&variables, &instruction.inputs, 0)?;
                    variables.push(Box::new(witness_var.clone()));
                }

                Operation::BuildPayToWitnessScriptHash => {
                    let script = get_nth_variable::<Vec<u8>>(&variables, &instruction.inputs, 0)?;
                    let witness_var =
                        get_nth_variable::<Witness>(&variables, &instruction.inputs, 1)?;

                    let mut witness = witness_var.clone();
                    witness.stack.push(script.clone());

                    // OP_0 0x20 <script hash>
                    let mut script_pubkey = vec![OP_0.to_u8(), 32];
                    let script_hash = sha256::Hash::hash(script.as_slice());
                    script_pubkey.extend(script_hash.as_byte_array().as_slice());

                    variables.push(Box::new(Scripts {
                        script_pubkey,
                        script_sig: vec![],
                        witness,
                    }));
                }

                Operation::BuildRawScripts => {
                    let script_pubkey_var =
                        get_nth_variable::<Vec<u8>>(&variables, &instruction.inputs, 0)?;
                    let script_sig_var =
                        get_nth_variable::<Vec<u8>>(&variables, &instruction.inputs, 1)?;
                    let witness_var =
                        get_nth_variable::<Witness>(&variables, &instruction.inputs, 2)?;

                    let script_pubkey = script_pubkey_var.clone();
                    let script_sig = script_sig_var.clone();
                    let witness = witness_var.clone();

                    variables.push(Box::new(Scripts {
                        script_pubkey,
                        script_sig,
                        witness,
                    }));
                }

                Operation::BuildOpReturnScripts => {
                    let size_var = get_nth_variable::<usize>(&variables, &instruction.inputs, 0)?;

                    let data = vec![0x41u8; *size_var];
                    let script = ScriptBuf::builder()
                        .push_opcode(OP_RETURN)
                        .push_slice(&PushBytesBuf::try_from(data).unwrap());

                    variables.push(Box::new(Scripts {
                        script_pubkey: script.into_bytes(),
                        script_sig: vec![],
                        witness: Witness { stack: Vec::new() },
                    }));
                }

                Operation::BeginBuildTx => {
                    let tx_version_var =
                        get_nth_variable::<u32>(&variables, &instruction.inputs, 0)?;
                    let tx_lock_time_var =
                        get_nth_variable::<u32>(&variables, &instruction.inputs, 1)?;

                    variables.push(Box::new(Tx {
                        tx: Transaction {
                            version: Version(*tx_version_var as i32),
                            lock_time: LockTime::from_consensus(*tx_lock_time_var),
                            input: Vec::new(),
                            output: Vec::new(),
                        },
                        txos: Vec::new(),
                        output_selector: 0,
                    }));
                }
                Operation::EndBuildTx => {
                    let mut tx_inputs_var =
                        get_nth_variable::<TxInputs>(&variables, &instruction.inputs, 1)?.clone();
                    let tx_outputs_var =
                        get_nth_variable::<TxOutputs>(&variables, &instruction.inputs, 2)?.clone();

                    let mut tx_var =
                        get_nth_variable_mut::<Tx>(&mut variables, &instruction.inputs, 0)?.clone();

                    tx_var.tx.input.extend(tx_inputs_var.inputs.drain(..));
                    tx_var.tx.output.extend(tx_outputs_var.outputs.iter().map(
                        |(scripts, amount)| TxOut {
                            value: Amount::from_sat(*amount),
                            script_pubkey:
                                Script::from_bytes(scripts.script_pubkey.as_slice()).into(),
                        },
                    ));

                    let mut hash = [0u8; 32];
                    hash.copy_from_slice(
                        tx_var
                            .tx
                            .compute_txid()
                            .as_raw_hash()
                            .as_byte_array()
                            .as_slice(),
                    );

                    tx_var.txos = tx_outputs_var
                        .outputs
                        .iter()
                        .enumerate()
                        .map(|(index, (scripts, amount))| Txo {
                            prev_out: (hash, index as u32),
                            scripts: scripts.clone(),
                            value: *amount,
                        })
                        .collect();

                    variables.push(Box::new(tx_var));
                }

                Operation::BeginBuildTxInputs => {
                    variables.push(Box::new(TxInputs {
                        inputs: Vec::new(),
                        total_value: 0,
                    }));
                }
                Operation::EndBuildTxInputs => {
                    let tx_inputs_var =
                        get_nth_variable::<TxInputs>(&variables, &instruction.inputs, 0)?;
                    variables.push(Box::new(tx_inputs_var.clone()));
                }
                Operation::AddTxInput => {
                    let txo_var = get_nth_variable::<Txo>(&variables, &instruction.inputs, 1)?;
                    let sequence_var = get_nth_variable::<u32>(&variables, &instruction.inputs, 2)?;

                    let previous_output = OutPoint::new(
                        Txid::from_slice_delegated(&txo_var.prev_out.0).unwrap(),
                        txo_var.prev_out.1,
                    );
                    let script_sig = Script::from_bytes(&txo_var.scripts.script_sig).into();
                    let witness = bitcoin::Witness::from(txo_var.scripts.witness.stack.as_slice());
                    let value = txo_var.value;
                    let sequence = *sequence_var;

                    let mut_tx_inputs_var =
                        get_nth_variable_mut::<TxInputs>(&mut variables, &instruction.inputs, 0)?;

                    mut_tx_inputs_var.inputs.push(TxIn {
                        previous_output,
                        script_sig,
                        witness,
                        sequence: Sequence(sequence),
                    });
                    mut_tx_inputs_var.total_value += value;
                }

                Operation::BeginBuildTxOutputs => {
                    let tx_inputs_var =
                        get_nth_variable::<TxInputs>(&variables, &instruction.inputs, 0)?;
                    let fees = tx_inputs_var.total_value;
                    variables.push(Box::new(TxOutputs {
                        outputs: Vec::new(),
                        fees,
                    }));
                }
                Operation::EndBuildTxOutputs => {
                    let tx_outputs_var =
                        get_nth_variable_mut::<TxOutputs>(&mut variables, &instruction.inputs, 0)?
                            .clone();
                    variables.push(Box::new(tx_outputs_var));
                }

                Operation::AddTxOutput => {
                    let scripts =
                        get_nth_variable::<Scripts>(&variables, &instruction.inputs, 1)?.clone();
                    let amount =
                        get_nth_variable::<u64>(&variables, &instruction.inputs, 2)?.clone();

                    let mut_tx_outputs_var =
                        get_nth_variable_mut::<TxOutputs>(&mut variables, &instruction.inputs, 0)?;

                    let amount = amount.min(mut_tx_outputs_var.fees);
                    mut_tx_outputs_var.outputs.push((scripts, amount));
                    mut_tx_outputs_var.fees -= amount;
                }

                Operation::AdvanceTime => {
                    let time_var = get_nth_variable::<u64>(&variables, &instruction.inputs, 0)?;
                    let duration_var =
                        get_nth_variable::<Duration>(&variables, &instruction.inputs, 1)?;

                    variables.push(Box::new(*time_var + duration_var.as_secs()));
                }

                Operation::SetTime => {
                    let time_var = get_nth_variable::<u64>(&variables, &instruction.inputs, 0)?;
                    output.actions.push(CompiledAction::SetTime(*time_var));
                }

                Operation::TakeTxo => {
                    let txo = {
                        let tx_var =
                            get_nth_variable_mut::<Tx>(&mut variables, &instruction.inputs, 0)?;
                        tx_var.output_selector += 1;
                        tx_var.txos[tx_var.output_selector - 1].clone()
                    };

                    variables.push(Box::new(txo));
                }

                Operation::SendRawMessage => {
                    let connection_var =
                        get_nth_variable::<Connection>(&variables, &instruction.inputs, 0)?;
                    let message_type_var =
                        get_nth_variable::<[char; 12]>(&variables, &instruction.inputs, 1)?;
                    let bytes_var =
                        get_nth_variable::<Vec<u8>>(&variables, &instruction.inputs, 2)?;

                    output.actions.push(CompiledAction::SendRawMessage(
                        connection_var.index,
                        message_type_var.iter().collect(),
                        bytes_var.clone(),
                    ));
                }

                Operation::SendTx => {
                    let connection_var =
                        get_nth_variable::<Connection>(&variables, &instruction.inputs, 0)?;
                    let tx_var = get_nth_variable::<Tx>(&variables, &instruction.inputs, 1)?;

                    output.actions.push(CompiledAction::SendRawMessage(
                        connection_var.index,
                        "tx".to_string(),
                        bitcoin::consensus::encode::serialize(&tx_var.tx),
                    ));
                }
                Operation::SendTxInv { wtxid } => {
                    let connection_var =
                        get_nth_variable::<Connection>(&variables, &instruction.inputs, 0)?;
                    let tx_var = get_nth_variable::<Tx>(&variables, &instruction.inputs, 1)?;

                    let inv = if wtxid {
                        Inventory::WTx(tx_var.tx.compute_wtxid())
                    } else {
                        Inventory::Transaction(tx_var.tx.compute_txid())
                    };

                    output.actions.push(CompiledAction::SendRawMessage(
                        connection_var.index,
                        "inv".to_string(),
                        bitcoin::consensus::encode::serialize(&vec![inv]),
                    ));
                }
            }
        }

        Ok(output)
    }

    pub fn new() -> Self {
        Self {}
    }
}

fn get_nth_variable<'a, T: 'static>(
    variables: &'a Vec<Box<dyn Any>>,
    inputs: &[usize],
    index: usize,
) -> Result<&'a T, CompilerError> {
    let var_index = inputs
        .get(index)
        .ok_or(CompilerError::IncorrectNumberOfInputs)?;
    let var = variables
        .get(*var_index)
        .ok_or(CompilerError::VariableNotFound)?;
    let var = var
        .downcast_ref::<T>()
        .ok_or(CompilerError::IncorrectVariableType)?;
    Ok(var)
}

fn get_nth_variable_mut<'a, T: 'static>(
    variables: &'a mut Vec<Box<dyn Any>>,
    inputs: &[usize],
    index: usize,
) -> Result<&'a mut T, CompilerError> {
    let var_index = inputs
        .get(index)
        .ok_or(CompilerError::IncorrectNumberOfInputs)?;
    let var = variables
        .get_mut(*var_index)
        .ok_or(CompilerError::VariableNotFound)?;
    let var = var
        .downcast_mut::<T>()
        .ok_or(CompilerError::IncorrectVariableType)?;
    Ok(var)
}

use crate::ProbeOperation;
use bitcoin::{
    Amount, CompactTarget, EcdsaSighashType, NetworkKind, OutPoint, PrivateKey, Script, ScriptBuf,
    Sequence, Transaction, TxIn, TxMerkleNode, TxOut, Txid, WitnessMerkleNode, Wtxid,
    absolute::LockTime,
    bip152::{BlockTransactions, BlockTransactionsRequest, HeaderAndShortIds},
    consensus::Encodable,
    ecdsa,
    hashes::{Hash, serde_macros::serde_details::SerdeHash, sha256},
    key::Secp256k1,
    opcodes::{OP_0, OP_TRUE, all::OP_RETURN},
    p2p::{
        message_blockdata::Inventory,
        message_bloom::{BloomFlags, FilterAdd, FilterLoad},
        message_compact_blocks::CmpctBlock,
        message_filter::{GetCFCheckpt, GetCFHeaders, GetCFilters},
    },
    script::PushBytesBuf,
    secp256k1::{self, SecretKey},
    sighash::SighashCache,
    transaction,
};
use std::{any::Any, time::Duration};

use crate::{
    Instruction, Operation, Program,
    bloom::filter_insert,
    generators::{block::Header, compact_block::BlockTransactionsRequestRecved},
};

/// `Compiler` is responsible for compiling IR into a sequence of low-level actions to be performed
/// on a node (i.e. mapping `fuzzamoto_ir::Program` -> `CompiledProgram`).
pub struct Compiler {
    secp_ctx: Secp256k1<bitcoin::secp256k1::All>,

    variables: Vec<Box<dyn Any>>,
    output: CompiledProgram,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum CompiledAction {
    /// Create a new connection
    Connect(usize, String),
    /// Send a message on one of the connections
    SendRawMessage(usize, String, Vec<u8>),
    /// Set mock time for all nodes in the test
    SetTime(u64),
    /// Enable message logging from a connection
    EnableLogging(),
    /// Disable message logging from a connection
    DisableLogging(),
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

impl std::fmt::Display for CompilerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompilerError::MiscError(e) => write!(f, "Misc error: {}", e),
            CompilerError::IncorrectNumberOfInputs => write!(f, "Incorrect number of inputs"),
            CompilerError::VariableNotFound => write!(f, "Variable not found"),
            CompilerError::IncorrectVariableType => write!(f, "Incorrect variable type"),
        }
    }
}

pub type CompilerResult = Result<CompiledProgram, CompilerError>;

#[derive(Clone, Debug)]
struct Scripts {
    script_pubkey: Vec<u8>,
    script_sig: Vec<u8>,
    witness: Witness,

    // op & vars
    requires_signing: Option<(Operation, Vec<usize>)>,
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

impl Txo {
    pub fn new() -> Self {
        Self {
            prev_out: ([0u8; 32], 0),
            scripts: Scripts {
                script_pubkey: Vec::new(),
                script_sig: Vec::new(),
                witness: Witness { stack: Vec::new() },
                requires_signing: None,
            },
            value: 0,
        }
    }
}

#[derive(Clone)]
struct TxOutputs {
    outputs: Vec<(Scripts, u64)>,
    fees: u64,
}

#[derive(Clone)]
struct TxInput {
    txo_var: usize,
    sequence_var: usize,
}

#[derive(Clone)]
struct TxInputs {
    inputs: Vec<TxInput>,
    total_value: u64,
}

#[derive(Clone, Debug)]
struct Tx {
    tx: Transaction,
    txos: Vec<Txo>,
    output_selector: usize,
    id: Txid,
}

#[derive(Clone)]
struct CoinbaseInput {
    sequence: usize,
    total_value: u64,
}

#[derive(Clone)]
struct CoinbaseTx {
    tx: Tx,
    scripts: Vec<Scripts>,
}

struct Nop;

impl Compiler {
    pub fn compile(&mut self, ir: &Program) -> CompilerResult {
        for instruction in &ir.instructions {
            match instruction.operation.clone() {
                Operation::Nop { .. }
                | Operation::LoadNode(..)
                | Operation::LoadConnection(..)
                | Operation::LoadConnectionType(..)
                | Operation::LoadDuration(..)
                | Operation::LoadAmount(..)
                | Operation::LoadTxVersion(..)
                | Operation::LoadBlockVersion(..)
                | Operation::LoadLockTime(..)
                | Operation::LoadSequence(..)
                | Operation::LoadTime(..)
                | Operation::LoadBlockHeight(..)
                | Operation::LoadCompactFilterType(..)
                | Operation::LoadMsgType(..)
                | Operation::LoadBytes(..)
                | Operation::LoadSize(..)
                | Operation::LoadPrivateKey(..)
                | Operation::LoadSigHashFlags(..)
                | Operation::LoadHeader { .. }
                | Operation::LoadTxo { .. }
                | Operation::LoadFilterLoad { .. }
                | Operation::LoadFilterAdd { .. }
                | Operation::LoadNonce(..)
                | Operation::LoadTxIndexes { .. }
                | Operation::LoadBlockTxnRequestVec { .. } => {
                    self.handle_load_operations(&instruction)?;
                }

                Operation::BeginBuildCmpctBlock | Operation::EndBuildCmpctBlock => {
                    self.handle_cmpct_block_building_operations(&instruction)?;
                }

                Operation::BeginBlockTransactions
                | Operation::AddTx
                | Operation::EndBlockTransactions
                | Operation::BuildBlock => {
                    self.handle_block_building_operations(&instruction)?;
                }

                Operation::BeginBuildInventory
                | Operation::EndBuildInventory
                | Operation::AddTxidWithWitnessInv
                | Operation::AddWtxidInv
                | Operation::AddTxidInv
                | Operation::AddCompactBlockInv
                | Operation::AddBlockInv
                | Operation::AddBlockWithWitnessInv
                | Operation::AddFilteredBlockInv => {
                    self.handle_inventory_operations(&instruction)?;
                }

                Operation::BeginWitnessStack
                | Operation::AddWitness
                | Operation::EndWitnessStack => {
                    self.handle_witness_operations(&instruction)?;
                }

                Operation::BuildPayToWitnessScriptHash
                | Operation::BuildPayToScriptHash
                | Operation::BuildPayToAnchor
                | Operation::BuildRawScripts
                | Operation::BuildOpReturnScripts
                | Operation::BuildPayToPubKey
                | Operation::BuildPayToPubKeyHash
                | Operation::BuildPayToWitnessPubKeyHash => {
                    self.handle_script_building_operations(&instruction)?;
                }

                Operation::BuildBIP152BlockTxReqFromMetadata | Operation::BuildBIP152BlockTxReq => {
                    self.handle_bip152_blocktx_operations(&instruction)?;
                }

                Operation::BuildFilterAddFromTx
                | Operation::BuildFilterAddFromTxo
                | Operation::AddTxToFilter
                | Operation::AddTxoToFilter
                | Operation::BeginBuildFilterLoad
                | Operation::EndBuildFilterLoad => {
                    self.handle_filter_building_operations(&instruction)?;
                }

                Operation::BeginBuildTx
                | Operation::EndBuildTx
                | Operation::BeginBuildTxInputs
                | Operation::EndBuildTxInputs
                | Operation::AddTxInput
                | Operation::BeginBuildTxOutputs
                | Operation::EndBuildTxOutputs
                | Operation::AddTxOutput
                | Operation::TakeTxo => {
                    self.handle_transaction_building_operations(&instruction)?;
                }

                Operation::BeginBuildCoinbaseTx
                | Operation::EndBuildCoinbaseTx
                | Operation::BuildCoinbaseTxInput
                | Operation::BeginBuildCoinbaseTxOutputs
                | Operation::EndBuildCoinbaseTxOutputs
                | Operation::AddCoinbaseTxOutput => {
                    self.handle_coinbase_building_operations(&instruction)?;
                }

                Operation::AdvanceTime | Operation::SetTime => {
                    self.handle_time_operations(&instruction)?;
                }

                Operation::SendRawMessage
                | Operation::SendTxNoWit
                | Operation::SendTx
                | Operation::SendGetData
                | Operation::SendInv
                | Operation::SendHeader
                | Operation::SendBlock
                | Operation::SendBlockNoWit
                | Operation::SendGetCFilters
                | Operation::SendGetCFHeaders
                | Operation::SendGetCFCheckpt
                | Operation::SendFilterLoad
                | Operation::SendFilterAdd
                | Operation::SendFilterClear
                | Operation::SendCompactBlock
                | Operation::SendBlockTxn => {
                    self.handle_message_sending_operations(&instruction)?;
                }

                Operation::Probe(_) => {
                    self.handle_probe_operations(&instruction)?;
                }
            }
        }

        Ok(self.output.clone()) // TODO: do not clone
    }

    pub fn new() -> Self {
        Self {
            // TODO: make this deterministic
            secp_ctx: Secp256k1::new(),
            variables: Vec::with_capacity(4096),
            output: CompiledProgram {
                actions: Vec::with_capacity(4096),
            },
        }
    }

    fn handle_load_operation<T: 'static>(&mut self, value: T) {
        self.append_variable(value);
    }

    fn handle_inventory_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BeginBuildInventory => {
                self.append_variable(Vec::<Inventory>::new());
            }
            Operation::EndBuildInventory => {
                let bytes_var = self
                    .get_input::<Vec<Inventory>>(&instruction.inputs, 0)?
                    .clone();
                self.append_variable(bytes_var.clone());
            }
            Operation::AddTxidWithWitnessInv => {
                let tx_var = self.get_input::<Tx>(&instruction.inputs, 1)?;
                let inv = Inventory::WitnessTransaction(tx_var.tx.compute_txid());
                let inventory_var = self.get_input_mut::<Vec<Inventory>>(&instruction.inputs, 0)?;
                inventory_var.push(inv);
            }
            Operation::AddWtxidInv => {
                let tx_var = self.get_input::<Tx>(&instruction.inputs, 1)?;
                let inv = Inventory::WTx(tx_var.tx.compute_wtxid());
                let inventory_var = self.get_input_mut::<Vec<Inventory>>(&instruction.inputs, 0)?;
                inventory_var.push(inv);
            }
            Operation::AddTxidInv => {
                let tx_var = self.get_input::<Tx>(&instruction.inputs, 1)?;
                let inv = Inventory::Transaction(tx_var.tx.compute_txid());
                let inventory_var = self.get_input_mut::<Vec<Inventory>>(&instruction.inputs, 0)?;
                inventory_var.push(inv);
            }
            Operation::AddCompactBlockInv => {
                let block_var = self.get_input::<bitcoin::Block>(&instruction.inputs, 1)?;
                let inv = Inventory::CompactBlock(block_var.header.block_hash());
                let inventory_var = self.get_input_mut::<Vec<Inventory>>(&instruction.inputs, 0)?;
                inventory_var.push(inv);
            }
            Operation::AddBlockInv => {
                let block_var = self.get_input::<bitcoin::Block>(&instruction.inputs, 1)?;
                let inv = Inventory::Block(block_var.header.block_hash());
                let inventory_var = self.get_input_mut::<Vec<Inventory>>(&instruction.inputs, 0)?;
                inventory_var.push(inv);
            }
            Operation::AddBlockWithWitnessInv => {
                let block_var = self.get_input::<bitcoin::Block>(&instruction.inputs, 1)?;
                let inv = Inventory::WitnessBlock(block_var.header.block_hash());
                let inventory_var = self.get_input_mut::<Vec<Inventory>>(&instruction.inputs, 0)?;
                inventory_var.push(inv);
            }
            Operation::AddFilteredBlockInv => {
                let block_var = self.get_input::<bitcoin::Block>(&instruction.inputs, 1)?;
                let inv = Inventory::Unknown {
                    inv_type: 3, // MSG_FILTERED_BLOCK, see Bitcoin Core
                    hash: *block_var.header.block_hash().as_byte_array(),
                };
                let inventory_var = self.get_input_mut::<Vec<Inventory>>(&instruction.inputs, 0)?;
                inventory_var.push(inv);
            }
            _ => unreachable!("Non-inventory operation passed to handle_inventory_operations"),
        }
        Ok(())
    }

    fn handle_bip152_blocktx_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BuildBIP152BlockTxReqFromMetadata => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?.clone();
                let reqs = self
                    .get_input_mut::<Vec<BlockTransactionsRequestRecved>>(&instruction.inputs, 2)?
                    .clone();
                let block = self.get_input::<bitcoin::Block>(&instruction.inputs, 1)?;

                let mut blocktxn_req = BlockTransactionsRequest {
                    block_hash: block.block_hash(),
                    indexes: vec![],
                };
                /*
                log::info!(
                    "connection_var {} block_hash {:?}",
                    connection_var,
                    block.block_hash().as_raw_hash().as_byte_array()
                );
                */
                for req in reqs {
                    // log::info!("{} {:?}", req.conn(), req.hash());
                    if connection_var == req.conn()
                        && block.block_hash().as_raw_hash().as_byte_array() == req.hash()
                    {
                        // bingo we send this
                        blocktxn_req.indexes.extend(req.indexes());
                    }
                }
                let max = block.txdata.len() as u64;
                let mut filtered_indexes: Vec<u64> = blocktxn_req
                    .indexes
                    .iter()
                    .copied()
                    .filter(|&i| i < max)
                    .collect();
                filtered_indexes.sort();
                filtered_indexes.dedup();
                blocktxn_req.indexes = filtered_indexes;

                let blocktxn = BlockTransactions::from_request(&blocktxn_req, block)
                    .expect("from_request should never fail");

                self.append_variable(blocktxn);
            }
            Operation::BuildBIP152BlockTxReq => {
                let block = self.get_input::<bitcoin::Block>(&instruction.inputs, 0)?;
                let txindexes = self.get_input::<Vec<usize>>(&instruction.inputs, 1)?;

                let mut blocktxn_req = BlockTransactionsRequest {
                    block_hash: block.block_hash(),
                    indexes: txindexes.iter().map(|&x| x as u64).collect(),
                };

                let max = block.txdata.len() as u64;
                let mut filtered_indexes: Vec<u64> = blocktxn_req
                    .indexes
                    .iter()
                    .copied()
                    .filter(|&i| i < max)
                    .collect();
                filtered_indexes.sort();
                filtered_indexes.dedup();
                blocktxn_req.indexes = filtered_indexes;
                let blocktxn = BlockTransactions::from_request(&blocktxn_req, block)
                    .expect("from_request should never fail");

                // log::info!("blocktxn_req {:?}", blocktxn_req);
                self.append_variable(blocktxn);
            }
            _ => unreachable!("Non-bip152 blocktx operation passed to handle_witness_operations"),
        }
        Ok(())
    }

    fn handle_witness_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BeginWitnessStack => {
                self.append_variable(Witness { stack: Vec::new() });
            }
            Operation::AddWitness => {
                let bytes_var = self.get_input::<Vec<u8>>(&instruction.inputs, 1)?.clone();
                let witness_var = self.get_input_mut::<Witness>(&instruction.inputs, 0)?;
                witness_var.stack.push(bytes_var);
            }
            Operation::EndWitnessStack => {
                let witness_var = self.get_input::<Witness>(&instruction.inputs, 0)?;
                self.append_variable(witness_var.clone());
            }
            _ => unreachable!("Non-witness operation passed to handle_witness_operations"),
        }
        Ok(())
    }

    fn handle_filter_building_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BeginBuildFilterLoad => {
                let filter = self
                    .get_input::<FilterLoad>(&instruction.inputs, 0)?
                    .clone();
                self.append_variable(filter);
            }
            Operation::AddTxToFilter => {
                let tx_as_array = self
                    .get_input::<Tx>(&instruction.inputs, 1)?
                    .id
                    .as_raw_hash()
                    .as_byte_array()
                    .to_vec();
                let mut_filter = self.get_input_mut::<FilterLoad>(&instruction.inputs, 0)?;
                let n_hash_funcs = mut_filter.hash_funcs;
                filter_insert(&mut mut_filter.filter, n_hash_funcs, &tx_as_array);
            }
            Operation::AddTxoToFilter => {
                let txo = self.get_input::<Txo>(&instruction.inputs, 1)?.prev_out.0;
                let mut_filter = self.get_input_mut::<FilterLoad>(&instruction.inputs, 0)?;
                let n_hash_funcs = mut_filter.hash_funcs;
                filter_insert(&mut mut_filter.filter, n_hash_funcs, &txo);
            }
            Operation::EndBuildFilterLoad => {
                let filter = self
                    .get_input::<FilterLoad>(&instruction.inputs, 0)?
                    .clone();
                self.append_variable(filter);
            }
            Operation::BuildFilterAddFromTx => {
                let tx = self.get_input::<Tx>(&instruction.inputs, 0)?;

                let filteradd = FilterAdd {
                    data: tx.id.as_raw_hash().as_byte_array().to_vec(),
                };
                self.append_variable(filteradd);
            }
            Operation::BuildFilterAddFromTxo => {
                let txo = self.get_input::<Txo>(&instruction.inputs, 0)?;

                let filteradd = FilterAdd {
                    data: txo.scripts.script_pubkey.clone(),
                };

                self.append_variable(filteradd);
            }
            _ => unreachable!(
                "Non-filter-building operation passed to handle_filter_building_operations"
            ),
        }
        Ok(())
    }

    fn handle_cmpct_block_building_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BeginBuildCmpctBlock => {
                let block = self.get_input::<bitcoin::Block>(&instruction.inputs, 0)?;
                let nonce = self.get_input::<u64>(&instruction.inputs, 1)?;
                let version = self.get_input::<u32>(&instruction.inputs, 2)?;
                let mut prefill = self
                    .get_input::<Vec<usize>>(&instruction.inputs, 3)?
                    .clone();

                // OperationMutator could destory our constraint. so we clamp here.
                let version = version.clamp(&1, &2);
                prefill.sort();
                prefill.dedup();
                let max = block.txdata.len();
                let prefill_filtered = prefill
                    .iter()
                    .copied()
                    .filter(|&i| i < max)
                    .collect::<Vec<usize>>();
                let header_and_shortids =
                    HeaderAndShortIds::from_block(block, *nonce, *version, &prefill_filtered)
                        .expect("from_block should never fail");
                self.append_variable(CmpctBlock {
                    compact_block: header_and_shortids,
                });
            }
            Operation::EndBuildCmpctBlock => {
                let cmpct_block = self
                    .get_input_mut::<CmpctBlock>(&instruction.inputs, 0)?
                    .clone();
                self.append_variable(cmpct_block);
            }
            _ => unreachable!(
                "Non-cmpctblock-building operation passed to handle_cmpct_block_building_operations"
            ),
        }
        Ok(())
    }

    fn handle_script_building_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BuildPayToWitnessScriptHash => {
                let script = self.get_input::<Vec<u8>>(&instruction.inputs, 0)?;
                let witness_var = self.get_input::<Witness>(&instruction.inputs, 1)?;

                let mut witness = witness_var.clone();
                witness.stack.push(script.clone());

                // OP_0 0x20 <script hash>
                let mut script_pubkey = vec![OP_0.to_u8(), 32];
                let script_hash = sha256::Hash::hash(script.as_slice());
                script_pubkey.extend(script_hash.as_byte_array().as_slice());

                self.append_variable(Scripts {
                    script_pubkey,
                    script_sig: vec![],
                    witness,
                    requires_signing: None,
                });
            }
            Operation::BuildPayToScriptHash => {
                let script = self.get_input::<Vec<u8>>(&instruction.inputs, 0)?;
                let witness_var = self.get_input::<Witness>(&instruction.inputs, 1)?;

                let mut witness = witness_var.clone();
                witness.stack.push(script.clone());

                let mut script_sig_builder = ScriptBuf::builder().push_opcode(OP_0);
                for elem in witness.stack.drain(..) {
                    script_sig_builder =
                        script_sig_builder.push_slice(&PushBytesBuf::try_from(elem).unwrap());
                }

                let script_hash = ScriptBuf::from(script.clone()).script_hash();
                let script_pubkey = ScriptBuf::new_p2sh(&script_hash).into_bytes();

                self.append_variable(Scripts {
                    script_pubkey,
                    script_sig: script_sig_builder.into_bytes(),
                    witness: Witness { stack: Vec::new() },
                    requires_signing: None,
                });
            }
            Operation::BuildPayToAnchor => {
                self.append_variable(Scripts {
                    script_pubkey: vec![OP_TRUE.to_u8(), 0x2, 0x4e, 0x73], // P2A: https://github.com/bitcoin/bitcoin/pull/30352
                    script_sig: vec![],
                    witness: Witness { stack: Vec::new() },
                    requires_signing: None,
                });
            }
            Operation::BuildRawScripts => {
                let script_pubkey_var = self.get_input::<Vec<u8>>(&instruction.inputs, 0)?;
                let script_sig_var = self.get_input::<Vec<u8>>(&instruction.inputs, 1)?;
                let witness_var = self.get_input::<Witness>(&instruction.inputs, 2)?;

                let script_pubkey = script_pubkey_var.clone();
                let script_sig = script_sig_var.clone();
                let witness = witness_var.clone();

                self.append_variable(Scripts {
                    script_pubkey,
                    script_sig,
                    witness,
                    requires_signing: None,
                });
            }
            Operation::BuildOpReturnScripts => {
                let size_var = self.get_input::<usize>(&instruction.inputs, 0)?;

                let data = vec![0x41u8; *size_var];
                let script = ScriptBuf::builder()
                    .push_opcode(OP_RETURN)
                    .push_slice(&PushBytesBuf::try_from(data).unwrap());

                self.append_variable(Scripts {
                    script_pubkey: script.into_bytes(),
                    script_sig: vec![],
                    witness: Witness { stack: Vec::new() },
                    requires_signing: None,
                });
            }
            Operation::BuildPayToPubKey
            | Operation::BuildPayToPubKeyHash
            | Operation::BuildPayToWitnessPubKeyHash => {
                let private_key_var = self.get_input::<[u8; 32]>(&instruction.inputs, 0)?;
                let _sig_hash_flags_var = self.get_input::<u8>(&instruction.inputs, 1)?;

                let private_key =
                    PrivateKey::from_slice(private_key_var, NetworkKind::Main).unwrap();
                let public_key = private_key.public_key(&self.secp_ctx);
                let public_key_bytes = public_key.to_bytes();

                let (script_pubkey, script_sig, witness_stack) = match &instruction.operation {
                    Operation::BuildPayToPubKey => {
                        (ScriptBuf::new_p2pk(&public_key), ScriptBuf::new(), vec![])
                    }
                    Operation::BuildPayToPubKeyHash => (
                        ScriptBuf::new_p2pkh(&public_key.pubkey_hash()),
                        ScriptBuf::builder().push_key(&public_key).into_script(),
                        vec![],
                    ),
                    Operation::BuildPayToWitnessPubKeyHash => (
                        ScriptBuf::new_p2wpkh(&public_key.wpubkey_hash().unwrap()),
                        ScriptBuf::new(),
                        vec![public_key_bytes],
                    ),
                    _ => (ScriptBuf::new(), ScriptBuf::new(), vec![]),
                };

                self.append_variable(Scripts {
                    script_pubkey: script_pubkey.into(),
                    script_sig: script_sig.into(),
                    witness: Witness {
                        stack: witness_stack,
                    },
                    requires_signing: Some((
                        instruction.operation.clone(),
                        vec![instruction.inputs[0], instruction.inputs[1]],
                    )),
                });
            }
            _ => unreachable!(
                "Non-script-building operation passed to handle_script_building_operations"
            ),
        }
        Ok(())
    }

    fn handle_transaction_building_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BeginBuildTx => {
                let tx_version_var = self.get_input::<u32>(&instruction.inputs, 0)?;
                let tx_lock_time_var = self.get_input::<u32>(&instruction.inputs, 1)?;

                self.append_variable(Tx {
                    tx: Transaction {
                        version: transaction::Version(*tx_version_var as i32),
                        lock_time: LockTime::from_consensus(*tx_lock_time_var),
                        input: Vec::new(),
                        output: Vec::new(),
                    },
                    txos: Vec::new(),
                    output_selector: 0,
                    id: Txid::all_zeros(),
                });
            }
            Operation::EndBuildTx => {
                self.finalize_tx(&instruction)?;
            }
            Operation::BeginBuildTxInputs => {
                self.append_variable(TxInputs {
                    inputs: Vec::new(),
                    total_value: 0,
                });
            }
            Operation::EndBuildTxInputs => {
                let tx_inputs_var = self.get_input::<TxInputs>(&instruction.inputs, 0)?;
                self.append_variable(tx_inputs_var.clone());
            }
            Operation::AddTxInput => {
                self.add_tx_input(&instruction)?;
            }
            Operation::BeginBuildTxOutputs => {
                let tx_inputs_var = self.get_input::<TxInputs>(&instruction.inputs, 0)?;
                let fees = tx_inputs_var.total_value;
                self.append_variable(TxOutputs {
                    outputs: Vec::new(),
                    fees,
                });
            }
            Operation::EndBuildTxOutputs => {
                let tx_outputs_var = self
                    .get_input_mut::<TxOutputs>(&instruction.inputs, 0)?
                    .clone();
                self.append_variable(tx_outputs_var);
            }
            Operation::AddTxOutput => {
                let scripts = self.get_input::<Scripts>(&instruction.inputs, 1)?.clone();
                let amount = self.get_input::<u64>(&instruction.inputs, 2)?.clone();

                let mut_tx_outputs_var = self.get_input_mut::<TxOutputs>(&instruction.inputs, 0)?;

                let amount = amount.min(mut_tx_outputs_var.fees);
                mut_tx_outputs_var.outputs.push((scripts, amount));
                mut_tx_outputs_var.fees -= amount;
            }
            Operation::TakeTxo => {
                let tx_var = self.get_input_mut::<Tx>(&instruction.inputs, 0)?;
                let num_txos = tx_var.txos.len();
                let mut txo = Txo::new();
                if num_txos != 0 {
                    txo = tx_var.txos[tx_var.output_selector % num_txos].clone();
                    tx_var.output_selector += 1;
                }

                self.append_variable(txo);
            }
            _ => unreachable!(
                "Non-transaction-building operation passed to handle_transaction_building_operations"
            ),
        }
        Ok(())
    }

    fn handle_coinbase_building_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BeginBuildCoinbaseTx => {
                let tx_version_var = self.get_input::<u32>(&instruction.inputs, 0)?;
                let tx_lock_time_var = self.get_input::<u32>(&instruction.inputs, 1)?;

                self.append_variable(Tx {
                    tx: Transaction {
                        version: transaction::Version(*tx_version_var as i32),
                        lock_time: LockTime::from_consensus(*tx_lock_time_var),
                        input: Vec::new(),
                        output: Vec::new(),
                    },
                    txos: Vec::new(),
                    output_selector: 0,
                    id: Txid::all_zeros(),
                });
            }
            Operation::EndBuildCoinbaseTx => {
                let mut tx_var = self.get_input_mut::<Tx>(&instruction.inputs, 0)?.clone();
                let coinbase_input_var = self
                    .get_input::<CoinbaseInput>(&instruction.inputs, 1)?
                    .clone();
                let tx_outputs_var = self.get_input::<TxOutputs>(&instruction.inputs, 2)?.clone();

                let mut witness = bitcoin::Witness::new();
                witness.push([0u8; 32]);
                let coinbase_txin = TxIn {
                    previous_output: OutPoint::null(),
                    script_sig: ScriptBuf::new(),
                    sequence: Sequence(coinbase_input_var.sequence.try_into().unwrap()),
                    witness,
                };

                tx_var.tx.input.push(coinbase_txin);

                tx_var
                    .tx
                    .output
                    .extend(
                        tx_outputs_var
                            .outputs
                            .iter()
                            .map(|(scripts, amount)| TxOut {
                                value: Amount::from_sat(*amount),
                                script_pubkey: Script::from_bytes(scripts.script_pubkey.as_slice())
                                    .into(),
                            }),
                    );

                let witness_commitment_output =
                    fuzzamoto::test_utils::mining::create_witness_commitment_output(
                        WitnessMerkleNode::from_raw_hash(Wtxid::all_zeros().into()),
                    );

                tx_var.tx.output.push(witness_commitment_output);

                let mut scripts_vec = Vec::new();
                for output in tx_outputs_var.outputs.iter() {
                    scripts_vec.push(output.0.clone());
                }

                self.append_variable(CoinbaseTx {
                    tx: tx_var,
                    scripts: scripts_vec,
                });
            }
            Operation::BuildCoinbaseTxInput => {
                let sequence_var = self.get_input::<u32>(&instruction.inputs, 0)?.clone();

                self.append_variable(CoinbaseInput {
                    sequence: sequence_var as usize,
                    total_value: Amount::from_int_btc(25).to_sat(),
                });
            }
            Operation::BeginBuildCoinbaseTxOutputs => {
                let coinbase_input_var = self.get_input::<CoinbaseInput>(&instruction.inputs, 0)?;
                let fees = coinbase_input_var.total_value;
                self.append_variable(TxOutputs {
                    outputs: Vec::new(),
                    fees,
                });
            }
            Operation::EndBuildCoinbaseTxOutputs => {
                let tx_outputs_var = self
                    .get_input_mut::<TxOutputs>(&instruction.inputs, 0)?
                    .clone();
                self.append_variable(tx_outputs_var);
            }
            Operation::AddCoinbaseTxOutput => {
                let scripts = self.get_input::<Scripts>(&instruction.inputs, 1)?.clone();
                let amount = self.get_input::<u64>(&instruction.inputs, 2)?.clone();

                let mut_tx_outputs_var = self.get_input_mut::<TxOutputs>(&instruction.inputs, 0)?;

                let amount = amount.min(mut_tx_outputs_var.fees);
                mut_tx_outputs_var.outputs.push((scripts, amount));
                mut_tx_outputs_var.fees -= amount;
            }
            _ => unreachable!(
                "Non-coinbase-building operation passed to handle_coinbase_building_operations"
            ),
        }
        Ok(())
    }

    fn handle_message_sending_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::SendBlockTxn => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let blocktxn = self
                    .get_input::<BlockTransactions>(&instruction.inputs, 1)?
                    .clone();
                // log::info!("OK {:?}", blocktxn);
                self.emit_send_message(*connection_var, "blocktxn", &blocktxn);
            }
            Operation::SendRawMessage => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let message_type_var = self.get_input::<[char; 12]>(&instruction.inputs, 1)?;
                let bytes_var = self.get_input::<Vec<u8>>(&instruction.inputs, 2)?;

                self.emit_send_raw_message(
                    *connection_var,
                    &message_type_var.iter().collect::<String>(),
                    bytes_var.clone(),
                );
            }
            Operation::SendTxNoWit | Operation::SendTx => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let tx_var = self.get_input::<Tx>(&instruction.inputs, 1)?;

                let mut tx_var = tx_var.clone();
                if matches!(instruction.operation, Operation::SendTxNoWit) {
                    for input in tx_var.tx.input.iter_mut() {
                        input.witness.clear();
                    }
                }

                self.emit_send_message(*connection_var, "tx", &tx_var.tx);
            }
            Operation::SendGetData | Operation::SendInv => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let inv_var = self.get_input::<Vec<Inventory>>(&instruction.inputs, 1)?;

                let msg_type = if matches!(instruction.operation, Operation::SendInv) {
                    "inv"
                } else {
                    "getdata"
                };

                self.emit_send_raw_message(
                    *connection_var,
                    msg_type,
                    bitcoin::consensus::encode::serialize(&inv_var),
                );
            }
            Operation::SendHeader => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let header_var = self.get_input::<Header>(&instruction.inputs, 1)?;

                let mut data = vec![1u8]; // 1 header
                data.extend(bitcoin::consensus::encode::serialize(
                    &header_var.to_bitcoin_header(),
                ));
                data.push(0); // empty txdata

                self.emit_send_raw_message(*connection_var, "headers", data);
            }
            Operation::SendBlock | Operation::SendBlockNoWit => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let block_var = self.get_input::<bitcoin::Block>(&instruction.inputs, 1)?;

                let mut block_var = block_var.clone();
                if matches!(instruction.operation, Operation::SendBlockNoWit) {
                    for tx in block_var.txdata.iter_mut() {
                        for input in tx.input.iter_mut() {
                            input.witness.clear();
                        }
                    }
                }
                self.emit_send_message(*connection_var, "block", &block_var);
            }
            Operation::SendGetCFilters | Operation::SendGetCFHeaders => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let compact_filter_type_var = self.get_input::<u8>(&instruction.inputs, 1)?;
                let block_height_var = self.get_input::<u32>(&instruction.inputs, 2)?;
                let header_var = self.get_input::<Header>(&instruction.inputs, 3)?;

                if matches!(instruction.operation, Operation::SendGetCFilters) {
                    self.emit_send_message(
                        *connection_var,
                        "getcfilters",
                        &GetCFilters {
                            filter_type: *compact_filter_type_var,
                            start_height: *block_height_var,
                            stop_hash: header_var.to_bitcoin_header().block_hash(),
                        },
                    );
                } else {
                    self.emit_send_message(
                        *connection_var,
                        "getcfheaders",
                        &GetCFHeaders {
                            filter_type: *compact_filter_type_var,
                            start_height: *block_height_var,
                            stop_hash: header_var.to_bitcoin_header().block_hash(),
                        },
                    );
                };
            }
            Operation::SendGetCFCheckpt => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let compact_filter_type_var = self.get_input::<u8>(&instruction.inputs, 1)?;
                let header_var = self.get_input::<Header>(&instruction.inputs, 2)?;

                self.emit_send_message(
                    *connection_var,
                    "getcfcheckpt",
                    &GetCFCheckpt {
                        filter_type: *compact_filter_type_var,
                        stop_hash: header_var.to_bitcoin_header().block_hash(),
                    },
                );
            }
            Operation::SendFilterLoad => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let filter_load = self.get_input::<FilterLoad>(&instruction.inputs, 1)?;

                self.emit_send_message(
                    *connection_var,
                    "filterload",
                    &FilterLoad {
                        filter: filter_load.filter.clone(),
                        hash_funcs: filter_load.hash_funcs,
                        tweak: filter_load.tweak,
                        flags: filter_load.flags,
                    },
                );
            }
            Operation::SendFilterAdd => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let filteradd = self.get_input::<FilterAdd>(&instruction.inputs, 1)?;

                self.emit_send_message(
                    *connection_var,
                    "filteradd",
                    &FilterAdd {
                        data: filteradd.data.clone(),
                    },
                );
            }
            Operation::SendFilterClear => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let empty: Vec<u8> = Vec::new();
                self.emit_send_message(*connection_var, "filterclear", &empty);
            }
            Operation::SendCompactBlock => {
                let connection_var = self.get_input::<usize>(&instruction.inputs, 0)?;
                let compact_block = self.get_input::<CmpctBlock>(&instruction.inputs, 1)?;
                self.emit_send_message(
                    *connection_var,
                    "cmpctblock",
                    &CmpctBlock {
                        compact_block: compact_block.compact_block.clone(),
                    },
                );
            }
            _ => unreachable!(
                "Non-message-sending operation passed to handle_message_sending_operations"
            ),
        }
        Ok(())
    }

    fn handle_probe_operations(&mut self, instruction: &Instruction) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::Probe(op) => match op {
                ProbeOperation::EnableRecv => {
                    self.emit_enable_logging_message();
                }
                ProbeOperation::DisableRecv => {
                    self.emit_disable_logging_message();
                }
            },
            _ => unreachable!("Non probing operation passed to handle_probe_operations"),
        }

        Ok(())
    }

    fn handle_load_operations(&mut self, instruction: &Instruction) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::Nop {
                outputs,
                inner_outputs,
            } => {
                for _ in 0..*outputs {
                    self.handle_load_operation(Nop);
                }
                for _ in 0..*inner_outputs {
                    self.handle_load_operation(Nop);
                }
            }
            Operation::LoadNode(index) => self.handle_load_operation(*index),
            Operation::LoadConnection(index) => self.handle_load_operation(*index),
            Operation::LoadConnectionType(connection_type) => {
                self.handle_load_operation(connection_type.clone())
            }
            Operation::LoadDuration(duration) => self.handle_load_operation(*duration),
            Operation::LoadAmount(amount) => self.handle_load_operation(*amount),
            Operation::LoadTxVersion(version) => self.handle_load_operation(*version),
            Operation::LoadBlockVersion(version) => self.handle_load_operation(*version),
            Operation::LoadLockTime(lock_time) => self.handle_load_operation(*lock_time),
            Operation::LoadSequence(sequence) => self.handle_load_operation(*sequence),
            Operation::LoadTime(time) => self.handle_load_operation(*time),
            Operation::LoadBlockHeight(height) => self.handle_load_operation(*height),
            Operation::LoadCompactFilterType(filter_type) => {
                self.handle_load_operation(*filter_type)
            }
            Operation::LoadMsgType(message_type) => self.handle_load_operation(*message_type),
            Operation::LoadBytes(bytes) => self.handle_load_operation(bytes.clone()),
            Operation::LoadSize(size) => self.handle_load_operation(*size),
            Operation::LoadPrivateKey(private_key) => self.handle_load_operation(*private_key),
            Operation::LoadSigHashFlags(sig_hash_flags) => {
                self.handle_load_operation(*sig_hash_flags)
            }
            Operation::LoadHeader {
                prev,
                merkle_root,
                nonce,
                bits,
                time,
                version,
                height,
            } => {
                self.handle_load_operation(Header {
                    prev: *prev,
                    merkle_root: *merkle_root,
                    nonce: *nonce,
                    bits: *bits,
                    time: *time,
                    version: *version,
                    height: *height,
                });
            }
            Operation::LoadTxo {
                outpoint,
                value,
                script_pubkey,
                spending_script_sig,
                spending_witness,
            } => {
                self.handle_load_operation(Txo {
                    prev_out: *outpoint,
                    value: *value,
                    scripts: Scripts {
                        script_pubkey: script_pubkey.clone(),
                        script_sig: spending_script_sig.clone(),
                        witness: Witness {
                            stack: spending_witness.clone(),
                        },
                        requires_signing: None,
                    },
                });
            }
            Operation::LoadFilterLoad {
                filter,
                hash_funcs,
                tweak,
                flags,
            } => {
                // because BloomFilter doesn't implement `Hash` and `Deserialize` so I can't use it inside `Operation`
                // thus here transform it on the fly.
                let flags = match flags {
                    0 => BloomFlags::None,
                    1 => BloomFlags::All,
                    2 => BloomFlags::PubkeyOnly,
                    _ => unreachable!("Invalid BloomFlags"),
                };
                self.handle_load_operation(FilterLoad {
                    filter: filter.clone(),
                    hash_funcs: *hash_funcs,
                    tweak: *tweak,
                    flags: flags,
                });
            }
            Operation::LoadFilterAdd { data } => {
                self.handle_load_operation(FilterAdd { data: data.clone() });
            }
            Operation::LoadNonce(nonce) => self.handle_load_operation(*nonce),
            Operation::LoadTxIndexes { indexes } => {
                self.handle_load_operation(indexes.clone());
            }
            Operation::LoadBlockTxnRequestVec { vec } => {
                self.handle_load_operation(vec.clone());
            }
            _ => unreachable!("Non-load operation passed to handle_load_operations"),
        }
        Ok(())
    }

    fn handle_block_building_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BeginBlockTransactions => {
                self.append_variable(Vec::<Tx>::new());
            }
            Operation::AddTx => {
                let tx_var = self.get_input::<Tx>(&instruction.inputs, 1)?.clone();
                let block_transactions_var =
                    self.get_input_mut::<Vec<Tx>>(&instruction.inputs, 0)?;
                block_transactions_var.push(tx_var);
            }
            Operation::EndBlockTransactions => {
                let block_transactions_var = self.get_input::<Vec<Tx>>(&instruction.inputs, 0)?;
                self.append_variable(block_transactions_var.clone());
            }
            Operation::BuildBlock => {
                self.build_block(&instruction)?;
            }
            _ => unreachable!(
                "Non-block-building operation passed to handle_block_building_operations"
            ),
        }
        Ok(())
    }

    fn handle_time_operations(&mut self, instruction: &Instruction) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::AdvanceTime => {
                let time_var = self.get_input::<u64>(&instruction.inputs, 0)?;
                let duration_var = self.get_input::<Duration>(&instruction.inputs, 1)?;
                self.append_variable(*time_var + duration_var.as_secs());
            }
            Operation::SetTime => {
                let time_var = self.get_input::<u64>(&instruction.inputs, 0)?;
                self.output.actions.push(CompiledAction::SetTime(*time_var));
            }
            _ => unreachable!("Non-time operation passed to handle_time_operations"),
        }
        Ok(())
    }

    fn get_variable<'a, T: 'static>(&'a self, index: usize) -> Result<&'a T, CompilerError> {
        let var = self
            .variables
            .get(index)
            .ok_or(CompilerError::VariableNotFound)?;
        let var = var
            .downcast_ref::<T>()
            .ok_or(CompilerError::IncorrectVariableType)?;
        Ok(var)
    }

    fn get_input<'a, T: 'static>(
        &'a self,
        inputs: &[usize],
        index: usize,
    ) -> Result<&'a T, CompilerError> {
        let var_index = inputs
            .get(index)
            .ok_or(CompilerError::IncorrectNumberOfInputs)?;
        self.get_variable(*var_index)
    }

    fn get_input_mut<'a, T: 'static>(
        &'a mut self,
        inputs: &[usize],
        index: usize,
    ) -> Result<&'a mut T, CompilerError> {
        let var_index = inputs
            .get(index)
            .ok_or(CompilerError::IncorrectNumberOfInputs)?;
        let var = self
            .variables
            .get_mut(*var_index)
            .ok_or(CompilerError::VariableNotFound)?;
        let var = var
            .downcast_mut::<T>()
            .ok_or(CompilerError::IncorrectVariableType)?;
        Ok(var)
    }

    fn append_variable<T: 'static>(&mut self, value: T) {
        self.variables.push(Box::new(value));
    }

    fn emit_send_raw_message(&mut self, connection_var: usize, message_type: &str, bytes: Vec<u8>) {
        self.output.actions.push(CompiledAction::SendRawMessage(
            connection_var,
            message_type.to_string(),
            bytes,
        ));
    }

    fn emit_enable_logging_message(&mut self) {
        self.output.actions.push(CompiledAction::EnableLogging());
    }

    fn emit_disable_logging_message(&mut self) {
        self.output.actions.push(CompiledAction::DisableLogging());
    }

    fn emit_send_message<T: Encodable>(
        &mut self,
        connection_var: usize,
        message_type: &str,
        message: &T,
    ) {
        self.emit_send_raw_message(
            connection_var,
            message_type,
            bitcoin::consensus::encode::serialize(message),
        );
    }

    fn build_block(&mut self, instruction: &Instruction) -> Result<(), CompilerError> {
        let mut coinbase_tx_var = self
            .get_input::<CoinbaseTx>(&instruction.inputs, 0)?
            .clone();
        let header_var = self.get_input::<Header>(&instruction.inputs, 1)?.clone();
        let time_var = self.get_input::<u64>(&instruction.inputs, 2)?.clone();
        let block_version_var = self.get_input::<i32>(&instruction.inputs, 3)?.clone();
        let block_transactions_var = self.get_input::<Vec<Tx>>(&instruction.inputs, 4)?.clone();

        coinbase_tx_var.tx.tx.input[0].script_sig = ScriptBuf::builder()
            .push_int((header_var.height + 1) as i64)
            .push_int(0xFFFFFFFF)
            .as_script()
            .into();

        let mut txdata = vec![coinbase_tx_var.tx.tx.clone()];
        txdata.extend(block_transactions_var.iter().map(|tx| tx.tx.clone()));

        let mut block = bitcoin::Block {
            header: bitcoin::block::Header {
                version: bitcoin::block::Version::from_consensus(block_version_var),
                prev_blockhash: header_var.to_bitcoin_header().block_hash(),
                merkle_root: TxMerkleNode::all_zeros(),
                bits: CompactTarget::from_consensus(header_var.bits),
                nonce: header_var.nonce,
                time: time_var as u32,
            },
            txdata,
        };
        fuzzamoto::test_utils::mining::fixup_commitments(&mut block);

        if cfg!(feature = "reduced_pow") {
            let mut block_hash = block.header.block_hash();
            while block_hash.as_raw_hash()[31] & 0x80 != 0 {
                block.header.nonce += 1;
                block_hash = block.header.block_hash();
            }
            // log::info!("{:?} height={}", block_hash, header_var.height);
        } else {
            let target = block.header.target();
            while block.header.validate_pow(target).is_err() {
                block.header.nonce += 1;
            }
        }

        let coinbase_txid = *coinbase_tx_var
            .tx
            .tx
            .compute_txid()
            .as_raw_hash()
            .as_byte_array();

        let mut txos = Vec::new();

        // Create a Txo for every output except the witness commitment output.
        let num_outputs = coinbase_tx_var.tx.tx.output.len();
        for (index, output) in coinbase_tx_var.tx.tx.output.iter().enumerate() {
            if index == num_outputs - 1 {
                break;
            }

            txos.push(Txo {
                prev_out: (coinbase_txid, index as u32),
                scripts: coinbase_tx_var.scripts[index].clone(),
                value: output.value.to_sat(),
            });
        }

        coinbase_tx_var.tx.txos = txos;

        self.append_variable(Header {
            prev: *block.header.prev_blockhash.as_byte_array(),
            merkle_root: *block.header.merkle_root.as_byte_array(),
            bits: block.header.bits.to_consensus(),
            time: block.header.time,
            height: header_var.height + 1,
            nonce: block.header.nonce,
            version: block.header.version.to_consensus(),
        });

        self.append_variable(block);

        self.append_variable(coinbase_tx_var.tx);

        Ok(())
    }

    fn add_tx_input(&mut self, instruction: &Instruction) -> Result<(), CompilerError> {
        let txo_var = self.get_input::<Txo>(&instruction.inputs, 1)?;
        let _sequence_var = self.get_input::<u32>(&instruction.inputs, 2)?;

        let value = txo_var.value;
        let mut_tx_inputs_var = self.get_input_mut::<TxInputs>(&instruction.inputs, 0)?;

        mut_tx_inputs_var.inputs.push(TxInput {
            txo_var: instruction.inputs[1],
            sequence_var: instruction.inputs[2],
        });
        mut_tx_inputs_var.total_value += value;
        Ok(())
    }

    fn finalize_tx(&mut self, instruction: &Instruction) -> Result<(), CompilerError> {
        let tx_inputs_var = self.get_input::<TxInputs>(&instruction.inputs, 1)?.clone();
        let tx_outputs_var = self.get_input::<TxOutputs>(&instruction.inputs, 2)?.clone();
        let mut tx_var = self.get_input_mut::<Tx>(&instruction.inputs, 0)?.clone();

        // Fill in the inputs and outputs
        tx_var
            .tx
            .input
            .extend(tx_inputs_var.inputs.iter().map(|tx_input| {
                let txo_var = self.get_variable::<Txo>(tx_input.txo_var).unwrap();
                let sequence_var = self.get_variable::<u32>(tx_input.sequence_var).unwrap();
                TxIn {
                    previous_output: OutPoint::new(
                        Txid::from_slice_delegated(&txo_var.prev_out.0).unwrap(),
                        txo_var.prev_out.1,
                    ),
                    script_sig: Script::from_bytes(&txo_var.scripts.script_sig).into(),
                    witness: bitcoin::Witness::from(txo_var.scripts.witness.stack.as_slice()),
                    sequence: Sequence(*sequence_var),
                }
            }));

        tx_var.tx.output.extend(
            tx_outputs_var
                .outputs
                .iter()
                .map(|(scripts, amount)| TxOut {
                    value: Amount::from_sat(*amount),
                    script_pubkey: Script::from_bytes(scripts.script_pubkey.as_slice()).into(),
                }),
        );

        // Sign inputs
        for (idx, input) in tx_inputs_var.inputs.iter().enumerate() {
            let txo_var = self.get_variable::<Txo>(input.txo_var).unwrap();
            if let Some((operation, input_indices)) = &txo_var.scripts.requires_signing {
                let private_key = *self.get_variable::<[u8; 32]>(input_indices[0]).unwrap();
                let sighash_flag = *self.get_variable::<u8>(input_indices[1]).unwrap();

                let mut cache = SighashCache::new(&tx_var.tx);

                match operation {
                    Operation::BuildPayToPubKey | Operation::BuildPayToPubKeyHash => {
                        if let Ok(hash) = cache.legacy_signature_hash(
                            idx,
                            Script::from_bytes(&txo_var.scripts.script_pubkey),
                            sighash_flag as u32,
                        ) {
                            let signature = ecdsa::Signature {
                                signature: self.secp_ctx.sign_ecdsa(
                                    &secp256k1::Message::from_digest(*hash.as_byte_array()),
                                    &SecretKey::from_slice(private_key.as_slice()).unwrap(),
                                ),
                                sighash_type: EcdsaSighashType::from_consensus(sighash_flag as u32),
                            };

                            tx_var.tx.input[idx]
                                .script_sig
                                .push_slice(PushBytesBuf::try_from(signature.to_vec()).unwrap());
                        }
                    }
                    Operation::BuildPayToWitnessPubKeyHash => {
                        let sighash_type = EcdsaSighashType::from_consensus(sighash_flag as u32);
                        if let Ok(hash) = cache.p2wpkh_signature_hash(
                            idx,
                            Script::from_bytes(&txo_var.scripts.script_pubkey),
                            Amount::from_sat(txo_var.value),
                            sighash_type,
                        ) {
                            let signature = ecdsa::Signature {
                                signature: self.secp_ctx.sign_ecdsa(
                                    &secp256k1::Message::from_digest(*hash.as_byte_array()),
                                    &SecretKey::from_slice(private_key.as_slice()).unwrap(),
                                ),
                                sighash_type,
                            };

                            tx_var.tx.input[idx].witness.push(signature.to_vec());
                        }
                    }

                    _ => {}
                }
            }
        }

        let txid = tx_var.tx.compute_txid();
        let id_bytes = *txid.as_raw_hash().as_byte_array();

        // Create all `Txo`s for this transaction and store them on the new finalized tx var
        tx_var.txos = tx_outputs_var
            .outputs
            .iter()
            .enumerate()
            .map(|(index, (scripts, amount))| Txo {
                prev_out: (id_bytes, index as u32),
                scripts: scripts.clone(),
                value: *amount,
            })
            .collect();

        tx_var.id = txid;
        self.append_variable(tx_var);

        Ok(())
    }
}

use std::{any::Any, time::Duration};

use bitcoin::{
    Amount, CompactTarget, EcdsaSighashType, NetworkKind, OutPoint, PrivateKey, Script, ScriptBuf,
    Sequence, Transaction, TxIn, TxMerkleNode, TxOut, Txid, WitnessMerkleNode, Wtxid,
    absolute::LockTime,
    consensus::Encodable,
    ecdsa,
    hashes::{Hash, serde_macros::serde_details::SerdeHash, sha256},
    key::{Secp256k1, TapTweak},
    opcodes::{
        OP_0, OP_TRUE,
        all::{OP_PUSHNUM_1, OP_RETURN},
    },
    p2p::{
        message_blockdata::Inventory,
        message_bloom::{BloomFlags, FilterAdd, FilterLoad},
        message_filter::{GetCFCheckpt, GetCFHeaders, GetCFilters},
    },
    script::PushBytesBuf,
    secp256k1::{self, Keypair, SecretKey, XOnlyPublicKey},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::{LeafVersion, TapLeafHash, TapNodeHash, TaprootBuilder as BitcoinTaprootBuilder},
    transaction,
};

use crate::{
    Instruction, Operation, Program, TaprootKeypair, TaprootLeaf, TaprootSpendInfo, TaprootTxo,
    bloom::filter_insert, generators::block::Header,
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

    requires_signing: Option<SigningRequest>,
}

#[derive(Debug, Clone)]
enum SigningRequest {
    Legacy {
        operation: Operation,
        private_key_var: usize,
        sighash_var: usize,
    },
    Taproot {
        spend_info_var: Option<usize>,
        source_taproot_txo_var: Option<usize>,
        leaf_var: Option<usize>,
        annex_var: Option<usize>,
    },
}

#[derive(Debug, Clone)]
struct Witness {
    stack: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct TaprootTreeBuilder {
    leaves: Vec<TaprootLeafPlan>,
}

#[derive(Clone, Debug)]
struct TaprootLeafPlan {
    script: Vec<u8>,
    version: LeafVersion,
}

fn build_control_block(
    spend_info: &TaprootSpendInfo,
    leaf: &TaprootLeaf,
    annex_present: bool,
) -> Vec<u8> {
    let mut control =
        Vec::with_capacity(1 + spend_info.keypair.public_key.len() + 32 * leaf.merkle_branch.len());
    let parity = spend_info.output_key_parity & 1;
    let mut first_byte = leaf.version | parity;
    if annex_present {
        first_byte |= 0b10;
    }
    control.push(first_byte);
    control.extend_from_slice(&spend_info.keypair.public_key);
    for hash in &leaf.merkle_branch {
        control.extend_from_slice(hash);
    }
    control
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
                | Operation::LoadTaprootTxo { .. }
                | Operation::LoadTaprootAnnex { .. }
                | Operation::LoadFilterLoad { .. }
                | Operation::LoadFilterAdd { .. } => {
                    self.handle_load_operations(&instruction)?;
                }
                Operation::TaprootTxoToSpendInfo
                | Operation::TaprootTxoToKeypair
                | Operation::TaprootTxoToTxo
                | Operation::TaprootSpendInfoSelectLeaf { .. }
                | Operation::TaprootScriptsUseLeaf
                | Operation::TaprootTxoUseLeaf
                | Operation::TaprootScriptsUseAnnex
                | Operation::TaprootTxoUseAnnex => {
                    self.handle_taproot_conversions(&instruction)?;
                }
                Operation::BeginTaprootTree | Operation::AddTapLeaf | Operation::EndTaprootTree => {
                    self.handle_taproot_tree_operations(&instruction)?;
                }
                Operation::LoadTaprootLeafVersion(_) => {
                    self.handle_load_operations(&instruction)?;
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
                | Operation::BuildPayToWitnessPubKeyHash
                | Operation::BuildPayToTaproot => {
                    self.handle_script_building_operations(&instruction)?;
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
                | Operation::SendFilterClear => {
                    self.handle_message_sending_operations(&instruction)?;
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

    fn handle_taproot_conversions(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::TaprootTxoToSpendInfo => {
                let taproot_txo = self.get_input::<TaprootTxo>(&instruction.inputs, 0)?;
                self.append_variable(taproot_txo.spend_info.clone());
            }
            Operation::TaprootTxoToKeypair => {
                let taproot_txo = self.get_input::<TaprootTxo>(&instruction.inputs, 0)?;
                self.append_variable(taproot_txo.spend_info.keypair.clone());
            }
            Operation::TaprootTxoToTxo => {
                let taproot_txo = self.get_input::<TaprootTxo>(&instruction.inputs, 0)?;
                let source_var = instruction.inputs.first().copied();
                self.append_variable(Txo {
                    prev_out: taproot_txo.outpoint,
                    scripts: Scripts {
                        script_pubkey: taproot_txo.spend_info.script_pubkey.clone(),
                        script_sig: vec![],
                        witness: Witness { stack: Vec::new() },
                        requires_signing: Some(SigningRequest::Taproot {
                            spend_info_var: None,
                            source_taproot_txo_var: source_var,
                            leaf_var: None,
                            annex_var: None,
                        }),
                    },
                    value: taproot_txo.value,
                });
            }
            Operation::TaprootSpendInfoSelectLeaf { index } => {
                let spend_info = self.get_input::<TaprootSpendInfo>(&instruction.inputs, 0)?;
                if spend_info.leaves.is_empty() {
                    return Err(CompilerError::MiscError(
                        "taproot spend info does not contain any leaves".to_string(),
                    ));
                }
                let idx = index % spend_info.leaves.len();
                self.append_variable(spend_info.leaves[idx].clone());
            }
            Operation::TaprootScriptsUseLeaf => {
                let mut scripts = self.get_input::<Scripts>(&instruction.inputs, 0)?.clone();
                let leaf_var = instruction
                    .inputs
                    .get(1)
                    .copied()
                    .ok_or(CompilerError::IncorrectNumberOfInputs)?;
                self.get_input::<TaprootLeaf>(&instruction.inputs, 1)?;

                match &mut scripts.requires_signing {
                    Some(SigningRequest::Taproot {
                        leaf_var: target, ..
                    }) => {
                        *target = Some(leaf_var);
                    }
                    _ => {
                        return Err(CompilerError::MiscError(
                            "TaprootScriptsUseLeaf requires a taproot script".to_string(),
                        ));
                    }
                }

                self.append_variable(scripts);
            }
            Operation::TaprootTxoUseLeaf => {
                let mut txo = self.get_input::<Txo>(&instruction.inputs, 0)?.clone();
                let leaf_var = instruction
                    .inputs
                    .get(1)
                    .copied()
                    .ok_or(CompilerError::IncorrectNumberOfInputs)?;
                self.get_input::<TaprootLeaf>(&instruction.inputs, 1)?;

                match &mut txo.scripts.requires_signing {
                    Some(SigningRequest::Taproot {
                        leaf_var: target, ..
                    }) => {
                        *target = Some(leaf_var);
                    }
                    _ => {
                        return Err(CompilerError::MiscError(
                            "TaprootTxoUseLeaf requires a taproot script".to_string(),
                        ));
                    }
                }

                self.append_variable(txo);
            }
            Operation::TaprootScriptsUseAnnex => {
                let mut scripts = self.get_input::<Scripts>(&instruction.inputs, 0)?.clone();
                let annex_var = instruction
                    .inputs
                    .get(1)
                    .copied()
                    .ok_or(CompilerError::IncorrectNumberOfInputs)?;
                self.get_input::<Vec<u8>>(&instruction.inputs, 1)?;

                match &mut scripts.requires_signing {
                    Some(SigningRequest::Taproot {
                        annex_var: target, ..
                    }) => {
                        *target = Some(annex_var);
                    }
                    _ => {
                        return Err(CompilerError::MiscError(
                            "TaprootScriptsUseAnnex requires a taproot script".to_string(),
                        ));
                    }
                }

                self.append_variable(scripts);
            }
            Operation::TaprootTxoUseAnnex => {
                let mut txo = self.get_input::<Txo>(&instruction.inputs, 0)?.clone();
                let annex_var = instruction
                    .inputs
                    .get(1)
                    .copied()
                    .ok_or(CompilerError::IncorrectNumberOfInputs)?;
                self.get_input::<Vec<u8>>(&instruction.inputs, 1)?;

                match &mut txo.scripts.requires_signing {
                    Some(SigningRequest::Taproot {
                        annex_var: target, ..
                    }) => {
                        *target = Some(annex_var);
                    }
                    _ => {
                        return Err(CompilerError::MiscError(
                            "TaprootTxoUseAnnex requires a taproot script".to_string(),
                        ));
                    }
                }

                self.append_variable(txo);
            }
            _ => unreachable!("Unsupported taproot helper"),
        }
        Ok(())
    }

    fn handle_taproot_tree_operations(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        match &instruction.operation {
            Operation::BeginTaprootTree => {
                self.append_variable(TaprootTreeBuilder { leaves: Vec::new() });
            }
            Operation::AddTapLeaf => {
                let script = self.get_input::<Vec<u8>>(&instruction.inputs, 1)?.clone();
                let version_byte = *self.get_input::<u8>(&instruction.inputs, 2)?;
                let version = LeafVersion::from_consensus(version_byte).map_err(|e| {
                    CompilerError::MiscError(format!("invalid taproot leaf version: {e:?}"))
                })?;
                let builder = self.get_input_mut::<TaprootTreeBuilder>(&instruction.inputs, 0)?;
                builder.leaves.push(TaprootLeafPlan { script, version });
            }
            Operation::EndTaprootTree => {
                let builder = self.get_input::<TaprootTreeBuilder>(&instruction.inputs, 0)?;
                let keypair = self.get_input::<TaprootKeypair>(&instruction.inputs, 1)?;
                if builder.leaves.is_empty() {
                    return Err(CompilerError::MiscError(
                        "cannot finalize taproot tree without leaves".to_string(),
                    ));
                }
                let internal_key =
                    XOnlyPublicKey::from_slice(&keypair.public_key).map_err(|_| {
                        CompilerError::MiscError("invalid taproot internal key bytes".to_string())
                    })?;

                let mut taproot_builder = BitcoinTaprootBuilder::new();
                for leaf in &builder.leaves {
                    let script_buf = ScriptBuf::from(leaf.script.clone());
                    taproot_builder = taproot_builder
                        .add_leaf_with_ver(0, script_buf, leaf.version)
                        .map_err(|e| {
                            CompilerError::MiscError(format!("failed to add tapleaf: {e:?}"))
                        })?;
                }

                let spend_info = taproot_builder
                    .finalize(&self.secp_ctx, internal_key)
                    .map_err(|e| {
                        CompilerError::MiscError(format!("finalizing taproot tree failed: {e:?}"))
                    })?;

                let output_key_bytes = spend_info.output_key().to_x_only_public_key().serialize();
                let push_bytes =
                    PushBytesBuf::try_from(output_key_bytes.to_vec()).map_err(|_| {
                        CompilerError::MiscError("failed to encode taproot key bytes".to_string())
                    })?;
                let script_pubkey = ScriptBuf::builder()
                    .push_opcode(OP_PUSHNUM_1)
                    .push_slice(&push_bytes)
                    .into_script();

                let leaves = builder
                    .leaves
                    .iter()
                    .map(|leaf| {
                        let script_buf = ScriptBuf::from(leaf.script.clone());
                        let control_block = spend_info
                            .control_block(&(script_buf.clone(), leaf.version))
                            .ok_or_else(|| {
                                CompilerError::MiscError(
                                    "missing control block for tapscript leaf".to_string(),
                                )
                            })?;
                        let merkle_branch = control_block
                            .merkle_branch
                            .iter()
                            .map(|hash| *hash.as_byte_array())
                            .collect();
                        Ok(TaprootLeaf {
                            version: leaf.version.to_consensus(),
                            script: leaf.script.clone(),
                            merkle_branch,
                        })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;

                let merkle_root = spend_info.merkle_root().map(|root| *root.as_byte_array());
                self.append_variable(TaprootSpendInfo {
                    keypair: keypair.clone(),
                    merkle_root,
                    output_key: output_key_bytes,
                    output_key_parity: match spend_info.output_key_parity() {
                        secp256k1::Parity::Even => 0,
                        secp256k1::Parity::Odd => 1,
                    },
                    script_pubkey: script_pubkey.as_bytes().to_vec(),
                    leaves,
                });
            }
            _ => unreachable!("Unsupported taproot tree operation"),
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
            Operation::BuildPayToTaproot => {
                let spend_info = self.get_input::<TaprootSpendInfo>(&instruction.inputs, 0)?;
                let spend_info_var = instruction.inputs.first().copied();

                self.append_variable(Scripts {
                    script_pubkey: spend_info.script_pubkey.clone(),
                    script_sig: vec![],
                    witness: Witness { stack: Vec::new() },
                    requires_signing: Some(SigningRequest::Taproot {
                        spend_info_var,
                        source_taproot_txo_var: None,
                        leaf_var: None,
                        annex_var: None,
                    }),
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
                    requires_signing: Some(SigningRequest::Legacy {
                        operation: instruction.operation.clone(),
                        private_key_var: instruction.inputs[0],
                        sighash_var: instruction.inputs[1],
                    }),
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
            _ => unreachable!(
                "Non-message-sending operation passed to handle_message_sending_operations"
            ),
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
            Operation::LoadTaprootTxo { txo } => {
                self.handle_load_operation(txo.clone());
            }
            Operation::LoadTaprootAnnex { annex } => {
                self.handle_load_operation(annex.clone());
            }
            Operation::LoadTaprootLeafVersion(version) => {
                self.handle_load_operation(*version);
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
            log::info!("{:?} height={}", block_hash, header_var.height);
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

        let mut prevouts = Vec::with_capacity(tx_inputs_var.inputs.len());
        for tx_input in &tx_inputs_var.inputs {
            let txo_var = self.get_variable::<Txo>(tx_input.txo_var).unwrap();
            prevouts.push(TxOut {
                value: Amount::from_sat(txo_var.value),
                script_pubkey: Script::from_bytes(&txo_var.scripts.script_pubkey).into(),
            });
        }

        // Sign inputs
        for (idx, input) in tx_inputs_var.inputs.iter().enumerate() {
            let txo_var = self.get_variable::<Txo>(input.txo_var).unwrap();
            if let Some(signing_request) = &txo_var.scripts.requires_signing {
                let mut cache = SighashCache::new(&tx_var.tx);

                match signing_request {
                    SigningRequest::Legacy {
                        operation,
                        private_key_var,
                        sighash_var,
                    } => {
                        let private_key = *self.get_variable::<[u8; 32]>(*private_key_var).unwrap();
                        let sighash_flag = *self.get_variable::<u8>(*sighash_var).unwrap();

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
                                        sighash_type: EcdsaSighashType::from_consensus(
                                            sighash_flag as u32,
                                        ),
                                    };

                                    tx_var.tx.input[idx].script_sig.push_slice(
                                        PushBytesBuf::try_from(signature.to_vec()).unwrap(),
                                    );
                                }
                            }
                            Operation::BuildPayToWitnessPubKeyHash => {
                                let sighash_type =
                                    EcdsaSighashType::from_consensus(sighash_flag as u32);
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
                    SigningRequest::Taproot {
                        spend_info_var,
                        source_taproot_txo_var,
                        leaf_var,
                        annex_var,
                    } => {
                        let spend_info = if let Some(var) = spend_info_var {
                            self.get_variable::<TaprootSpendInfo>(*var).unwrap().clone()
                        } else if let Some(var) = source_taproot_txo_var {
                            self.get_variable::<TaprootTxo>(*var)
                                .unwrap()
                                .spend_info
                                .clone()
                        } else {
                            return Err(CompilerError::MiscError(
                                "taproot signing missing spend info".to_string(),
                            ));
                        };

                        let annex_bytes = if let Some(var) = annex_var {
                            let annex = self.get_variable::<Vec<u8>>(*var).map_err(|_| {
                                CompilerError::MiscError(
                                    "taproot annex variable missing".to_string(),
                                )
                            })?;
                            if annex.is_empty() || annex[0] != 0x50 {
                                return Err(CompilerError::MiscError(
                                    "taproot annex must start with 0x50".to_string(),
                                ));
                            }
                            Some(annex.clone())
                        } else {
                            None
                        };

                        if let Some(leaf_idx) = leaf_var {
                            let leaf =
                                self.get_variable::<TaprootLeaf>(*leaf_idx).map_err(|_| {
                                    CompilerError::MiscError(
                                        "taproot leaf variable missing for script-path spend"
                                            .to_string(),
                                    )
                                })?;
                            let leaf_version =
                                LeafVersion::from_consensus(leaf.version).map_err(|e| {
                                    CompilerError::MiscError(format!(
                                        "invalid taproot leaf version: {e:?}"
                                    ))
                                })?;
                            let script = Script::from_bytes(&leaf.script);
                            let leaf_hash = TapLeafHash::from_script(&script, leaf_version);

                            let secret_key =
                                SecretKey::from_slice(spend_info.keypair.secret_key.as_slice())
                                    .unwrap();
                            let keypair = Keypair::from_secret_key(&self.secp_ctx, &secret_key);

                            let prevouts_ref = Prevouts::All(&prevouts);
                            let sighash = cache
                                .taproot_script_spend_signature_hash(
                                    idx,
                                    &prevouts_ref,
                                    leaf_hash,
                                    TapSighashType::Default,
                                )
                                .map_err(|e| {
                                    CompilerError::MiscError(format!(
                                        "taproot script sighash failed: {e:?}"
                                    ))
                                })?;
                            let msg = secp256k1::Message::from_digest(*sighash.as_byte_array());
                            let signature = self.secp_ctx.sign_schnorr_no_aux_rand(&msg, &keypair);

                            if let Some(annex) = &annex_bytes {
                                tx_var.tx.input[idx].witness.push(annex.clone());
                            }
                            tx_var.tx.input[idx]
                                .witness
                                .push(signature.as_ref().to_vec());
                            tx_var.tx.input[idx].witness.push(leaf.script.clone());
                            tx_var.tx.input[idx].witness.push(build_control_block(
                                &spend_info,
                                leaf,
                                annex_bytes.is_some(),
                            ));
                            continue;
                        }

                        let merkle_root = spend_info.merkle_root.map(TapNodeHash::from_byte_array);
                        let secret_key =
                            SecretKey::from_slice(spend_info.keypair.secret_key.as_slice())
                                .unwrap();
                        let keypair = Keypair::from_secret_key(&self.secp_ctx, &secret_key);
                        let tweaked_keypair =
                            keypair.tap_tweak(&self.secp_ctx, merkle_root).to_keypair();

                        let prevouts_ref = Prevouts::All(&prevouts);
                        let sighash = cache
                            .taproot_key_spend_signature_hash(
                                idx,
                                &prevouts_ref,
                                TapSighashType::Default,
                            )
                            .map_err(|e| {
                                CompilerError::MiscError(format!("taproot sighash failed: {e:?}"))
                            })?;
                        let msg = secp256k1::Message::from_digest(*sighash.as_byte_array());
                        let signature = self
                            .secp_ctx
                            .sign_schnorr_no_aux_rand(&msg, &tweaked_keypair);

                        if let Some(annex) = &annex_bytes {
                            tx_var.tx.input[idx].witness.push(annex.clone());
                        }
                        tx_var.tx.input[idx]
                            .witness
                            .push(signature.as_ref().to_vec());
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Operation, Program, ProgramBuilder, ProgramContext, TaprootKeypair, TaprootLeaf,
        TaprootSpendInfo, TaprootTxo,
    };
    use bitcoin::{
        consensus::Decodable,
        opcodes::all::OP_PUSHNUM_1,
        script::PushBytesBuf,
        secp256k1::{Keypair, Parity, Secp256k1, SecretKey, XOnlyPublicKey},
        taproot::{LeafVersion, TaprootBuilder},
        ScriptBuf, Transaction,
    };

    #[test]
    fn compile_taproot_key_path_with_annex_places_annex_first() {
        let annex = vec![0x50, 0xAA, 0xBB, 0xCC];
        let taproot_txo = sample_taproot_txo();
        let program = build_annex_program(taproot_txo, annex.clone());

        let mut compiler = Compiler::new();
        let compiled = compiler.compile(&program).expect("program compiles");
        let tx_bytes = match compiled.actions.first() {
            Some(CompiledAction::SendRawMessage(_, cmd, payload)) if cmd == "tx" => payload.clone(),
            other => panic!("unexpected action {:?}", other),
        };
        let tx = Transaction::consensus_decode(&mut tx_bytes.as_slice()).unwrap();
        assert_eq!(tx.input.len(), 1);
        assert!(tx.input[0].witness.len() >= 2);
        assert_eq!(tx.input[0].witness[0], annex);
    }

    #[test]
    fn compile_taproot_key_path_produces_expected_tx() {
        let taproot_txo = sample_taproot_txo();
        let program = build_key_path_program(taproot_txo);
        let tx = compile_program_to_transaction(&program);
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.input[0].witness.len(), 1);
        let actual_hex = serialize_tx_hex(&tx);
        assert_eq!(
            actual_hex,
            "0200000000010111111111111111111111111111111111111111111111111111111111111111110000000000feffffff01e8030000000000000451024e73014017cd3fa6d14090c717fc33f033dbda9ba99fccdf3b2122835d83e618b4d4810722c96ae64640ab3b40d244e1e7cf6a6acb4e4620867691f23eaeb038cfeb2a9900000000"
        );
    }

    #[test]
    fn compile_taproot_script_path_produces_expected_tx() {
        let taproot_txo = sample_script_path_taproot_txo();
        let program = build_script_path_program(taproot_txo.clone());
        let tx = compile_program_to_transaction(&program);
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.input[0].witness.len(), 3);
        assert_eq!(
            tx.input[0].witness[1],
            taproot_txo.spend_info.leaves[0].script
        );
        let actual_hex = serialize_tx_hex(&tx);
        assert_eq!(
            actual_hex,
            "0200000000010122222222222222222222222222222222222222222222222222222222222222220000000000feffffff01dc050000000000000451024e730340d3ae6bfda0b8652385157152dfc0f586ecf68a52199e9009c52be0250d9dc2fe973f4d7edde98179af5fbe12a42dca2e94ede2fc769a049257f84927025a9810015121c162c0a046dacce86ddd0343c6d3c7c79c2208ba0d9c9cf24a6d046d21d21f90f700000000"
        );
    }

    fn build_annex_program(taproot_txo: TaprootTxo, annex: Vec<u8>) -> Program {
        let mut builder = ProgramBuilder::new(ProgramContext {
            num_nodes: 1,
            num_connections: 1,
            timestamp: 0,
        });

        let connection = builder.force_append_expect_output(vec![], Operation::LoadConnection(0));
        let taproot_var = builder
            .force_append_expect_output(vec![], Operation::LoadTaprootTxo { txo: taproot_txo });
        let txo_var =
            builder.force_append_expect_output(vec![taproot_var.index], Operation::TaprootTxoToTxo);
        let annex_var =
            builder.force_append_expect_output(vec![], Operation::LoadTaprootAnnex { annex });
        let txo_var = builder.force_append_expect_output(
            vec![txo_var.index, annex_var.index],
            Operation::TaprootTxoUseAnnex,
        );
        let tx_version = builder.force_append_expect_output(vec![], Operation::LoadTxVersion(2));
        let lock_time = builder.force_append_expect_output(vec![], Operation::LoadLockTime(0));
        let mut_tx = builder.force_append_expect_output(
            vec![tx_version.index, lock_time.index],
            Operation::BeginBuildTx,
        );
        let mut_inputs = builder.force_append_expect_output(vec![], Operation::BeginBuildTxInputs);
        let sequence =
            builder.force_append_expect_output(vec![], Operation::LoadSequence(0xffff_fffe));
        builder.force_append(
            vec![mut_inputs.index, txo_var.index, sequence.index],
            Operation::AddTxInput,
        );
        let const_inputs =
            builder.force_append_expect_output(vec![mut_inputs.index], Operation::EndBuildTxInputs);

        let mut_outputs = builder
            .force_append_expect_output(vec![const_inputs.index], Operation::BeginBuildTxOutputs);
        let scripts = builder.force_append_expect_output(vec![], Operation::BuildPayToAnchor);
        let amount = builder.force_append_expect_output(vec![], Operation::LoadAmount(1_000));
        builder.force_append(
            vec![mut_outputs.index, scripts.index, amount.index],
            Operation::AddTxOutput,
        );
        let const_outputs = builder
            .force_append_expect_output(vec![mut_outputs.index], Operation::EndBuildTxOutputs);

        let const_tx = builder.force_append_expect_output(
            vec![mut_tx.index, const_inputs.index, const_outputs.index],
            Operation::EndBuildTx,
        );

        builder.force_append(vec![connection.index, const_tx.index], Operation::SendTx);

        builder.finalize().expect("valid program")
    }

    fn sample_taproot_txo() -> TaprootTxo {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[3u8; 32]).unwrap();
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        let (internal_key, parity) = XOnlyPublicKey::from_keypair(&keypair);
        let push_bytes =
            PushBytesBuf::try_from(internal_key.serialize().to_vec()).expect("push bytes");
        let script_pubkey = ScriptBuf::builder()
            .push_opcode(OP_PUSHNUM_1)
            .push_slice(&push_bytes)
            .into_bytes();
        TaprootTxo {
            outpoint: ([0x11; 32], 0),
            value: 50_000,
            spend_info: TaprootSpendInfo {
                keypair: TaprootKeypair {
                    secret_key: secret_key.secret_bytes(),
                    public_key: internal_key.serialize(),
                },
                merkle_root: None,
                output_key: internal_key.serialize(),
                output_key_parity: match parity {
                    Parity::Even => 0,
                    Parity::Odd => 1,
                },
                script_pubkey,
                leaves: Vec::new(),
            },
        }
    }

    fn sample_script_path_taproot_txo() -> TaprootTxo {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[5u8; 32]).unwrap();
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        let (internal_key, parity) = XOnlyPublicKey::from_keypair(&keypair);
        let script = ScriptBuf::builder().push_opcode(OP_PUSHNUM_1).into_script();
        let mut builder = TaprootBuilder::new();
        builder = builder
            .add_leaf_with_ver(0, script.clone(), LeafVersion::TapScript)
            .expect("add tapleaf");
        let spend_info = builder.finalize(&secp, internal_key).expect("finalize");

        let control_block = spend_info
            .control_block(&(script.clone(), LeafVersion::TapScript))
            .expect("control block");
        let leaf = TaprootLeaf {
            version: LeafVersion::TapScript.to_consensus(),
            script: script.as_bytes().to_vec(),
            merkle_branch: control_block
                .merkle_branch
                .iter()
                .map(|hash| *hash.as_byte_array())
                .collect(),
        };

        let output_key_bytes = spend_info.output_key().to_x_only_public_key().serialize();
        let push_bytes = PushBytesBuf::try_from(output_key_bytes.to_vec()).expect("push bytes");
        let script_pubkey = ScriptBuf::builder()
            .push_opcode(OP_PUSHNUM_1)
            .push_slice(&push_bytes)
            .into_bytes();

        TaprootTxo {
            outpoint: ([0x22; 32], 0),
            value: 60_000,
            spend_info: TaprootSpendInfo {
                keypair: TaprootKeypair {
                    secret_key: secret_key.secret_bytes(),
                    public_key: internal_key.serialize(),
                },
                merkle_root: spend_info.merkle_root().map(|root| *root.as_byte_array()),
                output_key: output_key_bytes,
                output_key_parity: match parity {
                    Parity::Even => 0,
                    Parity::Odd => 1,
                },
                script_pubkey,
                leaves: vec![leaf],
            },
        }
    }

    fn compile_program_to_transaction(program: &Program) -> Transaction {
        let mut compiler = Compiler::new();
        let compiled = compiler.compile(program).expect("program compiles");
        let bytes = match compiled.actions.first() {
            Some(CompiledAction::SendRawMessage(_, cmd, payload)) if cmd == "tx" => payload.clone(),
            other => panic!("unexpected action {:?}", other),
        };
        Transaction::consensus_decode(&mut bytes.as_slice()).expect("tx decode")
    }

    fn serialize_tx_hex(tx: &Transaction) -> String {
        bitcoin::consensus::serialize(tx)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn build_key_path_program(taproot_txo: TaprootTxo) -> Program {
        let mut builder = ProgramBuilder::new(test_context());
        let connection = builder.force_append_expect_output(vec![], Operation::LoadConnection(0));
        let taproot_var = builder
            .force_append_expect_output(vec![], Operation::LoadTaprootTxo { txo: taproot_txo });
        let txo_var =
            builder.force_append_expect_output(vec![taproot_var.index], Operation::TaprootTxoToTxo);
        let tx = build_single_input_transaction(&mut builder, txo_var.index, 1_000);
        builder.force_append(vec![connection.index, tx.index], Operation::SendTx);
        builder.finalize().expect("valid program")
    }

    fn build_script_path_program(taproot_txo: TaprootTxo) -> Program {
        let mut builder = ProgramBuilder::new(test_context());
        let connection = builder.force_append_expect_output(vec![], Operation::LoadConnection(0));
        let taproot_var = builder
            .force_append_expect_output(vec![], Operation::LoadTaprootTxo { txo: taproot_txo });
        let spend_info_var = builder
            .force_append_expect_output(vec![taproot_var.index], Operation::TaprootTxoToSpendInfo);
        let leaf_var = builder.force_append_expect_output(
            vec![spend_info_var.index],
            Operation::TaprootSpendInfoSelectLeaf { index: 0 },
        );
        let txo_var =
            builder.force_append_expect_output(vec![taproot_var.index], Operation::TaprootTxoToTxo);
        let txo_var = builder.force_append_expect_output(
            vec![txo_var.index, leaf_var.index],
            Operation::TaprootTxoUseLeaf,
        );
        let tx = build_single_input_transaction(&mut builder, txo_var.index, 1_500);
        builder.force_append(vec![connection.index, tx.index], Operation::SendTx);
        builder.finalize().expect("valid program")
    }

    fn build_single_input_transaction(
        builder: &mut ProgramBuilder,
        txo_index: usize,
        output_amount: u64,
    ) -> crate::builder::IndexedVariable {
        let tx_version = builder.force_append_expect_output(vec![], Operation::LoadTxVersion(2));
        let lock_time = builder.force_append_expect_output(vec![], Operation::LoadLockTime(0));
        let mut_tx = builder.force_append_expect_output(
            vec![tx_version.index, lock_time.index],
            Operation::BeginBuildTx,
        );

        let mut_inputs = builder.force_append_expect_output(vec![], Operation::BeginBuildTxInputs);
        let sequence =
            builder.force_append_expect_output(vec![], Operation::LoadSequence(0xffff_fffe));
        builder.force_append(
            vec![mut_inputs.index, txo_index, sequence.index],
            Operation::AddTxInput,
        );
        let const_inputs =
            builder.force_append_expect_output(vec![mut_inputs.index], Operation::EndBuildTxInputs);

        let mut_outputs = builder
            .force_append_expect_output(vec![const_inputs.index], Operation::BeginBuildTxOutputs);
        let scripts = builder.force_append_expect_output(vec![], Operation::BuildPayToAnchor);
        let amount = builder.force_append_expect_output(vec![], Operation::LoadAmount(output_amount));
        builder.force_append(
            vec![mut_outputs.index, scripts.index, amount.index],
            Operation::AddTxOutput,
        );
        let const_outputs = builder
            .force_append_expect_output(vec![mut_outputs.index], Operation::EndBuildTxOutputs);

        builder.force_append_expect_output(
            vec![mut_tx.index, const_inputs.index, const_outputs.index],
            Operation::EndBuildTx,
        )
    }

    fn test_context() -> ProgramContext {
        ProgramContext {
            num_nodes: 1,
            num_connections: 1,
            timestamp: 0,
        }
    }
}

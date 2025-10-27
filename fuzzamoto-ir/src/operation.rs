use crate::{ProgramValidationError, Variable};

use std::{fmt, time::Duration};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Hash)]
pub enum Operation {
    /// No operation (used for minimization)
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
    LoadAmount(u64),
    LoadSize(usize), // Size in bytes
    LoadTxVersion(u32),
    LoadBlockVersion(i32),
    LoadLockTime(u32),
    LoadSequence(u32),
    LoadBlockHeight(u32),
    LoadCompactFilterType(u8),
    LoadPrivateKey([u8; 32]),
    LoadSigHashFlags(u8),
    LoadTxo {
        outpoint: ([u8; 32], u32),
        value: u64,
        script_pubkey: Vec<u8>,

        spending_script_sig: Vec<u8>,
        spending_witness: Vec<Vec<u8>>,
    },
    LoadHeader {
        prev: [u8; 32],
        merkle_root: [u8; 32],
        nonce: u32,
        bits: u32,
        time: u32,
        version: i32,
        height: u32,
    },

    /// Send a message given a connection, message type and bytes
    SendRawMessage,
    /// Advance a time variable by a given duration
    AdvanceTime,
    /// Set mock time
    SetTime,

    /// Script building operations
    BuildRawScripts,
    BuildPayToWitnessScriptHash,
    // TODO: BuildPayToTaproot,
    // TODO: BuildPayToBareMulti, BeginMultiSig, EndMultiSig
    BuildPayToPubKey,
    BuildPayToPubKeyHash,
    BuildPayToWitnessPubKeyHash,
    BuildPayToScriptHash,
    BuildOpReturnScripts,
    BuildPayToAnchor,

    /// Transaction building operations
    BeginWitnessStack,
    EndWitnessStack,
    AddWitness,
    BeginBuildTx,
    EndBuildTx,
    BeginBuildTxInputs,
    EndBuildTxInputs,
    BeginBuildTxOutputs,
    EndBuildTxOutputs,
    AddTxOutput,
    AddTxInput,
    TakeTxo,

    /// Coinbase-specific building operations
    BeginBuildCoinbaseTx,
    EndBuildCoinbaseTx,
    BuildCoinbaseTxInput,
    BeginBuildCoinbaseTxOutputs,
    EndBuildCoinbaseTxOutputs,
    AddCoinbaseTxOutput,

    /// Block building
    BeginBlockTransactions,
    EndBlockTransactions,
    BuildBlock,
    AddTx,
    AddCoinbaseTx,

    /// Inventory building
    BeginBuildInventory,
    EndBuildInventory,
    AddCompactBlockInv,
    AddTxidInv,             // Tx by txid without witness
    AddTxidWithWitnessInv,  // Tx by txid with witness
    AddWtxidInv,            // Tx by wtxid with witness
    AddBlockInv,            // Block by hash without witness
    AddBlockWithWitnessInv, // Block by hash with witness
    AddFilteredBlockInv,    // SPV proof by block hash for txs matching filter

    /// Message sending
    SendGetData,
    SendInv,
    SendTx,
    SendTxNoWit,
    SendHeader,
    SendBlock,
    SendBlockNoWit,
    SendGetCFilters,
    SendGetCFHeaders,
    SendGetCFCheckpt,
    // TODO: SendCompactBlock
    // TODO: SendGetBlockTxn
    // TODO: SendBlockTxn
    // TODO: SendGetBlocks
    // TODO: SendGetHeaders
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
            Operation::LoadBlockHeight(height) => write!(f, "LoadBlockHeight({})", height),
            Operation::LoadCompactFilterType(filter_type) => {
                write!(f, "LoadCompactFilterType({})", filter_type)
            }
            Operation::SendRawMessage => write!(f, "SendRawMessage"),
            Operation::AdvanceTime => write!(f, "AdvanceTime"),
            Operation::LoadTime(time) => write!(f, "LoadTime({})", time),
            Operation::SetTime => write!(f, "SetTime"),
            Operation::BuildRawScripts => write!(f, "BuildRawScripts"),
            Operation::BuildPayToWitnessScriptHash => write!(f, "BuildPayToWitnessScriptHash"),
            Operation::BuildPayToScriptHash => write!(f, "BuildPayToScriptHash"),
            Operation::BuildOpReturnScripts => write!(f, "BuildOpReturnScripts"),
            Operation::BuildPayToAnchor => write!(f, "BuildPayToAnchor"),
            Operation::BuildPayToPubKey => write!(f, "BuildPayToPubKey"),
            Operation::BuildPayToPubKeyHash => write!(f, "BuildPayToPubKeyHash"),
            Operation::BuildPayToWitnessPubKeyHash => write!(f, "BuildPayToWitnessPubKeyHash"),
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
            Operation::LoadHeader {
                prev,
                merkle_root,
                nonce,
                bits,
                time,
                version,
                height,
            } => write!(
                f,
                "LoadHeader({}, {}, {}, {}, {}, {}, {})",
                hex_string(prev),
                hex_string(merkle_root),
                nonce,
                bits,
                time,
                version,
                height
            ),
            Operation::LoadAmount(amount) => write!(f, "LoadAmount({})", amount),
            Operation::LoadTxVersion(version) => write!(f, "LoadTxVersion({})", version),
            Operation::LoadBlockVersion(version) => write!(f, "LoadBlockVersion({})", version),
            Operation::LoadLockTime(lock_time) => write!(f, "LoadLockTime({})", lock_time),
            Operation::LoadSequence(sequence) => write!(f, "LoadSequence({})", sequence),
            Operation::LoadSize(size) => write!(f, "LoadSize({})", size),
            Operation::LoadPrivateKey(private_key) => {
                write!(f, "LoadPrivateKey({})", hex_string(private_key))
            }
            Operation::LoadSigHashFlags(sig_hash_flags) => {
                write!(f, "LoadSigHashFlags({})", sig_hash_flags)
            }

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

            Operation::BeginBuildCoinbaseTx => write!(f, "BeginBuildCoinbaseTx"),
            Operation::EndBuildCoinbaseTx => write!(f, "EndBuildCoinbaseTx"),
            Operation::BuildCoinbaseTxInput => write!(f, "BuildCoinbaseTxInput"),
            Operation::BeginBuildCoinbaseTxOutputs => write!(f, "BeginBuildCoinbaseTxOutputs"),
            Operation::EndBuildCoinbaseTxOutputs => write!(f, "EndBuildCoinbaseTxOutputs"),
            Operation::AddCoinbaseTxOutput => write!(f, "AddCoinbaseTxOutput"),

            Operation::BeginBuildInventory => write!(f, "BeginBuildInventory"),
            Operation::EndBuildInventory => write!(f, "EndBuildInventory"),
            Operation::AddCompactBlockInv => write!(f, "AddCompactBlockInv"),
            Operation::AddTxidInv => write!(f, "AddTxidInv"),
            Operation::AddTxidWithWitnessInv => write!(f, "AddTxidWithWitnessInv"),
            Operation::AddWtxidInv => write!(f, "AddWtxidInv"),
            Operation::AddBlockInv => write!(f, "AddBlockInv"),
            Operation::AddBlockWithWitnessInv => write!(f, "AddBlockWithWitnessInv"),
            Operation::AddFilteredBlockInv => write!(f, "AddFilteredBlockInv"),

            Operation::BeginBlockTransactions => write!(f, "BeginBlockTransactions"),
            Operation::EndBlockTransactions => write!(f, "EndBlockTransactions"),
            Operation::BuildBlock => write!(f, "BuildBlock"),
            Operation::AddTx => write!(f, "AddTx"),
            Operation::AddCoinbaseTx => write!(f, "AddCoinbaseTx"),

            Operation::SendGetData => write!(f, "SendGetData"),
            Operation::SendInv => write!(f, "SendInv"),
            Operation::SendTx => write!(f, "SendTx"),
            Operation::SendTxNoWit => write!(f, "SendTxNoWit"),
            Operation::SendHeader => write!(f, "SendHeader"),
            Operation::SendBlock => write!(f, "SendBlock"),
            Operation::SendBlockNoWit => write!(f, "SendBlockNoWit"),
            Operation::SendGetCFilters => write!(f, "SendGetCFilters"),
            Operation::SendGetCFHeaders => write!(f, "SendGetCFHeaders"),
            Operation::SendGetCFCheckpt => write!(f, "SendGetCFCheckpt"),
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
            Operation::AddCoinbaseTxOutput if index == 0 => true,
            Operation::TakeTxo if index == 0 => true,
            Operation::AddWitness if index == 0 => true,
            Operation::AddTxidInv if index == 0 => true,
            Operation::AddTxidWithWitnessInv if index == 0 => true,
            Operation::AddWtxidInv if index == 0 => true,
            Operation::AddTx if index == 0 => true,
            Operation::AddCoinbaseTx if index == 0 => true,
            _ => false,
        }
    }

    pub fn is_block_begin(&self) -> bool {
        match self {
            Operation::BeginBuildTx
            | Operation::BeginBuildInventory
            | Operation::BeginBuildTxInputs
            | Operation::BeginBuildTxOutputs
            | Operation::BeginWitnessStack
            | Operation::BeginBlockTransactions
            | Operation::BeginBuildCoinbaseTx
            | Operation::BeginBuildCoinbaseTxOutputs => true,
            // Exhaustive match to fail when new ops are added
            Operation::Nop { .. }
            | Operation::LoadBytes(_)
            | Operation::LoadMsgType(_)
            | Operation::LoadNode(_)
            | Operation::LoadConnection(_)
            | Operation::LoadConnectionType(_)
            | Operation::LoadDuration(_)
            | Operation::LoadBlockHeight(_)
            | Operation::LoadCompactFilterType(_)
            | Operation::SendRawMessage
            | Operation::AdvanceTime
            | Operation::LoadTime(_)
            | Operation::LoadSize(_)
            | Operation::SetTime
            | Operation::BuildPayToWitnessScriptHash
            | Operation::BuildRawScripts
            | Operation::BuildPayToScriptHash
            | Operation::BuildOpReturnScripts
            | Operation::BuildPayToAnchor
            | Operation::BuildPayToPubKey
            | Operation::BuildPayToPubKeyHash
            | Operation::BuildPayToWitnessPubKeyHash
            | Operation::LoadTxo { .. }
            | Operation::LoadHeader { .. }
            | Operation::LoadAmount(..)
            | Operation::LoadTxVersion(..)
            | Operation::LoadBlockVersion(..)
            | Operation::LoadLockTime(..)
            | Operation::LoadSequence(..)
            | Operation::LoadPrivateKey(..)
            | Operation::LoadSigHashFlags(..)
            | Operation::EndBuildTx
            | Operation::EndBuildTxInputs
            | Operation::EndBuildTxOutputs
            | Operation::EndBuildInventory
            | Operation::AddCompactBlockInv
            | Operation::AddTxidInv
            | Operation::AddTxidWithWitnessInv
            | Operation::AddWtxidInv
            | Operation::SendGetData
            | Operation::SendInv
            | Operation::AddTxInput
            | Operation::AddTxOutput
            | Operation::TakeTxo
            | Operation::EndWitnessStack
            | Operation::AddWitness
            | Operation::BuildBlock
            | Operation::AddBlockInv
            | Operation::AddBlockWithWitnessInv
            | Operation::AddFilteredBlockInv
            | Operation::AddTx
            | Operation::AddCoinbaseTx
            | Operation::EndBlockTransactions
            | Operation::SendTx
            | Operation::SendTxNoWit
            | Operation::SendHeader
            | Operation::SendBlock
            | Operation::SendGetCFilters
            | Operation::SendGetCFHeaders
            | Operation::SendGetCFCheckpt
            | Operation::SendBlockNoWit
            | Operation::EndBuildCoinbaseTx
            | Operation::EndBuildCoinbaseTxOutputs
            | Operation::BuildCoinbaseTxInput
            | Operation::AddCoinbaseTxOutput => false,
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
            | (Operation::BeginBuildInventory, Operation::EndBuildInventory)
            | (Operation::BeginWitnessStack, Operation::EndWitnessStack)
            | (Operation::BeginBlockTransactions, Operation::EndBlockTransactions)
            | (Operation::BeginBuildCoinbaseTx, Operation::EndBuildCoinbaseTx)
            | (Operation::BeginBuildCoinbaseTxOutputs, Operation::EndBuildCoinbaseTxOutputs) => {
                true
            }
            _ => false,
        }
    }

    pub fn is_block_end(&self) -> bool {
        match self {
            Operation::EndBuildTx
            | Operation::EndBuildTxInputs
            | Operation::EndBuildTxOutputs
            | Operation::EndBuildInventory
            | Operation::EndWitnessStack
            | Operation::EndBlockTransactions
            | Operation::EndBuildCoinbaseTx
            | Operation::EndBuildCoinbaseTxOutputs => true,
            // Exhaustive match to fail when new ops are added
            Operation::Nop { .. }
            | Operation::LoadBytes(_)
            | Operation::LoadMsgType(_)
            | Operation::LoadNode(_)
            | Operation::LoadConnection(_)
            | Operation::LoadConnectionType(_)
            | Operation::LoadDuration(_)
            | Operation::LoadBlockHeight(_)
            | Operation::LoadCompactFilterType(_)
            | Operation::SendRawMessage
            | Operation::AdvanceTime
            | Operation::LoadTime(_)
            | Operation::LoadSize(_)
            | Operation::SetTime
            | Operation::BuildPayToWitnessScriptHash
            | Operation::BuildRawScripts
            | Operation::BuildPayToScriptHash
            | Operation::BuildOpReturnScripts
            | Operation::BuildPayToAnchor
            | Operation::BuildPayToPubKey
            | Operation::BuildPayToPubKeyHash
            | Operation::BuildPayToWitnessPubKeyHash
            | Operation::LoadTxo { .. }
            | Operation::LoadHeader { .. }
            | Operation::LoadAmount(..)
            | Operation::LoadTxVersion(..)
            | Operation::LoadBlockVersion(..)
            | Operation::LoadLockTime(..)
            | Operation::LoadSequence(..)
            | Operation::LoadPrivateKey(..)
            | Operation::LoadSigHashFlags(..)
            | Operation::BeginBuildTx
            | Operation::BeginBuildTxInputs
            | Operation::BeginBuildTxOutputs
            | Operation::AddTxInput
            | Operation::AddTxOutput
            | Operation::TakeTxo
            | Operation::BeginWitnessStack
            | Operation::AddWitness
            | Operation::BeginBuildInventory
            | Operation::AddCompactBlockInv
            | Operation::AddTxidInv
            | Operation::AddTxidWithWitnessInv
            | Operation::AddWtxidInv
            | Operation::BuildBlock
            | Operation::AddBlockInv
            | Operation::AddBlockWithWitnessInv
            | Operation::AddFilteredBlockInv
            | Operation::AddTx
            | Operation::AddCoinbaseTx
            | Operation::BeginBlockTransactions
            | Operation::SendGetData
            | Operation::SendInv
            | Operation::SendTx
            | Operation::SendTxNoWit
            | Operation::SendHeader
            | Operation::SendBlock
            | Operation::SendGetCFilters
            | Operation::SendGetCFHeaders
            | Operation::SendGetCFCheckpt
            | Operation::SendBlockNoWit
            | Operation::BeginBuildCoinbaseTx
            | Operation::BeginBuildCoinbaseTxOutputs
            | Operation::BuildCoinbaseTxInput
            | Operation::AddCoinbaseTxOutput => false,
        }
    }

    pub fn num_inner_outputs(&self) -> usize {
        self.get_inner_output_variables().len()
    }

    pub fn num_outputs(&self) -> usize {
        self.get_output_variables().len()
    }

    pub fn num_inputs(&self) -> usize {
        self.get_input_variables().len()
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

        let expected_variables = self.get_input_variables();
        check_expected(variables, &expected_variables)
    }

    pub fn get_output_variables(&self) -> Vec<Variable> {
        match self {
            Operation::LoadBytes(_) => vec![Variable::Bytes],
            Operation::LoadMsgType(_) => vec![Variable::MsgType],
            Operation::LoadNode(_) => vec![Variable::Node],
            Operation::LoadConnection(_) => vec![Variable::Connection],
            Operation::LoadConnectionType(_) => vec![Variable::ConnectionType],
            Operation::LoadDuration(_) => vec![Variable::Duration],
            Operation::LoadBlockHeight(_) => vec![Variable::BlockHeight],
            Operation::LoadCompactFilterType(_) => vec![Variable::CompactFilterType],
            Operation::SendRawMessage => vec![],
            Operation::AdvanceTime => vec![Variable::Time],
            Operation::LoadTime(_) => vec![Variable::Time],
            Operation::SetTime => vec![],
            Operation::Nop { outputs, .. } => vec![Variable::Nop; *outputs],
            Operation::BuildPayToWitnessScriptHash => vec![Variable::Scripts],
            Operation::BuildPayToScriptHash => vec![Variable::Scripts],
            Operation::BuildRawScripts => vec![Variable::Scripts],
            Operation::BuildOpReturnScripts => vec![Variable::Scripts],
            Operation::BuildPayToAnchor => vec![Variable::Scripts],
            Operation::BuildPayToPubKey => vec![Variable::Scripts],
            Operation::BuildPayToPubKeyHash => vec![Variable::Scripts],
            Operation::BuildPayToWitnessPubKeyHash => vec![Variable::Scripts],
            Operation::LoadTxo { .. } => vec![Variable::Txo],
            Operation::LoadAmount(..) => vec![Variable::ConstAmount],
            Operation::LoadTxVersion(..) => vec![Variable::TxVersion],
            Operation::LoadBlockVersion(..) => vec![Variable::BlockVersion],
            Operation::LoadLockTime(..) => vec![Variable::LockTime],
            Operation::LoadSequence(..) => vec![Variable::Sequence],
            Operation::LoadSize(..) => vec![Variable::Size],
            Operation::TakeTxo => vec![Variable::Txo],
            Operation::LoadHeader { .. } => vec![Variable::Header],
            Operation::LoadPrivateKey(..) => vec![Variable::PrivateKey],
            Operation::LoadSigHashFlags(..) => vec![Variable::SigHashFlags],
            Operation::BeginBuildTx => vec![],
            Operation::EndBuildTx => vec![Variable::ConstTx],
            Operation::BeginBuildTxInputs => vec![],
            Operation::EndBuildTxInputs => vec![Variable::ConstTxInputs],
            Operation::BeginBuildTxOutputs => vec![],
            Operation::EndBuildTxOutputs => vec![Variable::ConstTxOutputs],
            Operation::AddTxInput => vec![],
            Operation::AddTxOutput => vec![],

            Operation::BeginBuildCoinbaseTx => vec![],
            Operation::EndBuildCoinbaseTx => vec![Variable::CoinbaseTx],
            Operation::BuildCoinbaseTxInput => vec![Variable::CoinbaseInput],
            Operation::BeginBuildCoinbaseTxOutputs => vec![],
            Operation::EndBuildCoinbaseTxOutputs => vec![Variable::ConstTxOutputs],
            Operation::AddCoinbaseTxOutput => vec![],

            Operation::BeginBuildInventory => vec![],
            Operation::EndBuildInventory => vec![Variable::ConstInventory],
            Operation::AddCompactBlockInv => vec![],
            Operation::AddTxidInv => vec![],
            Operation::AddTxidWithWitnessInv => vec![],
            Operation::AddWtxidInv => vec![],
            Operation::AddBlockInv => vec![],
            Operation::AddBlockWithWitnessInv => vec![],
            Operation::AddFilteredBlockInv => vec![],

            Operation::BeginWitnessStack => vec![],
            Operation::EndWitnessStack => vec![Variable::ConstWitnessStack],
            Operation::AddWitness => vec![],

            Operation::BuildBlock => vec![Variable::Header, Variable::Block],
            Operation::AddTx => vec![],
            Operation::AddCoinbaseTx => vec![],
            Operation::EndBlockTransactions => vec![Variable::ConstBlockTransactions],
            Operation::BeginBlockTransactions => vec![],

            Operation::SendTx => vec![],
            Operation::SendTxNoWit => vec![],
            Operation::SendGetData => vec![],
            Operation::SendInv => vec![],
            Operation::SendHeader => vec![],
            Operation::SendBlock => vec![],
            Operation::SendBlockNoWit => vec![],
            Operation::SendGetCFilters => vec![],
            Operation::SendGetCFHeaders => vec![],
            Operation::SendGetCFCheckpt => vec![],
        }
    }

    pub fn get_input_variables(&self) -> Vec<Variable> {
        match self {
            Operation::SendRawMessage => {
                vec![Variable::Connection, Variable::MsgType, Variable::Bytes]
            }
            Operation::AdvanceTime => vec![Variable::Time, Variable::Duration],
            Operation::SetTime => vec![Variable::Time],
            Operation::BuildPayToWitnessScriptHash => {
                vec![Variable::Bytes, Variable::ConstWitnessStack]
            }
            Operation::BuildPayToScriptHash => vec![Variable::Bytes, Variable::ConstWitnessStack],
            Operation::BuildRawScripts => vec![
                Variable::Bytes,
                Variable::Bytes,
                Variable::ConstWitnessStack,
            ],
            Operation::BuildOpReturnScripts => vec![Variable::Size],
            Operation::BuildPayToPubKey => vec![Variable::PrivateKey, Variable::SigHashFlags],
            Operation::BuildPayToPubKeyHash => vec![Variable::PrivateKey, Variable::SigHashFlags],
            Operation::BuildPayToWitnessPubKeyHash => {
                vec![Variable::PrivateKey, Variable::SigHashFlags]
            }
            Operation::BeginBuildTx => vec![Variable::TxVersion, Variable::LockTime],
            Operation::EndBuildTx => vec![
                Variable::MutTx,
                Variable::ConstTxInputs,
                Variable::ConstTxOutputs,
            ],
            Operation::EndBuildTxInputs => vec![Variable::MutTxInputs],
            Operation::EndBuildTxOutputs => vec![Variable::MutTxOutputs],
            Operation::AddTxInput => vec![Variable::MutTxInputs, Variable::Txo, Variable::Sequence],
            Operation::AddTxOutput => vec![
                Variable::MutTxOutputs,
                Variable::Scripts,
                Variable::ConstAmount,
            ],
            Operation::BeginBuildTxOutputs => vec![Variable::ConstTxInputs],
            Operation::BeginBuildCoinbaseTx => vec![Variable::TxVersion, Variable::LockTime],
            Operation::EndBuildCoinbaseTx => vec![
                Variable::MutTx,
                Variable::CoinbaseInput,
                Variable::ConstTxOutputs,
            ],
            Operation::BuildCoinbaseTxInput => vec![Variable::Header, Variable::Sequence],
            Operation::BeginBuildCoinbaseTxOutputs => vec![Variable::CoinbaseInput],
            Operation::EndBuildCoinbaseTxOutputs => vec![Variable::MutTxOutputs],
            Operation::AddCoinbaseTxOutput => vec![
                Variable::MutTxOutputs,
                Variable::Scripts,
                Variable::ConstAmount,
            ],
            Operation::TakeTxo => vec![Variable::ConstTx],
            Operation::AddWitness => vec![Variable::MutWitnessStack, Variable::Bytes],
            Operation::EndWitnessStack => vec![Variable::MutWitnessStack],
            Operation::SendTx | Operation::SendTxNoWit => {
                vec![Variable::Connection, Variable::ConstTx]
            }
            Operation::EndBuildInventory => vec![Variable::MutInventory],
            Operation::AddCompactBlockInv => vec![Variable::MutInventory, Variable::Block],
            Operation::AddTxidInv | Operation::AddTxidWithWitnessInv | Operation::AddWtxidInv => {
                vec![Variable::MutInventory, Variable::ConstTx]
            }
            Operation::AddBlockInv
            | Operation::AddBlockWithWitnessInv
            | Operation::AddFilteredBlockInv => {
                vec![Variable::MutInventory, Variable::Block]
            }
            Operation::BuildBlock => vec![
                Variable::Header,
                Variable::Time,
                Variable::BlockVersion,
                Variable::ConstBlockTransactions,
            ],
            Operation::AddTx => vec![Variable::MutBlockTransactions, Variable::ConstTx],
            Operation::AddCoinbaseTx => vec![Variable::MutBlockTransactions, Variable::CoinbaseTx],
            Operation::EndBlockTransactions => vec![Variable::MutBlockTransactions],
            Operation::SendGetData | Operation::SendInv => {
                vec![Variable::Connection, Variable::ConstInventory]
            }
            Operation::SendHeader => vec![Variable::Connection, Variable::Header],
            Operation::SendBlock | Operation::SendBlockNoWit => {
                vec![Variable::Connection, Variable::Block]
            }
            Operation::SendGetCFilters => vec![
                Variable::Connection,
                Variable::CompactFilterType,
                Variable::BlockHeight,
                Variable::Header,
            ],
            Operation::SendGetCFHeaders => vec![
                Variable::Connection,
                Variable::CompactFilterType,
                Variable::BlockHeight,
                Variable::Header,
            ],
            Operation::SendGetCFCheckpt => vec![
                Variable::Connection,
                Variable::CompactFilterType,
                Variable::Header,
            ],
            // Operations with no inputs
            Operation::Nop { .. }
            | Operation::LoadBytes(_)
            | Operation::LoadMsgType(_)
            | Operation::LoadNode(_)
            | Operation::LoadConnection(_)
            | Operation::LoadConnectionType(_)
            | Operation::LoadDuration(_)
            | Operation::LoadBlockHeight(_)
            | Operation::LoadCompactFilterType(_)
            | Operation::LoadTime(_)
            | Operation::LoadTxo { .. }
            | Operation::LoadHeader { .. }
            | Operation::LoadAmount(..)
            | Operation::LoadTxVersion(..)
            | Operation::LoadBlockVersion(..)
            | Operation::LoadLockTime(..)
            | Operation::LoadSequence(..)
            | Operation::LoadSize(_)
            | Operation::LoadPrivateKey(..)
            | Operation::LoadSigHashFlags(..)
            | Operation::BeginBuildTxInputs
            | Operation::BeginBuildInventory
            | Operation::BeginBlockTransactions
            | Operation::BeginWitnessStack
            | Operation::BuildPayToAnchor => vec![],
        }
    }

    pub fn get_inner_output_variables(&self) -> Vec<Variable> {
        match self {
            Operation::BeginBuildTx => vec![Variable::MutTx],
            Operation::BeginBuildTxInputs => vec![Variable::MutTxInputs],
            Operation::BeginBuildTxOutputs => vec![Variable::MutTxOutputs],
            Operation::BeginWitnessStack => vec![Variable::MutWitnessStack],
            Operation::BeginBuildInventory => vec![Variable::MutInventory],
            Operation::BeginBlockTransactions => vec![Variable::MutBlockTransactions],
            Operation::BeginBuildCoinbaseTx => vec![Variable::MutTx],
            Operation::BeginBuildCoinbaseTxOutputs => vec![Variable::MutTxOutputs],
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
            | Operation::LoadBlockHeight(_)
            | Operation::LoadCompactFilterType(_)
            | Operation::SendRawMessage
            | Operation::AdvanceTime
            | Operation::LoadTime(_)
            | Operation::SetTime
            | Operation::BuildPayToWitnessScriptHash
            | Operation::BuildRawScripts
            | Operation::BuildPayToScriptHash
            | Operation::BuildOpReturnScripts
            | Operation::BuildPayToAnchor
            | Operation::BuildPayToPubKey
            | Operation::BuildPayToPubKeyHash
            | Operation::BuildPayToWitnessPubKeyHash
            | Operation::LoadTxo { .. }
            | Operation::LoadHeader { .. }
            | Operation::LoadAmount(..)
            | Operation::LoadTxVersion(..)
            | Operation::LoadBlockVersion(..)
            | Operation::LoadLockTime(..)
            | Operation::LoadSequence(..)
            | Operation::LoadSize(..)
            | Operation::LoadPrivateKey(..)
            | Operation::LoadSigHashFlags(..)
            | Operation::EndBuildTx
            | Operation::EndBuildTxInputs
            | Operation::EndBuildTxOutputs
            | Operation::AddTxInput
            | Operation::AddTxOutput
            | Operation::TakeTxo
            | Operation::EndWitnessStack
            | Operation::AddWitness
            | Operation::EndBuildInventory
            | Operation::AddCompactBlockInv
            | Operation::AddTxidInv
            | Operation::AddTxidWithWitnessInv
            | Operation::AddWtxidInv
            | Operation::AddBlockInv
            | Operation::AddBlockWithWitnessInv
            | Operation::AddFilteredBlockInv
            | Operation::BuildBlock
            | Operation::AddTx
            | Operation::AddCoinbaseTx
            | Operation::EndBlockTransactions
            | Operation::SendGetData
            | Operation::SendInv
            | Operation::SendTx
            | Operation::SendTxNoWit
            | Operation::SendHeader
            | Operation::SendBlock
            | Operation::SendBlockNoWit
            | Operation::SendGetCFilters
            | Operation::SendGetCFHeaders
            | Operation::SendGetCFCheckpt
            | Operation::EndBuildCoinbaseTx
            | Operation::BuildCoinbaseTxInput
            | Operation::EndBuildCoinbaseTxOutputs
            | Operation::AddCoinbaseTxOutput => vec![],
        }
    }
}

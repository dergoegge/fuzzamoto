use bitcoin::{BlockHash, hashes::Hash};
use rand::{Rng, RngCore, seq::SliceRandom};

use super::GeneratorError;
use crate::{
    CoinbaseTxGenerator, Generator, GeneratorResult, IndexedVariable, Instruction,
    InstructionContext, Operation, PerTestcaseMetadata, ProgramBuilder, Variable,
};
use std::collections::HashMap;
/// `BlockGenerator` generates instructions for creating a new block and sending it to a node
pub struct BlockGenerator {
    coinbase_generator: CoinbaseTxGenerator,
}

pub fn select_header_nth(
    headers: &HashMap<BlockHash, Header>,
    builder: &mut ProgramBuilder,
    meta: Option<&PerTestcaseMetadata>,
    nth: usize,
) -> Option<usize> {
    let meta = meta.as_ref()?;
    let nth = meta.recent_blocks.iter().rev().nth(nth)?;
    if let Some((var, _inst)) = nth.defining_block {
        return Some(var);
    }

    let block_hash = BlockHash::from_slice(&nth.hash).ok()?;
    let header = headers.get(&block_hash)?;
    let var = builder
        .append(Instruction {
            inputs: vec![],
            operation: Operation::LoadHeader {
                prev: header.prev,
                merkle_root: header.merkle_root,
                nonce: header.nonce,
                bits: header.bits,
                time: header.time,
                version: header.version,
                height: header.height,
            },
        })
        .expect("Inserting LoadHeader should always succeed")
        .pop()
        .expect("LoadHeader should always produce a var");
    Some(var.index)
}

pub fn build_block_from_header<R: RngCore>(
    coinbase_generator: &CoinbaseTxGenerator,
    builder: &mut ProgramBuilder,
    rng: &mut R,
    header_var_index: usize,
    meta: Option<&PerTestcaseMetadata>,
) -> Result<(IndexedVariable, IndexedVariable), GeneratorError> {
    let time_var = builder
        .get_random_variable(rng, Variable::Time)
        .ok_or(GeneratorError::MissingVariables)?;
    let mut random_tx_vars = builder.get_random_variables(rng, Variable::ConstTx);
    random_tx_vars.sort_by_key(|tx| tx.index);

    let begin_txs_var =
        builder.force_append_expect_output(vec![], Operation::BeginBlockTransactions);

    for tx_var in random_tx_vars {
        builder.force_append(vec![begin_txs_var.index, tx_var.index], Operation::AddTx);
    }

    let end_txs_var = builder
        .force_append_expect_output(vec![begin_txs_var.index], Operation::EndBlockTransactions);

    let block_version_var =
        builder.force_append_expect_output(vec![], Operation::LoadBlockVersion(5));

    let coinbase_tx_var =
        if let Some(coinbase_var) = builder.get_random_variable(rng, Variable::CoinbaseTx) {
            coinbase_var
        } else {
            coinbase_generator.generate(builder, rng, meta)?;
            builder
                .get_random_variable(rng, Variable::CoinbaseTx)
                .unwrap()
        };

    let block_and_header_var = builder
        .append(Instruction {
            inputs: vec![
                coinbase_tx_var.index,
                header_var_index,
                time_var.index,
                block_version_var.index,
                end_txs_var.index,
            ],
            operation: Operation::BuildBlock,
        })
        .expect("Buildblock should not fail");

    let conn_var = builder.get_or_create_random_connection(rng);
    builder.force_append(
        vec![conn_var.index, block_and_header_var[0].index],
        Operation::SendHeader,
    );
    builder.force_append(
        vec![conn_var.index, block_and_header_var[1].index],
        Operation::SendBlock,
    );
    builder.force_append(
        vec![block_and_header_var[2].index],
        Operation::TakeCoinbaseTxo,
    );

    Ok((
        block_and_header_var[0].clone(),
        block_and_header_var[1].clone(),
    ))
}

impl<R: RngCore> Generator<R> for BlockGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        meta: Option<&PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let header_var = if rng.gen_bool(0.5) {
            builder.get_random_variable(rng, Variable::Header)
        } else {
            builder.get_nearest_sent_header()
        }
        .ok_or(GeneratorError::MissingVariables)?;

        let (_block, _header) = build_block_from_header(
            &self.coinbase_generator,
            builder,
            rng,
            header_var.index,
            meta,
        )?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "BlockGenerator"
    }
}

impl Default for BlockGenerator {
    fn default() -> Self {
        Self {
            coinbase_generator: CoinbaseTxGenerator::default(),
        }
    }
}

/// `TipBlockGenerator` generates instructions for creating a new block on top of the current tip.
pub struct TipBlockGenerator {
    coinbase_generator: CoinbaseTxGenerator,
    headers: HashMap<BlockHash, Header>,
}

impl<R: RngCore> Generator<R> for TipBlockGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        meta: Option<&PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let Some(header_var) = select_header_nth(&self.headers, builder, meta, 0) else {
            return Ok(());
        };

        let (_header, _block) =
            build_block_from_header(&self.coinbase_generator, builder, rng, header_var, meta)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TipBlockGenerator"
    }

    fn choose_index(
        &self,
        program: &crate::Program,
        rng: &mut R,
        meta: Option<&PerTestcaseMetadata>,
    ) -> Option<usize> {
        if let Some(meta) = meta.as_ref()
            && let Some(nth) = meta.recent_blocks.iter().rev().nth(0)
            && let Some((_, inst)) = nth.defining_block
        {
            Some(inst + 1)
        } else {
            program
                .get_random_instruction_index(rng, <Self as Generator<R>>::requested_context(self))
        }
    }
}

impl TipBlockGenerator {
    pub fn new(headers: Vec<Header>) -> Self {
        let mut mp = HashMap::new();
        for header in headers {
            let block_hash = header.block_hash();
            mp.insert(block_hash, header);
        }

        Self {
            coinbase_generator: CoinbaseTxGenerator::default(),
            headers: mp,
        }
    }
}

pub struct ReorgBlockGenerator {
    coinbase_generator: CoinbaseTxGenerator,
    headers: HashMap<BlockHash, Header>,
}

impl<R: RngCore> Generator<R> for ReorgBlockGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        meta: Option<&PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let Some(header_var) = select_header_nth(&self.headers, builder, meta, 1) else {
            return Ok(());
        };

        let (first_header, _) =
            build_block_from_header(&self.coinbase_generator, builder, rng, header_var, meta)?;
        let (_, _) = build_block_from_header(
            &self.coinbase_generator,
            builder,
            rng,
            first_header.index,
            meta,
        )?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "ReorgBlockGenerator"
    }

    fn choose_index(
        &self,
        program: &crate::Program,
        rng: &mut R,
        meta: Option<&PerTestcaseMetadata>,
    ) -> Option<usize> {
        if let Some(meta) = meta.as_ref()
            && let Some(nth) = meta.recent_blocks.iter().rev().nth(1)
            && let Some((_, inst)) = nth.defining_block
        {
            Some(inst + 1)
        } else {
            program
                .get_random_instruction_index(rng, <Self as Generator<R>>::requested_context(self))
        }
    }
}

impl Default for ReorgBlockGenerator {
    fn default() -> Self {
        Self {
            coinbase_generator: CoinbaseTxGenerator::default(),
            headers: HashMap::new(),
        }
    }
}

impl ReorgBlockGenerator {
    pub fn new(headers: Vec<Header>) -> Self {
        let mut mp = HashMap::new();
        for header in headers {
            let block_hash = header.block_hash();
            mp.insert(block_hash, header);
        }

        Self {
            coinbase_generator: CoinbaseTxGenerator::default(),
            headers: mp,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Header {
    pub prev: [u8; 32],
    pub merkle_root: [u8; 32],
    pub nonce: u32,
    pub bits: u32,
    pub time: u32,
    pub version: i32,
    pub height: u32,
}

impl Header {
    pub fn to_bitcoin_header(&self) -> bitcoin::block::Header {
        bitcoin::block::Header {
            version: bitcoin::block::Version::from_consensus(self.version),
            prev_blockhash: bitcoin::BlockHash::from_slice(&self.prev).unwrap(),
            merkle_root: bitcoin::TxMerkleNode::from_slice(&self.merkle_root).unwrap(),
            bits: bitcoin::CompactTarget::from_consensus(self.bits),
            nonce: self.nonce,
            time: self.time,
        }
    }

    pub fn block_hash(&self) -> BlockHash {
        let bitcoin_header = self.to_bitcoin_header();
        bitcoin_header.block_hash()
    }
}

pub struct HeaderGenerator {
    pub headers: Vec<Header>,
}

impl HeaderGenerator {
    pub fn new(headers: Vec<Header>) -> Self {
        Self { headers }
    }
}

impl<R: RngCore> Generator<R> for HeaderGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let header = self.headers.choose(rng).unwrap().clone();

        builder.force_append(
            vec![],
            Operation::LoadHeader {
                prev: header.prev,
                merkle_root: header.merkle_root,
                nonce: header.nonce,
                bits: header.bits,
                time: header.time,
                version: header.version,
                height: header.height,
            },
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "HeaderGenerator"
    }
}

#[derive(Default)]
pub struct SendBlockGenerator;

impl<R: RngCore> Generator<R> for SendBlockGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let block_var = builder
            .get_random_variable(rng, Variable::Block)
            .ok_or(GeneratorError::MissingVariables)?;
        let conn_var = builder.get_or_create_random_connection(rng);

        if rng.gen_bool(0.95) {
            builder.force_append(vec![conn_var.index, block_var.index], Operation::SendBlock);
        } else {
            builder.force_append(
                vec![conn_var.index, block_var.index],
                Operation::SendBlockNoWit,
            );
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "SendBlockGenerator"
    }
}

/// `AddTxToBlockGenerator` generates `AddTx` instructions, adding transactions to a block
#[derive(Default)]
pub struct AddTxToBlockGenerator;

impl<R: RngCore> Generator<R> for AddTxToBlockGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let block_var = builder
            .get_nearest_variable(Variable::MutBlockTransactions)
            .ok_or(GeneratorError::MissingVariables)?;
        let mut random_tx_vars = builder.get_random_variables(rng, Variable::ConstTx);
        random_tx_vars.sort_by_key(|tx| tx.index);
        for tx_var in random_tx_vars {
            builder.force_append(vec![block_var.index, tx_var.index], Operation::AddTx);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "AddTxToBlockGenerator"
    }

    fn requested_context(&self) -> InstructionContext {
        InstructionContext::BlockTransactions
    }
}

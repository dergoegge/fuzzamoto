use bitcoin::opcodes::all::{OP_CHECKSIG, OP_PUSHNUM_1};
use rand::{Rng, RngCore, seq::SliceRandom};

use crate::{
    Operation, ProgramBuilder, TaprootTxo,
    builder::IndexedVariable,
    generators::{Generator, GeneratorError, GeneratorResult},
};

/// Generates a simple transaction that spends a context Taproot UTXO via key-path.
pub struct TaprootKeyPathGenerator {
    available_txos: Vec<TaprootTxo>,
}

/// Generates a simple transaction that spends a Taproot UTXO via script-path.
pub struct TaprootScriptPathGenerator {
    available_txos: Vec<TaprootTxo>,
}

/// Builds a new multi-leaf Taproot tree and spends it via a non-default tapleaf.
pub struct TaprootTreeSpendGenerator {
    available_txos: Vec<TaprootTxo>,
}

impl TaprootKeyPathGenerator {
    pub fn new(available_txos: Vec<TaprootTxo>) -> Self {
        Self { available_txos }
    }
}

impl TaprootScriptPathGenerator {
    pub fn new(available_txos: Vec<TaprootTxo>) -> Self {
        Self {
            available_txos: available_txos
                .into_iter()
                .filter(|txo| !txo.spend_info.leaves.is_empty())
                .collect(),
        }
    }
}

impl TaprootTreeSpendGenerator {
    pub fn new(available_txos: Vec<TaprootTxo>) -> Self {
        Self { available_txos }
    }
}

impl<R: RngCore> Generator<R> for TaprootKeyPathGenerator {
    fn generate(&self, builder: &mut ProgramBuilder, rng: &mut R) -> GeneratorResult {
        if self.available_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        }
        let taproot_txo = &self.available_txos[rng.gen_range(0..self.available_txos.len())];

        let taproot_var = builder.force_append_expect_output(
            vec![],
            Operation::LoadTaprootTxo {
                txo: taproot_txo.clone(),
            },
        );
        let txo_var =
            builder.force_append_expect_output(vec![taproot_var.index], Operation::TaprootTxoToTxo);
        let txo_var = maybe_attach_annex(builder, rng, txo_var);

        let tx_version_var =
            builder.force_append_expect_output(vec![], Operation::LoadTxVersion(2));
        let tx_lock_time_var =
            builder.force_append_expect_output(vec![], Operation::LoadLockTime(0));
        let mut_tx_var = builder.force_append_expect_output(
            vec![tx_version_var.index, tx_lock_time_var.index],
            Operation::BeginBuildTx,
        );

        let mut_inputs_var =
            builder.force_append_expect_output(vec![], Operation::BeginBuildTxInputs);
        let sequence_var =
            builder.force_append_expect_output(vec![], Operation::LoadSequence(0xfffffffe));
        builder.force_append(
            vec![mut_inputs_var.index, txo_var.index, sequence_var.index],
            Operation::AddTxInput,
        );
        let inputs_var = builder
            .force_append_expect_output(vec![mut_inputs_var.index], Operation::EndBuildTxInputs);

        let mut_outputs_var = builder
            .force_append_expect_output(vec![inputs_var.index], Operation::BeginBuildTxOutputs);
        let scripts_var = builder.force_append_expect_output(vec![], Operation::BuildPayToAnchor);
        let amount_var =
            builder.force_append_expect_output(vec![], Operation::LoadAmount(taproot_txo.value));
        builder.force_append(
            vec![mut_outputs_var.index, scripts_var.index, amount_var.index],
            Operation::AddTxOutput,
        );
        let outputs_var = builder
            .force_append_expect_output(vec![mut_outputs_var.index], Operation::EndBuildTxOutputs);

        let const_tx_var = builder.force_append_expect_output(
            vec![mut_tx_var.index, inputs_var.index, outputs_var.index],
            Operation::EndBuildTx,
        );

        let connection_var = builder.get_or_create_random_connection(rng);
        builder.force_append(
            vec![connection_var.index, const_tx_var.index],
            Operation::SendTx,
        );

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TaprootKeyPathGenerator"
    }
}

impl<R: RngCore> Generator<R> for TaprootScriptPathGenerator {
    fn generate(&self, builder: &mut ProgramBuilder, rng: &mut R) -> GeneratorResult {
        if self.available_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        }
        let taproot_txo = &self.available_txos[rng.gen_range(0..self.available_txos.len())];

        let taproot_var = builder.force_append_expect_output(
            vec![],
            Operation::LoadTaprootTxo {
                txo: taproot_txo.clone(),
            },
        );
        let spend_info_var = builder
            .force_append_expect_output(vec![taproot_var.index], Operation::TaprootTxoToSpendInfo);
        let leaf_idx = rng.gen_range(0..taproot_txo.spend_info.leaves.len());
        let leaf_var = builder.force_append_expect_output(
            vec![spend_info_var.index],
            Operation::TaprootSpendInfoSelectLeaf { index: leaf_idx },
        );
        let txo_var =
            builder.force_append_expect_output(vec![taproot_var.index], Operation::TaprootTxoToTxo);
        let txo_with_leaf_var = builder.force_append_expect_output(
            vec![txo_var.index, leaf_var.index],
            Operation::TaprootTxoUseLeaf,
        );
        let txo_with_leaf_var = maybe_attach_annex(builder, rng, txo_with_leaf_var);

        let tx_version_var =
            builder.force_append_expect_output(vec![], Operation::LoadTxVersion(2));
        let tx_lock_time_var =
            builder.force_append_expect_output(vec![], Operation::LoadLockTime(0));
        let mut_tx_var = builder.force_append_expect_output(
            vec![tx_version_var.index, tx_lock_time_var.index],
            Operation::BeginBuildTx,
        );

        let mut_inputs_var =
            builder.force_append_expect_output(vec![], Operation::BeginBuildTxInputs);
        let sequence_var =
            builder.force_append_expect_output(vec![], Operation::LoadSequence(0xfffffffe));
        builder.force_append(
            vec![
                mut_inputs_var.index,
                txo_with_leaf_var.index,
                sequence_var.index,
            ],
            Operation::AddTxInput,
        );
        let inputs_var = builder
            .force_append_expect_output(vec![mut_inputs_var.index], Operation::EndBuildTxInputs);

        let mut_outputs_var = builder
            .force_append_expect_output(vec![inputs_var.index], Operation::BeginBuildTxOutputs);
        let scripts_var = builder.force_append_expect_output(vec![], Operation::BuildPayToAnchor);
        let amount_var =
            builder.force_append_expect_output(vec![], Operation::LoadAmount(taproot_txo.value));
        builder.force_append(
            vec![mut_outputs_var.index, scripts_var.index, amount_var.index],
            Operation::AddTxOutput,
        );
        let outputs_var = builder
            .force_append_expect_output(vec![mut_outputs_var.index], Operation::EndBuildTxOutputs);

        let const_tx_var = builder.force_append_expect_output(
            vec![mut_tx_var.index, inputs_var.index, outputs_var.index],
            Operation::EndBuildTx,
        );

        let connection_var = builder.get_or_create_random_connection(rng);
        builder.force_append(
            vec![connection_var.index, const_tx_var.index],
            Operation::SendTx,
        );

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TaprootScriptPathGenerator"
    }
}

impl<R: RngCore> Generator<R> for TaprootTreeSpendGenerator {
    fn generate(&self, builder: &mut ProgramBuilder, rng: &mut R) -> GeneratorResult {
        const MIN_PARENT_FEE: u64 = 500; // leave room for child fees
        let taproot_txo = self
            .available_txos
            .choose(rng)
            .ok_or(GeneratorError::MissingVariables)?;

        if taproot_txo.value <= MIN_PARENT_FEE {
            return Err(GeneratorError::MissingVariables);
        }

        let taproot_var = builder.force_append_expect_output(
            vec![],
            Operation::LoadTaprootTxo {
                txo: taproot_txo.clone(),
            },
        );
        let keypair_var = builder
            .force_append_expect_output(vec![taproot_var.index], Operation::TaprootTxoToKeypair);
        let funding_txo_var =
            builder.force_append_expect_output(vec![taproot_var.index], Operation::TaprootTxoToTxo);

        let mut_tree_var = builder.force_append_expect_output(vec![], Operation::BeginTaprootTree);
        let leaf_count = rng.gen_range(2..=4);
        for _ in 0..leaf_count {
            maybe_insert_hidden_nodes(builder, rng, mut_tree_var.index);
            let script_var = builder
                .force_append_expect_output(vec![], Operation::LoadBytes(random_tapscript(rng)));
            let version_var = builder.force_append_expect_output(
                vec![],
                Operation::LoadTaprootLeafVersion(random_leaf_version(rng)),
            );
            builder.force_append(
                vec![mut_tree_var.index, script_var.index, version_var.index],
                Operation::AddTapLeaf {
                    depth: random_leaf_depth(rng),
                },
            );
        }
        let spend_info_var = builder.force_append_expect_output(
            vec![mut_tree_var.index, keypair_var.index],
            Operation::EndTaprootTree,
        );
        let scripts_var = builder
            .force_append_expect_output(vec![spend_info_var.index], Operation::BuildPayToTaproot);

        let parent_value = taproot_txo.value.saturating_sub(MIN_PARENT_FEE);
        if parent_value == 0 {
            return Err(GeneratorError::MissingVariables);
        }
        // Parent tx pays to the newly constructed Taproot output.
        let parent_tx = build_single_output_tx(
            builder,
            funding_txo_var.index,
            scripts_var.index,
            parent_value,
        );

        // Immediately spend that output via a different tapleaf to cover control blocks.
        let produced_txo =
            builder.force_append_expect_output(vec![parent_tx.index], Operation::TakeTxo);
        let leaf_index = if leaf_count > 1 {
            rng.gen_range(1..leaf_count)
        } else {
            0
        };
        let leaf_var = builder.force_append_expect_output(
            vec![spend_info_var.index],
            Operation::TaprootSpendInfoSelectLeaf { index: leaf_index },
        );
        let mut spend_txo_var = builder.force_append_expect_output(
            vec![produced_txo.index, leaf_var.index],
            Operation::TaprootTxoUseLeaf,
        );
        spend_txo_var = maybe_attach_annex(builder, rng, spend_txo_var);

        let child_scripts = builder.force_append_expect_output(vec![], Operation::BuildPayToAnchor);
        let child_value = parent_value.saturating_sub(500).max(1);
        let child_tx = build_single_output_tx(
            builder,
            spend_txo_var.index,
            child_scripts.index,
            child_value,
        );

        let connection = builder.get_or_create_random_connection(rng);
        builder.force_append(vec![connection.index, parent_tx.index], Operation::SendTx);
        builder.force_append(vec![connection.index, child_tx.index], Operation::SendTx);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TaprootTreeSpendGenerator"
    }
}

/// When enabled, insert `LoadTaprootAnnex`/`TaprootTxoUseAnnex` so the spend carries an annex.
fn maybe_attach_annex<R: RngCore>(
    builder: &mut ProgramBuilder,
    rng: &mut R,
    txo_var: IndexedVariable,
) -> IndexedVariable {
    if !rng.gen_bool(0.5) {
        return txo_var;
    }

    let annex_var = builder.force_append_expect_output(
        vec![],
        Operation::LoadTaprootAnnex {
            annex: random_annex(rng),
        },
    );
    builder.force_append_expect_output(
        vec![txo_var.index, annex_var.index],
        Operation::TaprootTxoUseAnnex,
    )
}

/// Build a short annex payload that satisfies the BIP341 0x50 prefix rule.
fn random_annex<R: RngCore>(rng: &mut R) -> Vec<u8> {
    let extra_len = rng.gen_range(0..=64);
    let mut annex = Vec::with_capacity(1 + extra_len);
    annex.push(0x50);
    for _ in 0..extra_len {
        annex.push(rng.r#gen());
    }
    annex
}

/// Pick an arbitrary tapscript leaf version in the v1 range.
fn random_leaf_version<R: RngCore>(rng: &mut R) -> u8 {
    *[0xC0u8, 0xC2, 0xC4, 0xD0].choose(rng).unwrap()
}

/// Emit lightweight tapscripts so we mix success, CHECKSIG, and OP_TRUE leaves.
fn random_tapscript<R: RngCore>(rng: &mut R) -> Vec<u8> {
    match rng.gen_range(0..3) {
        0 => vec![OP_PUSHNUM_1.to_u8()],
        1 => {
            let mut script = Vec::with_capacity(34);
            script.push(32);
            for _ in 0..32 {
                script.push(rng.r#gen());
            }
            script.push(OP_CHECKSIG.to_u8());
            script
        }
        _ => vec![0x50],
    }
}

fn random_leaf_depth<R: RngCore>(rng: &mut R) -> u8 {
    rng.gen_range(0..=3)
}

fn random_node_hash<R: RngCore>(rng: &mut R) -> [u8; 32] {
    let mut hash = [0u8; 32];
    rng.fill_bytes(&mut hash);
    hash
}

fn maybe_insert_hidden_nodes<R: RngCore>(
    builder: &mut ProgramBuilder,
    rng: &mut R,
    tree_var_index: usize,
) {
    const MAX_HIDDEN_NODES: usize = 2;
    const MIN_DEPTH: u8 = 1;
    const MAX_DEPTH: u8 = 3;

    if !rng.gen_bool(0.5) {
        return;
    }

    let hidden_count = rng.gen_range(1..=MAX_HIDDEN_NODES);
    for _ in 0..hidden_count {
        builder.force_append(
            vec![tree_var_index],
            Operation::AddTaprootHiddenNode {
                depth: rng.gen_range(MIN_DEPTH..=MAX_DEPTH),
                hash: random_node_hash(rng),
            },
        );
    }
}

/// Convenience wrapper for creating a single-input/single-output transaction.
fn build_single_output_tx(
    builder: &mut ProgramBuilder,
    funding_txo_index: usize,
    scripts_index: usize,
    amount: u64,
) -> IndexedVariable {
    let tx_version_var = builder.force_append_expect_output(vec![], Operation::LoadTxVersion(2));
    let tx_lock_time_var = builder.force_append_expect_output(vec![], Operation::LoadLockTime(0));
    let mut_tx_var = builder.force_append_expect_output(
        vec![tx_version_var.index, tx_lock_time_var.index],
        Operation::BeginBuildTx,
    );

    let mut_inputs_var = builder.force_append_expect_output(vec![], Operation::BeginBuildTxInputs);
    let sequence_var =
        builder.force_append_expect_output(vec![], Operation::LoadSequence(0xfffffffe));
    builder.force_append(
        vec![mut_inputs_var.index, funding_txo_index, sequence_var.index],
        Operation::AddTxInput,
    );
    let inputs_var =
        builder.force_append_expect_output(vec![mut_inputs_var.index], Operation::EndBuildTxInputs);

    let mut_outputs_var =
        builder.force_append_expect_output(vec![inputs_var.index], Operation::BeginBuildTxOutputs);
    let amount_var = builder.force_append_expect_output(vec![], Operation::LoadAmount(amount));
    builder.force_append(
        vec![mut_outputs_var.index, scripts_index, amount_var.index],
        Operation::AddTxOutput,
    );
    let outputs_var = builder
        .force_append_expect_output(vec![mut_outputs_var.index], Operation::EndBuildTxOutputs);

    builder.force_append_expect_output(
        vec![mut_tx_var.index, inputs_var.index, outputs_var.index],
        Operation::EndBuildTx,
    )
}

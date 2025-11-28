use bitcoin::{
    opcodes::all::{OP_CHECKSIG, OP_PUSHNUM_1},
    taproot::LeafVersion,
};
use rand::{Rng, RngCore, seq::SliceRandom};

use crate::{
    Operation, ProgramBuilder,
    builder::IndexedVariable,
    generators::{Generator, GeneratorError, GeneratorResult},
};

/// Generates a simple transaction that builds and spends a Taproot output via key-path.
pub struct TaprootKeyPathGenerator;

/// Generates a simple transaction that builds and spends a Taproot output via script-path.
pub struct TaprootScriptPathGenerator;

/// Builds a new multi-leaf Taproot tree and spends it via a selected tapleaf.
pub struct TaprootTreeSpendGenerator;

impl Default for TaprootKeyPathGenerator {
    fn default() -> Self {
        Self
    }
}

impl Default for TaprootScriptPathGenerator {
    fn default() -> Self {
        Self
    }
}

impl Default for TaprootTreeSpendGenerator {
    fn default() -> Self {
        Self
    }
}

impl<R: RngCore> Generator<R> for TaprootKeyPathGenerator {
    fn generate(&self, builder: &mut ProgramBuilder, rng: &mut R) -> GeneratorResult {
        let funding_txos = builder.get_random_utxos(rng);
        if funding_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        }
        let funding_txo = funding_txos[rng.gen_range(0..funding_txos.len())].clone();

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
            vec![mut_inputs_var.index, funding_txo.index, sequence_var.index],
            Operation::AddTxInput,
        );
        let inputs_var = builder
            .force_append_expect_output(vec![mut_inputs_var.index], Operation::EndBuildTxInputs);

        let mut_outputs_var = builder
            .force_append_expect_output(vec![inputs_var.index], Operation::BeginBuildTxOutputs);

        let keypair_var = builder.force_append_expect_output(
            vec![],
            Operation::BuildTaprootKeypair {
                secret_key: gen_secret_key_bytes(rng),
            },
        );
        let mut_tree_var = builder.force_append_expect_output(vec![], Operation::BeginTaprootTree);
        let spend_info_var = builder.force_append_expect_output(
            vec![mut_tree_var.index, keypair_var.index],
            Operation::EndTaprootTree {
                selected_leaf_index: None,
            },
        );
        let scripts_var = builder
            .force_append_expect_output(vec![spend_info_var.index], Operation::BuildPayToTaproot);

        let amount_var = builder.force_append_expect_output(vec![], Operation::LoadAmount(50_000));
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

        let produced_txo =
            builder.force_append_expect_output(vec![const_tx_var.index], Operation::TakeTxo);

        let child_tx = build_child_tx(builder, produced_txo, rng);

        let connection_var = builder.get_or_create_random_connection(rng);
        builder.force_append(
            vec![connection_var.index, const_tx_var.index],
            Operation::SendTx,
        );
        builder.force_append(
            vec![connection_var.index, child_tx.index],
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
        let funding_txos = builder.get_random_utxos(rng);
        if funding_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        }
        let funding_txo = funding_txos[rng.gen_range(0..funding_txos.len())].clone();

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
            vec![mut_inputs_var.index, funding_txo.index, sequence_var.index],
            Operation::AddTxInput,
        );
        let inputs_var = builder
            .force_append_expect_output(vec![mut_inputs_var.index], Operation::EndBuildTxInputs);

        let mut_outputs_var = builder
            .force_append_expect_output(vec![inputs_var.index], Operation::BeginBuildTxOutputs);

        let keypair_var = builder.force_append_expect_output(
            vec![],
            Operation::BuildTaprootKeypair {
                secret_key: gen_secret_key_bytes(rng),
            },
        );
        let mut_tree_var = builder.force_append_expect_output(vec![], Operation::BeginTaprootTree);
        let script_var =
            builder.force_append_expect_output(vec![], Operation::LoadBytes(random_tapscript(rng)));
        let version_var = builder.force_append_expect_output(
            vec![],
            Operation::LoadTaprootLeafVersion(LeafVersion::TapScript.to_consensus()),
        );
        builder.force_append(
            vec![mut_tree_var.index, script_var.index, version_var.index],
            Operation::AddTapLeaf {
                depth: random_leaf_depth(rng, false),
            },
        );
        let spend_info_var = builder.force_append_expect_output(
            vec![mut_tree_var.index, keypair_var.index],
            Operation::EndTaprootTree {
                selected_leaf_index: Some(0),
            },
        );
        let scripts_var = builder
            .force_append_expect_output(vec![spend_info_var.index], Operation::BuildPayToTaproot);

        let amount_var = builder.force_append_expect_output(vec![], Operation::LoadAmount(60_000));
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

        let produced_txo =
            builder.force_append_expect_output(vec![const_tx_var.index], Operation::TakeTxo);
        let produced_txo = maybe_attach_annex(builder, rng, produced_txo);

        let child_tx = build_child_tx(builder, produced_txo, rng);

        let connection_var = builder.get_or_create_random_connection(rng);
        builder.force_append(
            vec![connection_var.index, const_tx_var.index],
            Operation::SendTx,
        );
        builder.force_append(
            vec![connection_var.index, child_tx.index],
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
        let funding_txos = builder.get_random_utxos(rng);
        if funding_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        }
        let funding_txo = funding_txos
            .choose(rng)
            .cloned()
            .ok_or(GeneratorError::MissingVariables)?;

        let mut_tree_var = builder.force_append_expect_output(vec![], Operation::BeginTaprootTree);
        let leaf_count = rng.gen_range(2..=4);
        let mut deepest_leaf_depth = 0u8;
        let mut saw_non_default_version = false;
        for leaf_idx in 0..leaf_count {
            maybe_insert_hidden_nodes(builder, rng, mut_tree_var.index);

            let script_var = builder
                .force_append_expect_output(vec![], Operation::LoadBytes(random_tapscript(rng)));
            let (candidate_version, is_non_default) = random_leaf_version(rng);
            let mut final_version = candidate_version;
            let mut produced_non_default = is_non_default;
            if leaf_idx == leaf_count - 1 && !saw_non_default_version && !produced_non_default {
                final_version = pick_strict_non_default_version(rng);
                produced_non_default = true;
            }
            saw_non_default_version |= produced_non_default;
            let version_var = builder.force_append_expect_output(
                vec![],
                Operation::LoadTaprootLeafVersion(final_version),
            );

            let depth = if leaf_idx == 0 {
                random_leaf_depth(rng, true)
            } else {
                random_leaf_depth(rng, false)
            };
            deepest_leaf_depth = deepest_leaf_depth.max(depth);

            builder.force_append(
                vec![mut_tree_var.index, script_var.index, version_var.index],
                Operation::AddTapLeaf { depth },
            );
        }
        // Inject an additional hidden node near the deepest visible leaf so the resulting control
        // block includes multiple merkle branch hashes.
        maybe_attach_deep_hidden_node(builder, rng, mut_tree_var.index, deepest_leaf_depth);
        let keypair_var = builder.force_append_expect_output(
            vec![],
            Operation::BuildTaprootKeypair {
                secret_key: gen_secret_key_bytes(rng),
            },
        );
        let spend_info_var = builder.force_append_expect_output(
            vec![mut_tree_var.index, keypair_var.index],
            Operation::EndTaprootTree {
                selected_leaf_index: Some(1.min(leaf_count - 1) as u8),
            },
        );
        let scripts_var = builder
            .force_append_expect_output(vec![spend_info_var.index], Operation::BuildPayToTaproot);

        const PARENT_VALUE_SATS: u64 = 80_000;
        let parent_value = PARENT_VALUE_SATS.saturating_sub(MIN_PARENT_FEE);
        if parent_value == 0 {
            return Err(GeneratorError::MissingVariables);
        }
        // Parent tx pays to the newly constructed Taproot output.
        let parent_tx =
            build_single_output_tx(builder, funding_txo.index, scripts_var.index, parent_value);

        // Immediately spend that output; leaf choice is baked into spend_info (first leaf).
        let produced_txo =
            builder.force_append_expect_output(vec![parent_tx.index], Operation::TakeTxo);
        let spend_txo_var = maybe_attach_annex(builder, rng, produced_txo);

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

fn build_child_tx<R: RngCore>(
    builder: &mut ProgramBuilder,
    funding_txo: IndexedVariable,
    rng: &mut R,
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
        vec![mut_inputs_var.index, funding_txo.index, sequence_var.index],
        Operation::AddTxInput,
    );
    let inputs_var =
        builder.force_append_expect_output(vec![mut_inputs_var.index], Operation::EndBuildTxInputs);

    let mut_outputs_var =
        builder.force_append_expect_output(vec![inputs_var.index], Operation::BeginBuildTxOutputs);
    let scripts_var = builder.force_append_expect_output(vec![], Operation::BuildPayToAnchor);
    let amount_var = builder
        .force_append_expect_output(vec![], Operation::LoadAmount(rng.gen_range(5_000..20_000)));
    builder.force_append(
        vec![mut_outputs_var.index, scripts_var.index, amount_var.index],
        Operation::AddTxOutput,
    );
    let outputs_var = builder
        .force_append_expect_output(vec![mut_outputs_var.index], Operation::EndBuildTxOutputs);

    builder.force_append_expect_output(
        vec![mut_tx_var.index, inputs_var.index, outputs_var.index],
        Operation::EndBuildTx,
    )
}

fn gen_secret_key_bytes<R: RngCore>(rng: &mut R) -> [u8; 32] {
    loop {
        let mut secret = [0u8; 32];
        rng.fill_bytes(&mut secret);
        // secp256k1 rejects zero/>=n; let the compiler validate later, but avoid the all-zero case here.
        if secret.iter().any(|&b| b != 0) {
            return secret;
        }
    }
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

/// Returns a consensus tapleaf version plus a flag indicating whether it is non-default.
fn random_leaf_version<R: RngCore>(rng: &mut R) -> (u8, bool) {
    if rng.gen_bool(0.5) {
        (LeafVersion::TapScript.to_consensus(), false)
    } else {
        (pick_strict_non_default_version(rng), true)
    }
}

fn pick_strict_non_default_version<R: RngCore>(rng: &mut R) -> u8 {
    *[0xC2u8, 0xC4, 0xC6, 0xD0].choose(rng).unwrap()
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

fn random_leaf_depth<R: RngCore>(rng: &mut R, ensure_depth: bool) -> u8 {
    if ensure_depth {
        // Force the first leaf to dig deep so each tree yields at least one long merkle branch.
        rng.gen_range(3..=5)
    } else {
        rng.gen_range(0..=5)
    }
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

/// Add a hidden sibling near the deepest visible leaf so script-path witnesses
/// exercise multi-hash control blocks.
fn maybe_attach_deep_hidden_node<R: RngCore>(
    builder: &mut ProgramBuilder,
    rng: &mut R,
    tree_index: usize,
    deepest_leaf_depth: u8,
) {
    if deepest_leaf_depth <= 1 {
        return;
    }
    let depth = deepest_leaf_depth.saturating_sub(1);
    if rng.gen_bool(0.5) {
        builder.force_append(
            vec![tree_index],
            Operation::AddTaprootHiddenNode {
                depth,
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

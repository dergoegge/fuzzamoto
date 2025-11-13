use rand::{Rng, RngCore};

use crate::{
    builder::IndexedVariable, Operation, ProgramBuilder, TaprootTxo,
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

fn random_annex<R: RngCore>(rng: &mut R) -> Vec<u8> {
    let extra_len = rng.gen_range(0..=64);
    let mut annex = Vec::with_capacity(1 + extra_len);
    annex.push(0x50);
    for _ in 0..extra_len {
        annex.push(rng.r#gen());
    }
    annex
}

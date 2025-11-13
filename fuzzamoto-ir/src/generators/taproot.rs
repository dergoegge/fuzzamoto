use rand::{Rng, RngCore};

use crate::{
    Operation, ProgramBuilder, TaprootTxo,
    generators::{Generator, GeneratorError, GeneratorResult},
};

/// Generates a simple transaction that spends a context Taproot UTXO via key-path.
pub struct TaprootKeyPathGenerator {
    available_txos: Vec<TaprootTxo>,
}

impl TaprootKeyPathGenerator {
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

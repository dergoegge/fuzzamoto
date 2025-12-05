use crate::{
    IndexedVariable, Operation, PerTestcaseMetadata,
    generators::{Generator, ProgramBuilder},
};
use bitcoin::opcodes::OP_TRUE;
use rand::{Rng, RngCore, seq::SliceRandom};

use super::{GeneratorError, GeneratorResult};

enum OutputType {
    PayToWitnessScriptHash,
    PayToScriptHash,
    PayToAnchor,
    PayToPubKey,
    PayToPubKeyHash,
    PayToWitnessPubKeyHash,
    OpReturn,
}

fn get_random_output_type<R: RngCore>(rng: &mut R) -> OutputType {
    match rng.gen_range(0..7) {
        0 => OutputType::PayToWitnessScriptHash,
        1 => OutputType::PayToAnchor,
        2 => OutputType::PayToScriptHash,
        3 => OutputType::PayToPubKey,
        4 => OutputType::PayToPubKeyHash,
        5 => OutputType::PayToWitnessPubKeyHash,
        _ => OutputType::OpReturn,
    }
}

fn build_outputs(
    builder: &mut ProgramBuilder,
    mut_outputs_var: &IndexedVariable,
    output_amounts: &[(u64, OutputType)],
    coinbase: bool,
) -> Result<(), GeneratorError> {
    for (amount, output_type) in output_amounts.iter() {
        let scripts_var = match output_type {
            OutputType::PayToWitnessScriptHash => {
                let optrue_bytes_var = builder.force_append_expect_output(
                    vec![],
                    Operation::LoadBytes(vec![OP_TRUE.to_u8()]),
                );
                let mut_witness_stack_var =
                    builder.force_append_expect_output(vec![], Operation::BeginWitnessStack);

                let witness_stack_var = builder.force_append_expect_output(
                    vec![mut_witness_stack_var.index],
                    Operation::EndWitnessStack,
                );

                builder.force_append_expect_output(
                    vec![optrue_bytes_var.index, witness_stack_var.index],
                    Operation::BuildPayToWitnessScriptHash,
                )
            }
            OutputType::PayToAnchor => {
                builder.force_append_expect_output(vec![], Operation::BuildPayToAnchor)
            }
            OutputType::OpReturn => {
                let size_var =
                    builder.force_append_expect_output(vec![], Operation::LoadSize(2 << 15));
                builder.force_append_expect_output(
                    vec![size_var.index],
                    Operation::BuildOpReturnScripts,
                )
            }
            OutputType::PayToScriptHash => {
                let optrue_bytes_var = builder.force_append_expect_output(
                    vec![],
                    Operation::LoadBytes(vec![OP_TRUE.to_u8()]),
                );
                let mut_witness_stack_var =
                    builder.force_append_expect_output(vec![], Operation::BeginWitnessStack);

                let witness_stack_var = builder.force_append_expect_output(
                    vec![mut_witness_stack_var.index],
                    Operation::EndWitnessStack,
                );

                builder.force_append_expect_output(
                    vec![optrue_bytes_var.index, witness_stack_var.index],
                    Operation::BuildPayToScriptHash,
                )
            }
            OutputType::PayToPubKey
            | OutputType::PayToPubKeyHash
            | OutputType::PayToWitnessPubKeyHash => {
                let private_key_var = builder
                    .force_append_expect_output(vec![], Operation::LoadPrivateKey([0x41u8; 32]));
                let sighash_flags_var =
                    builder.force_append_expect_output(vec![], Operation::LoadSigHashFlags(0));

                let op = match output_type {
                    OutputType::PayToPubKey => Operation::BuildPayToPubKey,
                    OutputType::PayToPubKeyHash => Operation::BuildPayToPubKeyHash,
                    OutputType::PayToWitnessPubKeyHash => Operation::BuildPayToWitnessPubKeyHash,
                    _ => unreachable!(),
                };

                builder.force_append_expect_output(
                    vec![private_key_var.index, sighash_flags_var.index],
                    op,
                )
            }
        };

        let amount_var = builder.force_append_expect_output(vec![], Operation::LoadAmount(*amount));

        let add_operation = if coinbase {
            Operation::AddCoinbaseTxOutput
        } else {
            Operation::AddTxOutput
        };

        builder.force_append(
            vec![mut_outputs_var.index, scripts_var.index, amount_var.index],
            add_operation,
        );
    }

    Ok(())
}

fn build_tx(
    builder: &mut ProgramBuilder,
    funding_txos: &[IndexedVariable],
    tx_version: u32,
    output_amounts: &[(u64, OutputType)],
) -> Result<(IndexedVariable, Vec<IndexedVariable>), GeneratorError> {
    let tx_version_var =
        builder.force_append_expect_output(vec![], Operation::LoadTxVersion(tx_version));

    let tx_lock_time_var = builder.force_append_expect_output(vec![], Operation::LoadLockTime(0));
    let mut_tx_var = builder.force_append_expect_output(
        vec![tx_version_var.index, tx_lock_time_var.index],
        Operation::BeginBuildTx,
    );
    let mut_inputs_var = builder.force_append_expect_output(vec![], Operation::BeginBuildTxInputs);

    for funding_txo in funding_txos {
        let sequence_var =
            builder.force_append_expect_output(vec![], Operation::LoadSequence(0xffffffff));
        builder.force_append(
            vec![mut_inputs_var.index, funding_txo.index, sequence_var.index],
            Operation::AddTxInput,
        );
    }

    let inputs_var =
        builder.force_append_expect_output(vec![mut_inputs_var.index], Operation::EndBuildTxInputs);

    let mut_outputs_var =
        builder.force_append_expect_output(vec![inputs_var.index], Operation::BeginBuildTxOutputs);

    let _ = build_outputs(builder, &mut_outputs_var, output_amounts, false);

    let outputs_var = builder
        .force_append_expect_output(vec![mut_outputs_var.index], Operation::EndBuildTxOutputs);

    let const_tx_var = builder.force_append_expect_output(
        vec![mut_tx_var.index, inputs_var.index, outputs_var.index],
        Operation::EndBuildTx,
    );

    // Make every output of the transaction spendable
    let mut outputs = Vec::new();
    for _ in 0..output_amounts.len() {
        outputs
            .push(builder.force_append_expect_output(vec![const_tx_var.index], Operation::TakeTxo));
    }

    Ok((const_tx_var, outputs))
}

/// `SingleTxGenerator` generates instructions for a single new transaction into a program
pub struct SingleTxGenerator;

impl<R: RngCore> Generator<R> for SingleTxGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let funding_txos = builder.get_random_utxos(rng);
        if funding_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        };

        let tx_version = *[1, 2, 3].choose(rng).unwrap();
        let output_amounts = {
            let mut amounts = vec![];
            let num_outputs = rng.gen_range(1..(funding_txos.len() + 5));
            for _i in 0..num_outputs {
                amounts.push((
                    rng.gen_range(5000..100_000_000),
                    get_random_output_type(rng),
                ));
            }
            amounts
        };
        let (const_tx_var, _) = build_tx(builder, &funding_txos, tx_version, &output_amounts)?;

        if rng.gen_bool(0.5) {
            let conn_var = builder.get_or_create_random_connection(rng);

            let mut_inventory_var =
                builder.force_append_expect_output(vec![], Operation::BeginBuildInventory);
            builder.force_append(
                vec![mut_inventory_var.index, const_tx_var.index],
                Operation::AddWtxidInv,
            );
            let const_inventory_var = builder.force_append_expect_output(
                vec![mut_inventory_var.index],
                Operation::EndBuildInventory,
            );

            builder.force_append(
                vec![conn_var.index, const_inventory_var.index],
                Operation::SendInv,
            );
            builder.force_append(vec![conn_var.index, const_tx_var.index], Operation::SendTx);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "SingleTxGenerator"
    }
}

impl Default for SingleTxGenerator {
    fn default() -> Self {
        Self {}
    }
}

/// `OneParentOneChildGenerator` generates instructions for creating a 1P1C package and sending it
/// to a node, with the child tx being the first to be sent
pub struct OneParentOneChildGenerator;

impl<R: RngCore> Generator<R> for OneParentOneChildGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let funding_txos = builder.get_random_utxos(rng);
        if funding_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        };

        let (parent_tx_var, parent_output_vars) = build_tx(
            builder,
            &funding_txos,
            2,
            &[
                (100_000_000, OutputType::PayToWitnessScriptHash),
                (10000, OutputType::PayToAnchor),
            ],
        )?;
        let (child_tx_var, _) = build_tx(
            builder,
            &[parent_output_vars.last().unwrap().clone()],
            2,
            &[(50_000_000, OutputType::PayToWitnessScriptHash)],
        )?;

        let conn_var = builder.get_or_create_random_connection(rng);

        let mut send_tx = |tx_var: IndexedVariable| {
            let mut_inventory_var =
                builder.force_append_expect_output(vec![], Operation::BeginBuildInventory);
            builder.force_append(
                vec![mut_inventory_var.index, tx_var.index],
                Operation::AddWtxidInv,
            );
            let const_inventory_var = builder.force_append_expect_output(
                vec![mut_inventory_var.index],
                Operation::EndBuildInventory,
            );

            builder.force_append(
                vec![conn_var.index, const_inventory_var.index],
                Operation::SendInv,
            );

            builder.force_append(vec![conn_var.index, tx_var.index], Operation::SendTx);
        };
        // Send the child tx first to trigger 1p1c logic
        send_tx(child_tx_var);
        send_tx(parent_tx_var);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "1P1CGenerator"
    }
}

impl Default for OneParentOneChildGenerator {
    fn default() -> Self {
        Self {}
    }
}

/// `LongChainGenerator` generates instructions for creating a chain of 25 transactions and sending
/// them to a node
pub struct LongChainGenerator;

impl<R: RngCore> Generator<R> for LongChainGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let mut funding_txos = builder.get_random_utxos(rng);
        if funding_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        };

        // Create a chain of 25 transactions (default ancestor limit in Bitcoin Core), where each
        // transaction spends the output of the previous transaction
        let mut tx_vars = Vec::new();
        for i in 0..25 {
            let (tx_var, outputs) = build_tx(
                builder,
                &funding_txos,
                2,
                &[(
                    100_000_000 - (i * 100_000),
                    OutputType::PayToWitnessScriptHash,
                )],
            )?;
            tx_vars.push(tx_var);
            funding_txos = outputs;
        }

        let conn_var = builder.get_or_create_random_connection(rng);

        // Send the transactions to the network
        for tx_var in tx_vars {
            let mut_inventory_var =
                builder.force_append_expect_output(vec![], Operation::BeginBuildInventory);
            builder.force_append(
                vec![mut_inventory_var.index, tx_var.index],
                Operation::AddWtxidInv,
            );
            let const_inventory_var = builder.force_append_expect_output(
                vec![mut_inventory_var.index],
                Operation::EndBuildInventory,
            );

            builder.force_append(
                vec![conn_var.index, const_inventory_var.index],
                Operation::SendInv,
            );
            builder.force_append(vec![conn_var.index, tx_var.index], Operation::SendTx);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "LongChainGenerator"
    }
}

impl Default for LongChainGenerator {
    fn default() -> Self {
        Self {}
    }
}

/// `LargeTxGenerator` generates instructions for creating a single large transaction and sending
/// it to a node
pub struct LargeTxGenerator;

impl<R: RngCore> Generator<R> for LargeTxGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let funding_txos = builder.get_random_utxos(rng);
        if funding_txos.is_empty() {
            return Err(GeneratorError::MissingVariables);
        };

        let conn_var = builder.get_or_create_random_connection(rng);

        for utxo in funding_txos {
            let (tx_var, _) = build_tx(
                builder,
                &[utxo.clone()],
                2,
                &[(10_000, OutputType::OpReturn)],
            )?;

            let mut send_tx = |tx_var: IndexedVariable| {
                let mut_inventory_var =
                    builder.force_append_expect_output(vec![], Operation::BeginBuildInventory);
                builder.force_append(
                    vec![mut_inventory_var.index, tx_var.index],
                    Operation::AddWtxidInv,
                );
                let const_inventory_var = builder.force_append_expect_output(
                    vec![mut_inventory_var.index],
                    Operation::EndBuildInventory,
                );

                builder.force_append(
                    vec![conn_var.index, const_inventory_var.index],
                    Operation::SendInv,
                );
                builder.force_append(vec![conn_var.index, tx_var.index], Operation::SendTx);
            };
            send_tx(tx_var);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "LargeTxGenerator"
    }
}

impl Default for LargeTxGenerator {
    fn default() -> Self {
        Self {}
    }
}

/// `CoinbaseTxGenerator` generates instructions for a coinbase tx into a program
pub struct CoinbaseTxGenerator;

impl<R: RngCore> Generator<R> for CoinbaseTxGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let tx_version_var =
            builder.force_append_expect_output(vec![], Operation::LoadTxVersion(1));

        let tx_lock_time_var =
            builder.force_append_expect_output(vec![], Operation::LoadLockTime(0));

        let mut_tx_var = builder.force_append_expect_output(
            vec![tx_version_var.index, tx_lock_time_var.index],
            Operation::BeginBuildCoinbaseTx,
        );

        let sequence_var =
            builder.force_append_expect_output(vec![], Operation::LoadSequence(0xffffffff));

        let coinbase_input_var = builder
            .force_append_expect_output(vec![sequence_var.index], Operation::BuildCoinbaseTxInput);

        let mut_outputs_var = builder.force_append_expect_output(
            vec![coinbase_input_var.index],
            Operation::BeginBuildCoinbaseTxOutputs,
        );
        let output_amounts = {
            let mut amounts = vec![];
            let num_outputs = rng.gen_range(1..10);
            for _i in 0..num_outputs {
                amounts.push((
                    rng.gen_range(5000..100_000_000),
                    get_random_output_type(rng),
                ));
            }
            amounts
        };

        let _ = build_outputs(builder, &mut_outputs_var, &output_amounts, true);

        let outputs_var = builder.force_append_expect_output(
            vec![mut_outputs_var.index],
            Operation::EndBuildCoinbaseTxOutputs,
        );

        builder.force_append(
            vec![
                mut_tx_var.index,
                coinbase_input_var.index,
                outputs_var.index,
            ],
            Operation::EndBuildCoinbaseTx,
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "CoinbaseTxGenerator"
    }
}

impl Default for CoinbaseTxGenerator {
    fn default() -> Self {
        Self {}
    }
}

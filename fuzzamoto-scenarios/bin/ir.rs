#[cfg(feature = "nyx")]
use fuzzamoto_nyx_sys::*;

use bitcoin::hashes::Hash;
use fuzzamoto::{
    connections::Transport,
    fuzzamoto_main,
    scenarios::{
        IgnoredCharacterization, Scenario, ScenarioInput, ScenarioResult, generic::GenericScenario,
    },
    targets::{BitcoinCoreTarget, Target},
};
use fuzzamoto_ir::{
    ProgramContext,
    compiler::{CompiledAction, CompiledProgram},
};

/// `IrScenario` is a scenario with the same context as `GenericScenario` but it operates on
/// `fuzzamoto_ir::CompiledProgram`s as input.
struct IrScenario<TX: Transport, T: Target<TX>> {
    inner: GenericScenario<TX, T>,
}

pub struct TestCase {
    program: CompiledProgram,
}

impl<'a> ScenarioInput<'a> for TestCase {
    fn decode(bytes: &'a [u8]) -> Result<Self, String> {
        let program = postcard::from_bytes(bytes).map_err(|e| {
            log::error!("{:?}", e);
            let len = bytes.len();
            log::error!("{:?} {}", &bytes[len - 8..], len);
            e.to_string()
        })?;
        Ok(Self { program })
    }
}

impl<TX: Transport, T: Target<TX>> Scenario<'_, TestCase, IgnoredCharacterization>
    for IrScenario<TX, T>
{
    fn new(args: &[String]) -> Result<Self, String> {
        let inner = GenericScenario::new(args)?;

        // Dump program context
        let context = ProgramContext {
            num_nodes: 1,
            num_connections: inner.connections.len(),
            timestamp: inner.time,
        };

        log::info!("IR context: {:?}", context);

        #[cfg(feature = "nyx")]
        {
            let bytes = postcard::to_allocvec(&context).map_err(|e| e.to_string())?;
            let ctx_file_name = "program.ctx";
            unsafe {
                nyx_dump_file_to_host(
                    ctx_file_name.as_ptr() as *const i8,
                    ctx_file_name.len(),
                    bytes.as_ptr(),
                    bytes.len(),
                );
            }
        }

        if let Ok(txo_file) = std::env::var("DUMP_TXOS_FILE") {
            let mut txos: Vec<fuzzamoto_ir::Txo> = Vec::new();

            for (block, _height) in inner
                .block_tree
                .values()
                .filter(|(_, height)| *height < 100)
            {
                let coinbase = block.coinbase().unwrap();
                let mut hash = [0u8; 32];
                hash.copy_from_slice(
                    coinbase
                        .compute_txid()
                        .as_raw_hash()
                        .as_byte_array()
                        .as_slice(),
                );

                txos.push(fuzzamoto_ir::Txo {
                    outpoint: (hash, 0u32),
                    value: 25 * 100_000_000,
                    script_pubkey: vec![
                        // 0x0 0x20 sha256(OP_TRUE)
                        0u8, 32, 74, 232, 21, 114, 240, 110, 27, 136, 253, 92, 237, 122, 26, 0, 9,
                        69, 67, 46, 131, 225, 85, 30, 111, 114, 30, 233, 192, 11, 140, 195, 50, 96,
                    ],
                    spending_script_sig: vec![],
                    spending_witness: vec![vec![0x51]],
                });
            }

            let bytes = postcard::to_allocvec(&txos).map_err(|e| e.to_string())?;
            std::fs::write(txo_file, bytes).map_err(|e| e.to_string())?;
        }

        Ok(Self { inner })
    }

    fn run(&mut self, testcase: TestCase) -> ScenarioResult<IgnoredCharacterization> {
        for action in testcase.program.actions {
            match action {
                CompiledAction::SendRawMessage(from, command, message) => {
                    if self.inner.connections.is_empty() {
                        continue;
                    }

                    let num_connections = self.inner.connections.len();
                    if let Some(connection) = self
                        .inner
                        .connections
                        .get_mut(from as usize % num_connections)
                    {
                        let _ = connection.send(&(command.to_string(), message));
                    }
                }
                CompiledAction::SetTime(time) => {
                    let _ = self.inner.target.set_mocktime(time);
                }
                _ => {}
            }
        }

        for connection in self.inner.connections.iter_mut() {
            let _ = connection.ping();
        }

        if let Err(e) = self.inner.target.is_alive() {
            return ScenarioResult::Fail(format!("Target is not alive: {}", e));
        }

        ScenarioResult::Ok(IgnoredCharacterization)
    }
}

#[cfg(feature = "record")]
fuzzamoto_main!(
    IrScenario::<
        fuzzamoto::connections::RecordingTransport,
        fuzzamoto::targets::RecorderTarget<fuzzamoto::connections::RecordingTransport>,
    >,
    TestCase
);

#[cfg(not(feature = "record"))]
fuzzamoto_main!(
    IrScenario::<fuzzamoto::connections::V1Transport, BitcoinCoreTarget>,
    TestCase
);

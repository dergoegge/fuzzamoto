use crate::input::IrInput;
use bitcoin::consensus::Decodable;
use fuzzamoto_ir::generators::compact_block::BlockTransactionsRequestRecved;
use libafl::{
    HasMetadata,
    corpus::{Corpus, CorpusId, Testcase},
    executors::ExitKind,
    feedbacks::{Feedback, StateInitializer},
    observers::StdOutObserver,
    state::HasCorpus,
};
use libafl_bolts::{Error, Named, impl_serdeany};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;

use libafl_bolts::tuples::{Handle, Handled, MatchName, MatchNameRef};
/// Module to parse the message sent from the other node
#[derive(Clone, Debug)]
pub struct RecvFeedback {
    o_ref: Handle<StdOutObserver>,
}

impl RecvFeedback {
    /// Creates a new [`RecvFeedback`].
    #[must_use]
    pub fn new(observer: &StdOutObserver) -> Self {
        Self {
            o_ref: observer.handle(),
        }
    }
}

impl Named for RecvFeedback {
    fn name(&self) -> &std::borrow::Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("RecvFeedback");
        &NAME
    }
}

/// Runtime metadata for fuzzamoto. This data is changed at runtime in response to the harness execution during fuzzing
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeMetadata {
    // TODO: If you want to add another metadata, then add it to `PerTestcaseMetadata` (not here!)
    metadatas: HashMap<CorpusId, fuzzamoto_ir::PerTestcaseMetadata>,
}

impl RuntimeMetadata {
    pub fn metadata(&self, id: CorpusId) -> Option<&fuzzamoto_ir::PerTestcaseMetadata> {
        self.metadatas.get(&id)
    }
}

impl_serdeany!(RuntimeMetadata);

/// Parse the incoming message from the other peer and process it
pub fn process_command<S>(
    state: &mut S,
    conn: usize,
    command: &str,
    payload: &[u8],
) -> Result<(), Error>
where
    S: HasMetadata + HasCorpus<IrInput>,
{
    match command {
        "getblocktxn" => {
            let req =
                bitcoin::bip152::BlockTransactionsRequest::consensus_decode_from_finite_reader(
                    &mut std::io::Cursor::new(payload),
                )
                .unwrap();
            let btr = BlockTransactionsRequestRecved::new(conn, req);
            let current = *state.corpus().current();
            if let Some(cur) = current
                && let Ok(meta) = state.metadata_mut::<RuntimeMetadata>()
            {
                let txvec = meta.metadatas.entry(cur).or_default();
                txvec.add_block_tx_request(btr);
            }
        }
        _ => {
            // if we want to add handling for other messages, add it here.
        }
    }
    Ok(())
}

impl<S> StateInitializer<S> for RecvFeedback {}

impl<EM, OT, S> Feedback<EM, IrInput, OT, S> for RecvFeedback
where
    S: HasCorpus<IrInput> + HasMetadata,
    OT: MatchName,
{
    #[inline]
    fn is_interesting(
        &mut self,
        state: &mut S,
        _manager: &mut EM,
        _input: &IrInput,
        observers: &OT,
        _exit_kind: &ExitKind,
    ) -> Result<bool, Error> {
        let observer = observers
            .get(&self.o_ref)
            .ok_or(Error::illegal_state("StdOutObserver is missing"))?;

        let buffer = observer
            .output
            .as_ref()
            .ok_or(Error::illegal_state("StdOutObserver has no stdout"))?;
        if !buffer.is_empty() {
            let chunks: Vec<Vec<u8>> = buffer
                .split(|b| *b == b'\n')
                .map(|slice| slice.to_vec())
                .collect();

            for chunk in chunks {
                if chunk.is_empty() {
                    continue;
                }
                if let Ok((conn, command, payload)) =
                    serde_json::from_slice::<(usize, String, Vec<u8>)>(&chunk)
                {
                    process_command(state, conn, &command, &payload)?;
                    log::info!("Command received. From {:?}, command: {:?}", conn, command,);
                } else {
                    // for some reason, I cannot receive more than 2686 bytes (limitation from nyx side)
                    // check https://github.com/nyx-fuzz/QEMU-Nyx/blob/qemu-nyx-4.2.0/nyx/auxiliary_buffer.c#L48 and https://github.com/nyx-fuzz/QEMU-Nyx/blob/qemu-nyx-4.2.0/nyx/state/state.c#L242
                    // 4096 - (128 + 512 * 2 + 256) - 2 = 2686.
                    // but the target does send us more bytes than that (of course), in that case just print into the log
                    // (i can't do anything if nyx doesn't print it)
                    log::info!("Failed to deserialize payload (size: {})", chunk.len());
                }
            }
        }
        Ok(false)
    }

    fn append_metadata(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        _observers: &OT,
        _testcase: &mut Testcase<IrInput>,
    ) -> Result<(), Error> {
        Ok(())
    }
}

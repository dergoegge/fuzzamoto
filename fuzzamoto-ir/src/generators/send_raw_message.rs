use crate::{
    Instruction, Operation, Variable,
    generators::{Generator, GeneratorError, GeneratorResult, ProgramBuilder},
};
use rand::{Rng, RngCore, seq::SliceRandom};

/// `SendMessageGenerator` generates programs that sends a message on a random connection
pub struct SendMessageGenerator {
    allowed_msg_types: Vec<String>,
}

impl SendMessageGenerator {
    pub fn new(allowed_msg_types: Vec<String>) -> Self {
        Self { allowed_msg_types }
    }

    pub fn default_with_erlay() -> Self {
        let mut default = Self::default();
        default.allowed_msg_types.extend(vec![
            "sendtxrcncl".to_string(),
            "reqtxrcncl".to_string(),
            "sketch".to_string(),
            "reconcildiff".to_string(),
            "reqsketchext".to_string(),
        ]);
        default
    }
}

impl Default for SendMessageGenerator {
    fn default() -> Self {
        Self::new(vec![
            "version".to_string(),
            "verack".to_string(),
            "addr".to_string(),
            "inv".to_string(),
            "getdata".to_string(),
            "notfound".to_string(),
            "getblocks".to_string(),
            "getheaders".to_string(),
            "mempool".to_string(),
            "tx".to_string(),
            "block".to_string(),
            "headers".to_string(),
            "sendheaders".to_string(),
            "getaddr".to_string(),
            "ping".to_string(),
            "pong".to_string(),
            "merkleblock".to_string(),
            "filterload".to_string(),
            "filteradd".to_string(),
            "filterclear".to_string(),
            "getcfilters".to_string(),
            "cfilter".to_string(),
            "getcfheaders".to_string(),
            "cfheaders".to_string(),
            "getcfcheckpt".to_string(),
            "cfcheckpt".to_string(),
            "sendcmpct".to_string(),
            "cmpctblock".to_string(),
            "getblocktxn".to_string(),
            "blocktxn".to_string(),
            "alert".to_string(),
            "reject".to_string(),
            "feefilter".to_string(),
            "wtxidrelay".to_string(),
            "addrv2".to_string(),
            "sendaddrv2".to_string(),
        ])
    }
}

impl<R: RngCore> Generator<R> for SendMessageGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _rt_data: &fuzzamoto::RuntimeMetadata,
    ) -> GeneratorResult {
        // Use a connection from the parent program or load from context
        let conn_var_index = match builder.get_random_variable(rng, Variable::Connection) {
            Some(v) => v,
            None => {
                if builder.context().num_connections == 0 {
                    return Err(GeneratorError::InvalidContext(builder.context().clone()));
                }

                builder
                    .append(Instruction {
                        inputs: vec![],
                        operation: Operation::LoadConnection(
                            rng.gen_range(0..builder.context().num_connections),
                        ),
                    })
                    .expect("Inserting LoadConnection should always succeed")
                    .pop()
                    .expect("LoadConnection should always produce a var")
            }
        };

        let type_as_bytes = |t: &str| -> [char; 12] {
            let mut bytes = ['\0'; 12];
            for (i, &b) in t.as_bytes().iter().enumerate() {
                bytes[i] = b as char;
            }
            bytes
        };
        let msg_type_bytes = type_as_bytes(self.allowed_msg_types.choose(rng).unwrap());
        let msg_type_var = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::LoadMsgType(msg_type_bytes),
            })
            .expect("Inserting LoadMsgType should always succeed")
            .pop()
            .expect("LoadMsgType should always produce a var");

        let mut random_bytes = Vec::new();
        random_bytes.resize(64, 0);
        rng.fill_bytes(&mut random_bytes);
        let bytes_var = builder
            .append(Instruction {
                inputs: vec![],
                operation: Operation::LoadBytes(random_bytes),
            })
            .expect("Inserting LoadBytes should always succeed")
            .pop()
            .expect("LoadBytes should always produce a var");

        builder
            .append(Instruction {
                inputs: vec![conn_var_index.index, msg_type_var.index, bytes_var.index],
                operation: Operation::SendRawMessage,
            })
            .expect("Inserting SendRawMessage should always succeed");

        Ok(())
    }

    fn name(&self) -> &'static str {
        "SendMessageGenerator"
    }
}

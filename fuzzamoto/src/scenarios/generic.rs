use crate::{
    connections::{Connection, ConnectionType, HandshakeOpts, Transport},
    dictionaries::{Dictionary, FileDictionary},
    scenarios::{Scenario, ScenarioInput, ScenarioResult},
    taproot::{TaprootKeypair, TaprootLeaf, TaprootSpendInfo, TaprootTxo},
    targets::Target,
    test_utils,
};

use bitcoin::{
    Amount, Block, BlockHash, OutPoint, Sequence, Transaction, TxIn, TxOut, Witness,
    blockdata::opcodes::{
        OP_TRUE,
        all::{OP_CHECKSIG, OP_PUSHNUM_1},
    },
    consensus::encode::{self, Decodable, Encodable, VarInt},
    hashes::{Hash, sha256},
    p2p::{
        message::{CommandString, NetworkMessage},
        message_blockdata::Inventory,
        message_compact_blocks::SendCmpct,
    },
    script::{PushBytesBuf, ScriptBuf},
    secp256k1::{Keypair, Secp256k1, SecretKey, XOnlyPublicKey},
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo as BitcoinTaprootSpendInfo},
    transaction,
};

use io::{self, Read, Write};
use std::collections::{BTreeMap, VecDeque};

pub enum Action {
    Connect {
        connection_type: ConnectionType,
    },
    Message {
        from: u16,
        command: CommandString,
        data: Vec<u8>,
    },
    SetMocktime {
        time: u64,
    },
    AdvanceTime {
        seconds: u16,
    },
}

pub struct TestCase {
    pub actions: Vec<Action>,
}

/// Limit how many deterministic Taproot UTXOs we mine per snapshot
const TAPROOT_TARGET_OUTPUTS: usize = 4;
/// Fixed fee we subtract when spending the OP_TRUE coinbase into a Taproot output
const TAPROOT_FEE_SATS: u64 = 5_000;
/// Only reuse coinbases below this height so they are guaranteed to be mature
const COINBASE_MATURITY_HEIGHT_LIMIT: u32 = 100;

/// Records the tapscript + version we plan to commit so the builder loop can
/// add each leaf and later serialize the exact same data into the Taproot
/// context without recomputing anything.
#[derive(Clone)]
struct TaprootLeafPlan {
    script: ScriptBuf,
    version: LeafVersion,
}

impl<'a> ScenarioInput<'a> for TestCase {
    fn decode(bytes: &'a [u8]) -> Result<Self, String> {
        TestCase::consensus_decode(&mut &bytes[..]).map_err(|e| e.to_string())
    }
}

/// `GenericScenario` is an implementation agnostic scenario testing the p2p interface of a target
/// node.
///
/// The scenario setup creates a couple of connections to the target node and mines a chain of 200
/// blocks. Testcases simulate the processing of a series of messages by the target node, i.e. each
/// testcase represents a series of three types of actions:
///
/// 1. Send a message to the target node through one of the existing connections
/// 2. Open a new p2p connection
/// 3. Advance the mocktime of the target node
///
/// At the end of each test case execution the scenario ensures all sent messages are processed
/// through a ping/pong roundtrip and checks that the target remains alive with `Target::is_alive`.
pub struct GenericScenario<TX: Transport, T: Target<TX>> {
    pub target: T,
    pub connections: Vec<Connection<TX>>,
    pub time: u64,
    pub block_tree: BTreeMap<BlockHash, (Block, u32)>,
    pub taproot_txos: Vec<TaprootTxo>,

    _phantom: std::marker::PhantomData<(TX, T)>,
}

impl<TX: Transport, T: Target<TX>> GenericScenario<TX, T> {
    fn from_target(mut target: T) -> Result<Self, String> {
        let genesis_block = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);

        let mut time = genesis_block.header.time as u64;
        target.set_mocktime(time)?;

        let mut connections = vec![
            (
                target.connect(ConnectionType::Outbound)?,
                true,
                true,
                true,
                false,
            ),
            (
                target.connect(ConnectionType::Outbound)?,
                true,
                true,
                false,
                true,
            ),
            (
                target.connect(ConnectionType::Outbound)?,
                true,
                false,
                true,
                true,
            ),
            (
                target.connect(ConnectionType::Outbound)?,
                false,
                false,
                true,
                false,
            ),
            (
                target.connect(ConnectionType::Inbound)?,
                true,
                true,
                true,
                true,
            ),
            (
                target.connect(ConnectionType::Inbound)?,
                true,
                true,
                false,
                true,
            ),
            (
                target.connect(ConnectionType::Inbound)?,
                true,
                false,
                true,
                true,
            ),
            (
                target.connect(ConnectionType::Inbound)?,
                false,
                false,
                true,
                false,
            ),
        ];

        let mut send_compact = false;
        for (connection, relay, wtxidrelay, addrv2, erlay) in connections.iter_mut() {
            connection.version_handshake(HandshakeOpts {
                time: time as i64,
                relay: *relay,
                starting_height: 0,
                wtxidrelay: *wtxidrelay,
                addrv2: *addrv2,
                erlay: *erlay,
            })?;
            let sendcmpct = NetworkMessage::SendCmpct(SendCmpct {
                version: 2,
                send_compact,
            });
            connection.send(&("sendcmpct".to_string(), encode::serialize(&sendcmpct)))?;
            send_compact = !send_compact;
        }

        let mut prev_hash = genesis_block.block_hash();
        const INTERVAL: u64 = 1;

        let mut dictionary = FileDictionary::new();

        let mut block_tree = BTreeMap::new();
        let mut taproot_txos = Vec::new();
        for height in 1..=200 {
            time += INTERVAL;

            let block = test_utils::mining::mine_block(prev_hash, height, time as u32)?;

            // Send block to the first connection
            connections[0]
                .0
                .send(&("block".to_string(), encode::serialize(&block)))?;

            target.set_mocktime(time as u64)?;

            // Update for next iteration
            prev_hash = block.block_hash();

            // Add block hash and coinbase txid to the dictionary
            dictionary.add(block.block_hash().as_raw_hash().as_byte_array().as_slice());
            dictionary.add(
                block.txdata[0]
                    .compute_txid()
                    .as_raw_hash()
                    .as_byte_array()
                    .as_slice(),
            );

            block_tree.insert(prev_hash, (block, height));
        }

        taproot_txos.extend(Self::append_taproot_blocks(
            &mut connections,
            &mut target,
            &mut block_tree,
            &mut prev_hash,
            &mut time,
            &mut dictionary,
        )?);

        let mut output = std::io::Cursor::new(Vec::new());
        dictionary.write(&mut output);

        let result = String::from_utf8(output.into_inner()).unwrap();
        println!("{}", result);

        for (connection, _, _, _, _) in connections.iter_mut() {
            connection.ping()?;
        }

        // Announce the tip on all connections
        for (connection, _, _, _, _) in connections.iter_mut() {
            let inv = NetworkMessage::Inv(vec![Inventory::Block(prev_hash)]);
            connection.send_and_recv(&("inv".to_string(), encode::serialize(&inv)), false)?;
        }

        Ok(Self {
            target,
            time,
            connections: connections.drain(..).map(|(c, _, _, _, _)| c).collect(),
            block_tree,
            taproot_txos,
            _phantom: std::marker::PhantomData,
        })
    }

    /// Mines additional blocks that spend mature coinbases into Taproot outputs,
    /// relays them to the target, and records the resulting Taproot UTXOs.
    fn append_taproot_blocks(
        connections: &mut Vec<(Connection<TX>, bool, bool, bool, bool)>,
        target: &mut T,
        block_tree: &mut BTreeMap<BlockHash, (Block, u32)>,
        prev_hash: &mut BlockHash,
        time: &mut u64,
        dictionary: &mut FileDictionary,
    ) -> Result<Vec<TaprootTxo>, String> {
        let secp = Secp256k1::new();
        let funding_candidates = Self::collect_mature_coinbases(block_tree);
        let funding: Vec<_> = funding_candidates
            .into_iter()
            .take(TAPROOT_TARGET_OUTPUTS)
            .collect();

        if funding.is_empty() {
            return Ok(Vec::new());
        }

        let taproot_txs = Self::build_taproot_transactions(&secp, funding)?;
        let mut queue: VecDeque<_> = taproot_txs.into();
        let mut taproot_txos = Vec::new();
        let mut height = block_tree.values().map(|(_, h)| *h).max().unwrap_or(0);

        while let Some((tx, txo)) = queue.pop_front() {
            *time += 1;
            height += 1;

            let mut block = test_utils::mining::mine_block(*prev_hash, height, *time as u32)?;
            block.txdata.push(tx);

            test_utils::mining::fixup_commitments(&mut block);
            test_utils::mining::fixup_proof_of_work(&mut block);

            if let Some((connection, ..)) = connections.get_mut(0) {
                connection.send(&("block".to_string(), encode::serialize(&block)))?;
            } else {
                return Err("no connection available to relay taproot blocks".into());
            }

            target.set_mocktime(*time)?;

            *prev_hash = block.block_hash();

            dictionary.add(block.block_hash().as_raw_hash().as_byte_array().as_slice());
            dictionary.add(
                block.txdata[0]
                    .compute_txid()
                    .as_raw_hash()
                    .as_byte_array()
                    .as_slice(),
            );

            if let Some(tx) = block.txdata.last() {
                let txid = tx.compute_txid();
                dictionary.add(txid.as_raw_hash().as_byte_array().as_slice());
                let wtxid = tx.compute_wtxid();
                dictionary.add(wtxid.as_raw_hash().as_byte_array().as_slice());
            }

            taproot_txos.push(txo);
            block_tree.insert(*prev_hash, (block, height));
        }

        Ok(taproot_txos)
    }

    /// Returns coinbase outputs below the maturity threshold so they can be
    /// safely spent into Taproot transactions.
    fn collect_mature_coinbases(
        block_tree: &BTreeMap<BlockHash, (Block, u32)>,
    ) -> Vec<(OutPoint, Amount)> {
        let mut entries: Vec<_> = block_tree
            .values()
            .filter(|(_, height)| *height < COINBASE_MATURITY_HEIGHT_LIMIT)
            .map(|(block, height)| {
                let coinbase = block.coinbase().expect("block must include a coinbase");
                (
                    *height,
                    OutPoint {
                        txid: coinbase.compute_txid(),
                        vout: 0,
                    },
                    coinbase.output[0].value,
                )
            })
            .collect();
        entries.sort_by_key(|(height, _, _)| *height);
        entries
            .into_iter()
            .map(|(_, outpoint, amount)| (outpoint, amount))
            .collect()
    }

    /// Creates the list of Taproot transactions (key-path and script-path)
    /// backed by the selected coinbase outputs.
    fn build_taproot_transactions(
        secp: &Secp256k1<bitcoin::secp256k1::All>,
        funding: Vec<(OutPoint, Amount)>,
    ) -> Result<Vec<(Transaction, TaprootTxo)>, String> {
        let mut outputs = Vec::new();
        for (index, (outpoint, amount)) in funding.into_iter().enumerate() {
            // Alternate between pure key-path (even index) and script-path (odd index) taproots.
            let include_script_path = index % 2 == 1;
            let (tx, taproot_txo) = Self::create_taproot_transaction(
                secp,
                outpoint,
                amount,
                index as u32,
                include_script_path,
            )?;
            outputs.push((tx, taproot_txo));
        }
        Ok(outputs)
    }

    /// Builds a single Taproot transaction and the corresponding `TaprootTxo`
    /// metadata entry that will be stored in the context dump.
    fn create_taproot_transaction(
        secp: &Secp256k1<bitcoin::secp256k1::All>,
        funding_outpoint: OutPoint,
        funding_amount: Amount,
        seed: u32,
        include_script_path: bool,
    ) -> Result<(Transaction, TaprootTxo), String> {
        let secret_key = Self::deterministic_secret_key(&funding_outpoint, seed);
        let keypair = Keypair::from_secret_key(secp, &secret_key);
        let (internal_key, _) = XOnlyPublicKey::from_keypair(&keypair);

        let mut leaf_plans = Vec::new();
        if include_script_path {
            leaf_plans.push(TaprootLeafPlan {
                script: Self::build_default_tapscript(&internal_key)?,
                version: LeafVersion::TapScript,
            });
        }

        let mut builder = TaprootBuilder::new();
        for leaf in &leaf_plans {
            builder = builder
                .add_leaf_with_ver(0, leaf.script.clone(), leaf.version)
                .map_err(|e| format!("{e:?}"))?;
        }

        let spend_info = if leaf_plans.is_empty() {
            BitcoinTaprootSpendInfo::new_key_spend(secp, internal_key, None)
        } else {
            builder
                .finalize(secp, internal_key)
                .map_err(|e| format!("{e:?}"))?
        };

        let output_key_bytes = spend_info.output_key().to_x_only_public_key().serialize();
        let push_bytes = PushBytesBuf::try_from(output_key_bytes.to_vec())
            .map_err(|_| "failed to encode taproot key bytes".to_string())?;
        let script_pubkey = ScriptBuf::builder()
            .push_opcode(OP_PUSHNUM_1)
            .push_slice(&push_bytes)
            .into_script();

        let input_sats = funding_amount.to_sat();
        if input_sats <= TAPROOT_FEE_SATS {
            return Err("insufficient funds for taproot transaction".into());
        }
        let output_sats = input_sats - TAPROOT_FEE_SATS;

        let mut witness = Witness::new();
        witness.push([OP_TRUE.to_u8()]);

        let tx = Transaction {
            version: transaction::Version(2),
            lock_time: bitcoin::absolute::LockTime::from_height(0).unwrap(),
            input: vec![TxIn {
                previous_output: funding_outpoint,
                script_sig: ScriptBuf::new(),
                sequence: Sequence(0xFFFFFFFF),
                witness,
            }],
            output: vec![TxOut {
                value: Amount::from_sat(output_sats),
                script_pubkey: script_pubkey.clone(),
            }],
        };

        let leaves = leaf_plans
            .iter()
            .map(|plan| {
                let control_block = spend_info
                    .control_block(&(plan.script.clone(), plan.version))
                    .ok_or_else(|| "missing control block for tapscript leaf".to_string())?;
                let merkle_branch = control_block
                    .merkle_branch
                    .iter()
                    .map(|hash| *hash.as_byte_array())
                    .collect();
                Ok(TaprootLeaf {
                    version: plan.version.to_consensus(),
                    script: plan.script.as_bytes().to_vec(),
                    merkle_branch,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let taproot_txo = Self::encode_taproot_txo(
            &tx,
            output_sats,
            secret_key,
            internal_key,
            &script_pubkey,
            &spend_info,
            leaves,
        )?;

        Ok((tx, taproot_txo))
    }

    /// Packages the transaction plus spend info into the
    /// serializable `TaprootTxo` format stored in the context.
    fn encode_taproot_txo(
        tx: &Transaction,
        value: u64,
        secret_key: SecretKey,
        internal_key: XOnlyPublicKey,
        script_pubkey: &ScriptBuf,
        spend_info: &BitcoinTaprootSpendInfo,
        leaves: Vec<TaprootLeaf>,
    ) -> Result<TaprootTxo, String> {
        let txid = tx.compute_txid();
        let mut outpoint = [0u8; 32];
        outpoint.copy_from_slice(txid.as_raw_hash().as_byte_array());

        let merkle_root = spend_info.merkle_root().map(|root| *root.as_byte_array());

        Ok(TaprootTxo {
            outpoint: (outpoint, 0),
            value,
            spend_info: TaprootSpendInfo {
                keypair: TaprootKeypair {
                    secret_key: secret_key.secret_bytes(),
                    public_key: internal_key.serialize(),
                },
                merkle_root,
                output_key: spend_info.output_key().to_x_only_public_key().serialize(),
                output_key_parity: match spend_info.output_key_parity() {
                    bitcoin::secp256k1::Parity::Even => 0,
                    bitcoin::secp256k1::Parity::Odd => 1,
                },
                script_pubkey: script_pubkey.as_bytes().to_vec(),
                leaves,
            },
        })
    }

    /// Deterministically derives a secp256k1 secret key from the funding
    /// outpoint and a counter so Taproot keypairs stay reproducible.
    fn deterministic_secret_key(outpoint: &OutPoint, counter: u32) -> SecretKey {
        let mut entropy = Vec::with_capacity(4 + 4 + 32);
        entropy.extend_from_slice(outpoint.txid.as_raw_hash().as_byte_array());
        entropy.extend_from_slice(&outpoint.vout.to_le_bytes());
        entropy.extend_from_slice(&counter.to_le_bytes());
        Self::secret_key_from_entropy(&entropy)
    }

    /// Hashes the provided entropy until a valid secp256k1 secret key emerges.
    fn secret_key_from_entropy(seed: &[u8]) -> SecretKey {
        let mut hash = sha256::Hash::hash(seed);
        loop {
            if let Ok(secret) = SecretKey::from_slice(hash.as_byte_array()) {
                return secret;
            }
            hash = sha256::Hash::hash(hash.as_byte_array());
        }
    }

    /// Returns the default `P CHECKSIG` tapscript used for script-path Taproot leaf plans.
    fn build_default_tapscript(key: &XOnlyPublicKey) -> Result<ScriptBuf, String> {
        let push = PushBytesBuf::try_from(key.serialize().to_vec())
            .map_err(|_| "failed to encode taproot key for tapscript".to_string())?;
        Ok(ScriptBuf::builder()
            .push_slice(&push)
            .push_opcode(OP_CHECKSIG)
            .into_script())
    }
}

impl<'a, TX: Transport, T: Target<TX>> Scenario<'a, TestCase> for GenericScenario<TX, T> {
    fn new(args: &[String]) -> Result<Self, String> {
        let target = T::from_path(&args[1])?;
        Self::from_target(target)
    }

    fn run(&mut self, testcase: TestCase) -> ScenarioResult {
        for action in testcase.actions {
            match action {
                Action::Connect { connection_type: _ } => {
                    //if let Ok(connection) = self.target.connect(connection_type) {
                    //    self.connections.push(connection);
                    //}
                }
                Action::Message {
                    from,
                    command,
                    data,
                } => {
                    if self.connections.is_empty() {
                        continue;
                    }

                    let num_connections = self.connections.len();
                    if let Some(connection) =
                        self.connections.get_mut(from as usize % num_connections)
                    {
                        let _ = connection.send(&(command.to_string(), data));
                    }
                }
                Action::SetMocktime { time } => {
                    let _ = self.target.set_mocktime(time);
                }
                Action::AdvanceTime { seconds } => {
                    self.time += seconds as u64;
                    let _ = self.target.set_mocktime(self.time);
                }
            }
        }

        for connection in self.connections.iter_mut() {
            let _ = connection.ping();
        }

        if let Err(e) = self.target.is_alive() {
            return ScenarioResult::Fail(format!("Target is not alive: {}", e));
        }

        ScenarioResult::Ok
    }
}

impl Encodable for Action {
    fn consensus_encode<W: Write + ?Sized>(&self, s: &mut W) -> Result<usize, io::Error> {
        match self {
            Action::Connect { connection_type } => {
                let mut len = 0;
                len += 0u8.consensus_encode(s)?; // Tag for Connect
                match connection_type {
                    ConnectionType::Inbound => {
                        false.consensus_encode(s)?;
                    }
                    ConnectionType::Outbound => {
                        true.consensus_encode(s)?;
                    }
                };
                len += 1;
                Ok(len)
            }
            Action::Message {
                from,
                command,
                data,
            } => {
                let mut len = 0;
                len += 1u8.consensus_encode(s)?; // Tag for Message
                len += from.consensus_encode(s)?;
                len += command.consensus_encode(s)?;
                len += data.consensus_encode(s)?;
                Ok(len)
            }
            Action::SetMocktime { time } => {
                let mut len = 0;
                len += 2u8.consensus_encode(s)?; // Tag for SetMocktime
                len += time.consensus_encode(s)?;
                Ok(len)
            }
            Action::AdvanceTime { seconds } => {
                let mut len = 0;
                len += 3u8.consensus_encode(s)?; // Tag for AdvanceTime
                len += seconds.consensus_encode(s)?;
                Ok(len)
            }
        }
    }
}

impl Decodable for Action {
    fn consensus_decode<D: Read + ?Sized>(d: &mut D) -> Result<Self, encode::Error> {
        let tag = u8::consensus_decode(d)? % 4;
        match tag {
            0 => {
                let connection_type_b = bool::consensus_decode(d)?;
                let connection_type = if connection_type_b {
                    ConnectionType::Outbound
                } else {
                    ConnectionType::Inbound
                };
                Ok(Action::Connect { connection_type })
            }
            1 => {
                let from = u16::consensus_decode(d)?;
                let command = CommandString::consensus_decode(d)?;
                let data = Vec::<u8>::consensus_decode(d)?;
                Ok(Action::Message {
                    from,
                    command,
                    data,
                })
            }
            2 => {
                let time = u64::consensus_decode(d)?;
                Ok(Action::SetMocktime { time })
            }
            3 => {
                let seconds = u16::consensus_decode(d)?;
                Ok(Action::AdvanceTime { seconds })
            }
            _ => Err(encode::Error::ParseFailed("Invalid Action tag")),
        }
    }
}

impl Encodable for TestCase {
    fn consensus_encode<W: Write + ?Sized>(&self, s: &mut W) -> Result<usize, io::Error> {
        let mut len = 0;
        len += VarInt(self.actions.len() as u64).consensus_encode(s)?;
        for action in &self.actions {
            len += action.consensus_encode(s)?;
        }
        Ok(len)
    }
}

impl Decodable for TestCase {
    fn consensus_decode<D: Read + ?Sized>(d: &mut D) -> Result<Self, encode::Error> {
        let len = VarInt::consensus_decode(d)?.0;
        if len > 1000 {
            return Err(encode::Error::ParseFailed("too many actions"));
        }
        let mut actions = Vec::with_capacity(len as usize);
        for _ in 0..len {
            actions.push(Action::consensus_decode(d)?);
        }
        Ok(TestCase { actions })
    }
}

use crate::{
    connections::{Connection, ConnectionType, HandshakeOpts, Transport},
    scenarios::generic::{Action, TestCase},
    scenarios::{Scenario, ScenarioResult},
    targets::Target,
    test_utils,
};

use bitcoin::{consensus::encode, p2p::message::NetworkMessage, p2p::message_blockdata::Inventory};

/// `LibbitcoinGenericScenario` tests the P2P interface of libbitcoin-server.
///
/// Unlike `GenericScenario`, this only uses inbound connections since libbitcoin
/// does not support dynamic peer management for outbound connections.
pub struct LibbitcoinGenericScenario<TX: Transport, T: Target<TX>> {
    pub target: T,
    pub connections: Vec<Connection<TX>>,
    pub time: u64,
    _phantom: std::marker::PhantomData<(TX, T)>,
}

impl<TX: Transport, T: Target<TX>> LibbitcoinGenericScenario<TX, T> {
    fn from_target(mut target: T) -> Result<Self, String> {
        let genesis_block = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Regtest);
        let time = genesis_block.header.time as u64;

        // libbitcoin has no mocktime support, this is a no-op
        let _ = target.set_mocktime(time);

        // Create inbound connections
        // libbitcoin does not support wtxidrelay (BIP339), addrv2 (BIP155), or erlay (BIP330)
        let mut connections = Vec::new();
        for _ in 0..4 {
            let mut conn = target.connect(ConnectionType::Inbound)?;
            conn.version_handshake(HandshakeOpts {
                time: time as i64,
                relay: true,
                starting_height: 0,
                wtxidrelay: false,
                addrv2: false,
                erlay: false,
            })?;
            connections.push(conn);
        }

        // Mine initial chain of 200 blocks
        let mut prev_hash = genesis_block.block_hash();
        let mut current_time = time;

        for height in 1..=200 {
            current_time += 1;
            let block = test_utils::mining::mine_block(prev_hash, height, current_time as u32)?;
            connections[0].send(&("block".to_string(), encode::serialize(&block)))?;
            prev_hash = block.block_hash();
        }

        // Sync all connections
        for conn in connections.iter_mut() {
            conn.ping()?;
        }

        // Announce tip on all connections
        for conn in connections.iter_mut() {
            let inv = NetworkMessage::Inv(vec![Inventory::Block(prev_hash)]);
            conn.send_and_recv(&("inv".to_string(), encode::serialize(&inv)), false)?;
        }

        Ok(Self {
            target,
            time: current_time,
            connections,
            _phantom: std::marker::PhantomData,
        })
    }
}

impl<'a, TX: Transport, T: Target<TX>> Scenario<'a, TestCase> for LibbitcoinGenericScenario<TX, T> {
    fn new(args: &[String]) -> Result<Self, String> {
        let target = T::from_path(&args[1])?;
        Self::from_target(target)
    }

    fn run(&mut self, testcase: TestCase) -> ScenarioResult {
        for action in testcase.actions {
            match action {
                Action::Connect { .. } => {
                    // Dynamic connections not supported for libbitcoin
                }
                Action::Message {
                    from,
                    command,
                    data,
                } => {
                    if self.connections.is_empty() {
                        continue;
                    }
                    let idx = from as usize % self.connections.len();
                    let _ = self.connections[idx].send(&(command.to_string(), data));
                }
                Action::SetMocktime { time } => {
                    // No-op for libbitcoin (no mocktime support)
                    let _ = self.target.set_mocktime(time);
                }
                Action::AdvanceTime { seconds } => {
                    self.time += seconds as u64;
                    let _ = self.target.set_mocktime(self.time);
                }
            }
        }

        // Sync all connections
        for conn in self.connections.iter_mut() {
            let _ = conn.ping();
        }

        // Check target is still alive
        if let Err(e) = self.target.is_alive() {
            return ScenarioResult::Fail(format!("Target is not alive: {}", e));
        }

        ScenarioResult::Ok
    }
}

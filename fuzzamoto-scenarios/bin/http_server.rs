use fuzzamoto::{
    fuzzamoto_main,
    runners::Runner,
    scenarios::{Scenario, ScenarioInput, ScenarioResult},
    targets::{BitcoinCoreTarget, TargetNode},
};

use arbitrary::{Arbitrary, Unstructured};
use std::io::Write;

use std::net::TcpStream;

#[derive(Arbitrary)]
enum Action<'a> {
    Connect,
    SendMessage {
        connection_id: u8,
        message: &'a [u8],
    },
    Disconnect {
        connection_id: u8,
    },
}

#[derive(Arbitrary)]
struct TestCase<'a> {
    actions: Vec<Action<'a>>,
}

impl<'a> ScenarioInput<'a> for TestCase<'a> {
    fn decode(bytes: &'a [u8]) -> Result<Self, String> {
        let mut unstructured = Unstructured::new(bytes);
        let actions = Vec::arbitrary(&mut unstructured).map_err(|e| e.to_string())?;
        Ok(Self { actions })
    }
}

/// `HttpServerScenario` is a scenario that tests the HTTP server of Bitcoin Core.
///
/// Testcases simulate the processing of a series of actions by the HTTP server of Bitcoin Core.
/// Each testcase represents a series of three types of actions:
///
/// 1. Connect to the HTTP server
/// 2. Send a message to the HTTP server from a specific connection
/// 3. Disconnect one of the existing connections
struct HttpServerScenario {
    target: BitcoinCoreTarget,
}

impl<'a> Scenario<'a, TestCase<'a>> for HttpServerScenario {
    fn new(args: &[String]) -> Result<Self, String> {
        Ok(Self {
            target: BitcoinCoreTarget::from_path(&args[1])?,
        })
    }

    fn run(&mut self, input: TestCase, _runner: &dyn Runner) -> ScenarioResult {
        // Network actions are slow; limit them
        const MAX_ACTIONS: usize = 128;
        if input.actions.len() > MAX_ACTIONS {
            return ScenarioResult::Ok;
        }

        let mut connections = Vec::with_capacity(MAX_ACTIONS);
        for action in input.actions {
            match action {
                Action::Connect => {
                    let Ok(stream) = TcpStream::connect(self.target.node.params.rpc_socket) else {
                        return ScenarioResult::Fail("Failed to connect to the target".to_string());
                    };
                    let _ = stream.set_nodelay(true);
                    connections.push(stream);
                }
                Action::SendMessage {
                    connection_id,
                    message,
                } => {
                    if connections.is_empty() {
                        continue;
                    }
                    let index = connection_id as usize % connections.len();
                    let connection = connections.get_mut(index).unwrap();
                    let _ = connection.write_all(message);
                    let _ = connection.flush();
                }
                Action::Disconnect { connection_id } => {
                    if connections.is_empty() {
                        continue;
                    }
                    let index = connection_id as usize % connections.len();
                    let _ = connections.swap_remove(index);
                }
            }
        }

        if let Err(e) = self.target.is_alive() {
            return ScenarioResult::Fail(format!("Target is not alive: {e}"));
        }

        ScenarioResult::Ok
    }
}

fuzzamoto_main!(HttpServerScenario, TestCase);

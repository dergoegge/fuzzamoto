use crate::{
    connections::{Connection, ConnectionType, V1Transport},
    targets::{
        HasGetBestBlockHash, HasGetBlock, HasGetBlockCount, HasGetBlockHash, HasGetTxOutSetInfo,
        Target,
    },
};

use bitcoin::{Amount, Block, BlockHash};
use corepc_node::{Conf, Node, P2P};
use std::{
    net::{SocketAddrV4, TcpListener, TcpStream},
    str::FromStr,
};

use super::ConnectableTarget;

pub struct BitcoinCoreTarget {
    pub node: Node,
    listeners: Vec<TcpListener>,
    time: u64,
}

// Gently stop the node when the target is dropped, if we are not using nyx.
#[cfg(not(feature = "nyx"))]
impl Drop for BitcoinCoreTarget {
    fn drop(&mut self) {
        let _ = self.node.stop();
    }
}

impl BitcoinCoreTarget {
    fn create_listener() -> Result<(TcpListener, u16), String> {
        // Bind to port 0 to let the OS assign a random available port
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to create TCP listener: {}", e))?;

        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to get listener address: {}", e))?
            .port();

        Ok((listener, port))
    }
}

impl Target<V1Transport> for BitcoinCoreTarget {
    fn from_path(exe_path: &str) -> Result<Self, String> {
        let mut config = Conf::default();
        config.tmpdir = None;
        config.staticdir = None;
        config.p2p = P2P::Yes;

        #[cfg(feature = "inherit_stdout")]
        {
            config.args.extend_from_slice(&[
                "-debug",
                "-debugexclude=libevent",
                "-debugexclude=leveldb",
            ]);
            config.view_stdout = true;
        }
        config.args.extend_from_slice(&[
            "-txreconciliation",
            "-peerbloomfilters",
            "-peerblockfilters",
            "-blockfilterindex",
            "-par=4",
            "-rpcthreads=4",
            "-deprecatedrpc=create_bdb",
            "-keypool=10",
            "-listenonion=0",
            "-i2pacceptincoming=0",
            "-maxmempool=5", // 5MB
            "-dbcache=4",    // 4MiB
            "-datacarriersize=1000000",
            "-peertimeout=8223372036854775807",
            "-noconnect",
        ]);

        let node = Node::with_conf(exe_path, &config)
            .map_err(|e| format!("Failed to start node: {:?}", e))?;

        Ok(Self {
            node,
            listeners: Vec::new(),
            time: u64::MAX,
        })
    }

    fn connect(
        &mut self,
        connection_type: ConnectionType,
    ) -> Result<Connection<V1Transport>, String> {
        match connection_type {
            ConnectionType::Inbound => {
                // For inbound, connect directly to the P2P port
                let p2p_socket = self
                    .node
                    .params
                    .p2p_socket
                    .ok_or_else(|| "P2P socket address not available".to_string())?;
                let socket = TcpStream::connect(p2p_socket)
                    .map_err(|e| format!("Failed to connect to P2P port: {}", e))?;
                // Disable Nagle's algorithm, since most of the time we're sending small messages and
                // we want to reduce latency when fuzzing.
                socket
                    .set_nodelay(true)
                    .expect("Failed to set nodelay on inbound socket");

                Ok(Connection::new(connection_type, V1Transport { socket }))
            }
            ConnectionType::Outbound => {
                let (listener, port) = Self::create_listener()?;
                self.listeners.push(listener);
                let listener = self.listeners.last().unwrap();

                // Tell Bitcoin Core to connect to our listener
                let client = &self.node.client;
                client
                    .call::<serde_json::Value>(
                        "addconnection",
                        &[
                            format!("127.0.0.1:{}", port).into(),
                            "outbound-full-relay".into(),
                            false.into(), // no v2
                        ],
                    )
                    .map_err(|e| format!("Failed to initiate outbound connection: {:?}", e))?;

                // Wait for Bitcoin Core to connect
                let (socket, _addr) = listener
                    .accept()
                    .map_err(|e| format!("Failed to accept connection: {}", e))?;
                socket
                    .set_nodelay(true)
                    .expect("Failed to set nodelay on outbound socket");

                Ok(Connection::new(connection_type, V1Transport { socket }))
            }
        }
    }

    fn set_mocktime(&mut self, time: u64) -> Result<(), String> {
        let client = &self.node.client;

        if self.time != u64::MAX && time > self.time {
            // Mock the scheduler forward if we're advancing in time
            let delta = (time - self.time).min(3600);
            let _ = client.call::<()>("mockscheduler", &[delta.into()]);
        }
        self.time = time;
        client
            .call::<()>("setmocktime", &[time.into()])
            .map_err(|e| format!("Failed to set mocktime: {:?}", e))
    }

    fn is_alive(&self) -> Result<(), String> {
        // Call the echo rpc to check if the node is still alive
        let client = &self.node.client;
        client
            .call::<serde_json::Value>(
                "echo",
                &[r#"Ground Control to Major Tom
Your circuit's dead, there's something wrong
Can you hear me, Major Tom?
Can you hear me, Major Tom?
Can you hear me, Major Tom?
Can you-"#
                    .into()],
            )
            .map_err(|e| format!("Failed to check if node is alive: {:?}", e))?;

        client
            .call::<()>("syncwithvalidationinterfacequeue", &[])
            .map_err(|e| format!("Failed to sync with validation interface queue: {:?}", e))?;

        Ok(())
    }

    fn connect_to<O: ConnectableTarget>(&mut self, other: &O) -> Result<(), String> {
        if let Some(addr) = other.get_addr() {
            self.node
                .client
                .call::<serde_json::Value>(
                    "addconnection",
                    &[
                        format!("{:?}", addr).into(),
                        "outbound-full-relay".into(),
                        false.into(), // no v2
                    ],
                )
                .map_err(|e| format!("Failed to initiate outbound connection: {:?}", e))?;
        } else {
            return Err("Other node does not have a valid address".to_string());
        }

        Ok(())
    }
}

impl ConnectableTarget for BitcoinCoreTarget {
    fn get_addr(&self) -> Option<SocketAddrV4> {
        self.node.params.p2p_socket
    }

    fn is_connected_to<O: ConnectableTarget>(&self, other: &O) -> bool {
        let Some(other_addr) = other.get_addr() else {
            return false;
        };

        let Ok(peer_info) = self
            .node
            .client
            .call::<serde_json::Value>("getpeerinfo", &[])
        else {
            return false;
        };

        // Iterate through all connected peers and attempt to find `other_addr`
        for peer in peer_info.as_array().unwrap() {
            let addr = peer.get("addr").unwrap().as_str().unwrap();
            if SocketAddrV4::from_str(addr).unwrap() == other_addr {
                return true;
            }
        }

        false
    }
}

impl HasGetBestBlockHash for BitcoinCoreTarget {
    fn getbestblockhash(&self) -> Option<BlockHash> {
        match self.node.client.get_best_block_hash() {
            Ok(result) => result.block_hash().ok(),
            Err(_) => None,
        }
    }
}

impl HasGetBlockCount for BitcoinCoreTarget {
    fn getblockcount(&self) -> Option<u64> {
        match self.node.client.get_block_count() {
            Ok(result) => Some(result.0),
            Err(_) => None,
        }
    }
}

impl HasGetBlock for BitcoinCoreTarget {
    fn getblock(&self, hash: BlockHash) -> Option<Block> {
        match self.node.client.get_block(hash) {
            Ok(result) => Some(result),
            Err(_) => None,
        }
    }
}

impl HasGetBlockHash for BitcoinCoreTarget {
    fn getblockhash(&self, height: u64) -> Option<BlockHash> {
        match self.node.client.get_block_hash(height) {
            Ok(result) => result.block_hash().ok(),
            Err(_) => None,
        }
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct TxOutSetInfo {
    height: u64,
    amount: bitcoin::Amount,
}

impl TxOutSetInfo {
    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn amount(&self) -> bitcoin::Amount {
        self.amount
    }
}

impl HasGetTxOutSetInfo for BitcoinCoreTarget {
    fn gettxoutsetinfo(&self) -> Result<TxOutSetInfo, String> {
        let txoutsetinfo = self
            .node
            .client
            .call::<serde_json::Value>("gettxoutsetinfo", &[])
            .map_err(|e| format!("Failed to request txoutsetinfo: {:?}", e))?;

        let info = match txoutsetinfo {
            serde_json::Value::Object(obj) => obj,
            _ => return Err("Failed to request txoutsetinfo".to_string()),
        };

        let amount = match info.get("total_amount") {
            Some(serde_json::Value::Number(num)) => num,
            _ => return Err("Failed to request txoutsetinfo".to_string()),
        };
        let amount = match amount.as_f64() {
            Some(v) => v,
            None => {
                return Err("Failed to request txoutsetinfo".to_string());
            }
        };
        let amount = match Amount::from_btc(amount) {
            Ok(amount) => amount,
            _ => return Err("txoutsetinfo returns invalid amount".to_string()),
        };

        let height = match info.get("height") {
            Some(serde_json::Value::Number(num)) => num,
            _ => return Err("Failed to request txoutsetinfo".to_string()),
        };
        let height = match height.as_u64() {
            Some(v) => v,
            None => {
                return Err("Failed to request txoutsetinfo".to_string());
            }
        };

        Ok(TxOutSetInfo { height, amount })
    }
}

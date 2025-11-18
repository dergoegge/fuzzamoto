use bitcoin::consensus::encode::{Encodable, ReadExt};
use bitcoin::p2p::{ServiceFlags, address::Address, message_network::VersionMessage};
use std::io::{Read, Write};

use std::net;

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionType {
    Inbound,
    Outbound,
}

pub trait Transport {
    /// Send a message to the target node
    fn send(&mut self, message: &(String, Vec<u8>)) -> Result<(), String>;

    /// Receive a message from the target node
    fn receive(&mut self) -> Result<(String, Vec<u8>), String>;

    /// Get the local address of the transport
    fn local_addr(&self) -> Result<net::SocketAddr, String>;
}

pub struct V1Transport {
    pub socket: net::TcpStream,
}

impl Transport for V1Transport {
    fn send(&mut self, message: &(String, Vec<u8>)) -> Result<(), String> {
        log::debug!(
            "send {:?} message (len={} from={:?})",
            message.0,
            message.1.len(),
            self.socket.local_addr().unwrap(),
        );

        let mut header = Vec::with_capacity(24);

        header.extend_from_slice(&bitcoin::network::Network::Regtest.magic().to_bytes());

        // Command (12 bytes, null-padded)
        let mut command_bytes = [0u8; 12];
        command_bytes[..message.0.len()].copy_from_slice(message.0.as_bytes());
        header.extend_from_slice(&command_bytes);

        let mut hasher = bitcoin_hashes::sha256d::HashEngine::default();
        hasher.write_all(&message.1).unwrap();
        let checksum = bitcoin_hashes::Sha256d::from_engine(hasher);

        header.extend_from_slice(&(message.1.len() as u32).to_le_bytes());
        header.extend_from_slice(&checksum.as_byte_array()[0..4]);

        self.socket
            .write_all(&header)
            .map_err(|e| format!("Failed to send message header: {}", e))?;
        self.socket
            .write_all(&message.1)
            .map_err(|e| format!("Failed to send message payload: {}", e))?;

        Ok(())
    }

    fn receive(&mut self) -> Result<(String, Vec<u8>), String> {
        // Read the message header (24 bytes)
        let mut header_bytes = [0u8; 24];
        self.socket
            .read_exact(&mut header_bytes)
            .map_err(|e| format!("Failed to read message header: {}", e))?;

        let mut cursor = std::io::Cursor::new(&header_bytes);

        // Parse magic bytes (skip validation for now)
        let _magic = cursor
            .read_u32()
            .map_err(|e| format!("Failed to read magic: {}", e))?;

        // Read command (12 bytes, null-padded)
        let mut command = [0u8; 12];
        cursor
            .read_exact(&mut command)
            .map_err(|e| format!("Failed to read command: {}", e))?;

        // Convert command to string, trimming null bytes
        let command = String::from_utf8_lossy(&command)
            .trim_matches(char::from(0))
            .to_string();

        // Read payload length
        let payload_len = cursor
            .read_u32()
            .map_err(|e| format!("Failed to read payload length: {}", e))?;

        // Skip checksum (we're not validating it)
        let _checksum = cursor
            .read_u32()
            .map_err(|e| format!("Failed to read checksum: {}", e))?;

        // Read the payload
        let mut payload = vec![0u8; payload_len as usize];
        self.socket
            .read_exact(&mut payload)
            .map_err(|e| format!("Failed to read payload: {}", e))?;

        log::debug!(
            "received {:?} message (len={} on={:?})",
            command,
            payload_len,
            self.socket.local_addr().unwrap(),
        );

        Ok((command, payload))
    }

    fn local_addr(&self) -> Result<net::SocketAddr, String> {
        self.socket
            .local_addr()
            .map_err(|e| format!("Failed to get local address: {}", e))
    }
}

pub struct Connection<T: Transport> {
    connection_type: ConnectionType,
    transport: T,
    ping_counter: u64,
}

impl<T: Transport> Connection<T> {
    /// Create a new connection to the target node from a socket.
    ///
    /// # Arguments
    ///
    /// * `connection_type` - The type of connection to create (either inbound or outbound)
    /// * `transport` - The transport to use for the connection
    pub fn new(connection_type: ConnectionType, transport: T) -> Self {
        log::debug!(
            "new connection (type={:?} addr={:?})",
            connection_type,
            transport.local_addr().unwrap(),
        );
        Self {
            connection_type,
            transport,
            ping_counter: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HandshakeOpts {
    pub time: i64,
    pub relay: bool,
    pub starting_height: i32,
    pub wtxidrelay: bool,
    pub addrv2: bool,
    pub erlay: bool,
}

impl<T: Transport> Connection<T> {
    fn send_ping(&mut self, nonce: u64) -> Result<(), String> {
        let ping_message = ("ping".to_string(), nonce.to_le_bytes().to_vec());
        self.transport.send(&ping_message)?;
        Ok(())
    }

    fn wait_for_pong(&mut self, nonce: u64) -> Result<(), String> {
        loop {
            let received = self.transport.receive()?;
            if received.0 == "pong" && received.1.len() == 8 && received.1 == nonce.to_le_bytes() {
                break;
            }
        }

        Ok(())
    }

    pub fn send(&mut self, message: &(String, Vec<u8>)) -> Result<(), String> {
        self.transport.send(message)
    }

    pub fn receive(&mut self) -> Result<(String, Vec<u8>), String> {
        self.transport.receive()
    }

    pub fn local_addr(&self) -> Result<net::SocketAddr, String> {
        self.transport.local_addr()
    }

    pub fn ping(&mut self) -> Result<(), String> {
        self.ping_counter += 1;
        self.send_ping(self.ping_counter)?;
        self.wait_for_pong(self.ping_counter)?;
        Ok(())
    }

    pub fn send_and_ping(&mut self, message: &(String, Vec<u8>)) -> Result<(), String> {
        self.transport.send(message)?;
        // Sending two pings back-to-back, requires that the node calls `ProcessMessage` twice, and
        // thus ensures `SendMessages` must have been called at least once
        self.send_ping(0x0)?;
        self.ping_counter += 1;
        self.send_ping(self.ping_counter)?;
        self.wait_for_pong(self.ping_counter)?;
        Ok(())
    }

    pub fn version_handshake(&mut self, opts: HandshakeOpts) -> Result<(), String> {
        let socket_addr = self.transport.local_addr().unwrap();

        let mut version_message = VersionMessage::new(
            ServiceFlags::NETWORK | ServiceFlags::WITNESS,
            opts.time,
            Address::new(&socket_addr, ServiceFlags::NONE),
            Address::new(&socket_addr, ServiceFlags::NONE),
            0xdeadbeef,
            String::from("fuzzamoto"),
            opts.starting_height,
        );

        version_message.version = 70016; // wtxidrelay version
        version_message.relay = opts.relay;

        if self.connection_type == ConnectionType::Outbound {
            loop {
                let received = self.transport.receive()?;
                if received.0 == "version" {
                    break;
                }
            }
        }

        // Convert version message to (String, Vec<u8>) format
        let mut version_bytes = Vec::new();
        version_message
            .consensus_encode(&mut version_bytes)
            .map_err(|e| format!("Failed to encode version message: {}", e))?;
        self.transport
            .send(&("version".to_string(), version_bytes))?;

        // Send optional features if configured
        if opts.wtxidrelay {
            self.transport.send(&("wtxidrelay".to_string(), vec![]))?;
        }
        if opts.addrv2 {
            self.transport.send(&("sendaddrv2".to_string(), vec![]))?;
        }
        if opts.erlay {
            let version = 1u32;
            let salt = 0u64;
            let mut bytes = Vec::new();
            version.consensus_encode(&mut bytes).unwrap();
            salt.consensus_encode(&mut bytes).unwrap();
            self.transport.send(&("sendtxrcncl".to_string(), bytes))?;
        }

        // Send verack
        self.transport.send(&("verack".to_string(), vec![]))?;

        // Wait for verack
        loop {
            let received = self.transport.receive()?;
            if received.0 == "verack" {
                break;
            }
        }

        Ok(())
    }
}

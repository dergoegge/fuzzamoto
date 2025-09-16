use bitcoin::consensus::encode::Encodable;
use bitcoin::p2p::{ServiceFlags, address::Address, message_network::VersionMessage};
use std::io::{Read, Write};
#[cfg(feature = "desocket")]
use std::collections::VecDeque;

use std::net;

#[cfg(feature = "desocket")]
mod desocket;
#[cfg(feature = "desocket")]
pub use desocket::DesocketTransport;

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionType {
    Inbound,
    Outbound,
}

pub trait Transport {
    /// Send one complete P2P/RPC message (name, payload). The transport is responsible for framing/encoding.
    fn send(&mut self, msg: &(String, Vec<u8>)) -> std::io::Result<()>;

    /// Receive the next complete message if available.
    fn receive(&mut self) -> std::io::Result<Option<(String, Vec<u8>)>>;

    /// Get the local address of the transport
    fn local_addr(&self) -> std::io::Result<net::SocketAddr>;
}

/// Helper function to encode a P2P message for the wire
fn encode_p2p_message(cmd: &str, payload: &[u8], magic: [u8; 4]) -> Vec<u8> {
    use bitcoin_hashes::sha256d;
    let mut out = Vec::with_capacity(24 + payload.len());
    out.extend_from_slice(&magic);
    let mut name = [0u8; 12];
    let b = cmd.as_bytes();
    name[..b.len().min(12)].copy_from_slice(&b[..b.len().min(12)]);
    out.extend_from_slice(&name);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    let check = sha256d::Hash::hash(payload);
    out.extend_from_slice(&check.to_byte_array()[..4]);
    out.extend_from_slice(payload);
    out
}

pub struct V1Transport {
    pub socket: net::TcpStream,
    magic: [u8; 4],
}

impl V1Transport {
    pub fn new(socket: net::TcpStream) -> Self {
        Self {
            socket,
            magic: bitcoin::network::Network::Regtest.magic().to_bytes(),
        }
    }
}

impl Transport for V1Transport {
    fn send(&mut self, msg: &(String, Vec<u8>)) -> std::io::Result<()> {
        log::debug!(
            "send {:?} message (len={} from={:?})",
            msg.0,
            msg.1.len(),
            self.socket.local_addr().unwrap(),
        );

        let bytes = encode_p2p_message(&msg.0, &msg.1, self.magic);
        self.socket.write_all(&bytes)?;
        Ok(())
    }

    fn receive(&mut self) -> std::io::Result<Option<(String, Vec<u8>)>> {
        // Read the message header (24 bytes)
        let mut header_bytes = [0u8; 24];
        self.socket.read_exact(&mut header_bytes)?;

        let mut cursor = std::io::Cursor::new(&header_bytes);

        // Parse magic bytes (skip validation for now)
        let mut magic_buf = [0u8; 4];
        cursor.read_exact(&mut magic_buf)?;
        let _magic = u32::from_le_bytes(magic_buf);

        // Read command (12 bytes, null-padded)
        let mut command = [0u8; 12];
        cursor.read_exact(&mut command)?;

        // Convert command to string, trimming null bytes
        let command = String::from_utf8_lossy(&command)
            .trim_matches(char::from(0))
            .to_string();

        // Read payload length
        let mut len_buf = [0u8; 4];
        cursor.read_exact(&mut len_buf)?;
        let payload_len = u32::from_le_bytes(len_buf);

        // Skip checksum (we're not validating it)
        let mut checksum_buf = [0u8; 4];
        cursor.read_exact(&mut checksum_buf)?;

        // Read the payload
        let mut payload = vec![0u8; payload_len as usize];
        self.socket.read_exact(&mut payload)?;

        log::debug!(
            "received {:?} message (len={} on={:?})",
            command,
            payload_len,
            self.socket.local_addr().unwrap(),
        );

        Ok(Some((command, payload)))
    }

    fn local_addr(&self) -> std::io::Result<net::SocketAddr> {
        self.socket.local_addr()
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
    fn send_ping(&mut self, nonce: u64) -> std::io::Result<()> {
        let ping_message = ("ping".to_string(), nonce.to_le_bytes().to_vec());
        self.transport.send(&ping_message)?;
        Ok(())
    }

    fn wait_for_pong(&mut self, nonce: u64) -> std::io::Result<()> {
        loop {
            if let Some((cmd, payload)) = self.transport.receive()? {
                if cmd == "pong" && payload.len() == 8 && payload == nonce.to_le_bytes() {
                    break;
                }
            }
        }

        Ok(())
    }

    pub fn send(&mut self, message: &(String, Vec<u8>)) -> std::io::Result<()> {
        self.transport.send(message)
    }

    pub fn receive(&mut self) -> std::io::Result<(String, Vec<u8>)> {
        self.transport
            .receive()?
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::WouldBlock, "no message"))
    }

    pub fn ping(&mut self) -> std::io::Result<()> {
        self.ping_counter += 1;
        self.send_ping(self.ping_counter)?;
        self.wait_for_pong(self.ping_counter)?;
        Ok(())
    }

    pub fn send_and_ping(&mut self, message: &(String, Vec<u8>)) -> std::io::Result<()> {
        self.transport.send(message)?;
        // Sending two pings back-to-back, requires that the node calls `ProcessMessage` twice, and
        // thus ensures `SendMessages` must have been called at least once
        self.send_ping(0x0)?;
        self.ping_counter += 1;
        self.send_ping(self.ping_counter)?;
        self.wait_for_pong(self.ping_counter)?;
        Ok(())
    }

    pub fn version_handshake(&mut self, opts: HandshakeOpts) -> std::io::Result<()> {
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
                if let Some((cmd, _payload)) = self.transport.receive()? {
                    if cmd == "version" {
                        break;
                    }
                }
            }
        }

        // Convert version message to (String, Vec<u8>) format
        let mut version_bytes = Vec::new();
        version_message
            .consensus_encode(&mut version_bytes)?;
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
            if let Some((cmd, _payload)) = self.transport.receive()? {
                if cmd == "verack" {
                    break;
                }
            }
        }

        Ok(())
    }
}

// Mock transport for desocketing - eliminates real TCP socket overhead
#[cfg(feature = "desocket")]
pub struct MockTransport {
    // In-memory buffers to simulate network communication
    read_buffer: VecDeque<(String, Vec<u8>)>,
    local_address: net::SocketAddr,
}

#[cfg(feature = "desocket")]
impl MockTransport {
    pub fn new() -> Self {
        Self {
            read_buffer: VecDeque::new(),
            // Use a fake address for local_addr() compatibility
            local_address: "127.0.0.1:0".parse().unwrap(),
        }
    }

    /// Feed a message into the mock transport (simulates receiving from network)
    pub fn feed_message(&mut self, command: String, payload: Vec<u8>) {
        self.read_buffer.push_back((command, payload));
    }
}

#[cfg(feature = "desocket")]
impl Transport for MockTransport {
    fn send(&mut self, message: &(String, Vec<u8>)) -> std::io::Result<()> {
        log::debug!(
            "mock send {:?} message (len={})",
            message.0,
            message.1.len(),
        );
        
        // In a real implementation, this would be sent to the target
        // For now, we just log it - this is the baby step version
        Ok(())
    }

    fn receive(&mut self) -> std::io::Result<Option<(String, Vec<u8>)>> {
        if let Some(message) = self.read_buffer.pop_front() {
            log::debug!(
                "mock received {:?} message (len={})",
                message.0,
                message.1.len(),
            );
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }

    fn local_addr(&self) -> std::io::Result<net::SocketAddr> {
        Ok(self.local_address)
    }
}

#[cfg(test)]
mod tests;

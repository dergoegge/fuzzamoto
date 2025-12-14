use crate::{
    connections::{Connection, ConnectionType, V1Transport},
    targets::{ConnectableTarget, Target},
};

use std::{
    net::{SocketAddrV4, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

/// LibbitcoinTarget wraps a libbitcoin-server (bs) process for fuzzing.
///
/// Due to libbitcoin limitations:
/// - Only inbound connections are supported (no dynamic peer management)
/// - Mocktime is stubbed (libbitcoin has no mocktime support)
/// - Uses native libbitcoin consensus (not libconsensus wrapper)
pub struct LibbitcoinTarget {
    process: Child,
    p2p_addr: SocketAddrV4,
    datadir: PathBuf,
}

// Gracefully stop the node when dropped, if not using nyx.
#[cfg(not(feature = "nyx"))]
impl Drop for LibbitcoinTarget {
    fn drop(&mut self) {
        // Send SIGTERM to allow graceful shutdown
        let _ = self.process.kill();
        let _ = self.process.wait();
        // Clean up temp directory
        let _ = std::fs::remove_dir_all(&self.datadir);
    }
}

impl LibbitcoinTarget {
    /// Generate minimal regtest configuration for libbitcoin-server.
    ///
    /// - identifier: Regtest network magic bytes (0xDAB5BFFA as u32 = 3669344250)
    /// - inbound_port: P2P listen port
    /// - database.directory: Chain data storage location
    /// - use_libconsensus = false: Use libbitcoin's native consensus (not Bitcoin Core's)
    fn generate_config(datadir: &std::path::Path, p2p_port: u16) -> String {
        format!(
            r#"[network]
identifier = 3669344250
inbound_port = {p2p_port}

[database]
directory = {datadir}/database

[blockchain]
use_libconsensus = false
"#,
            datadir = datadir.display(),
            p2p_port = p2p_port,
        )
    }

    /// Wait for the P2P port to become ready by attempting connections.
    fn wait_for_p2p_ready(addr: SocketAddrV4, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();
        let retry_interval = Duration::from_millis(100);

        while start.elapsed() < timeout {
            match TcpStream::connect_timeout(&addr.into(), Duration::from_millis(500)) {
                Ok(_) => return Ok(()),
                Err(_) => std::thread::sleep(retry_interval),
            }
        }

        Err(format!(
            "P2P port {} not ready after {:?}",
            addr.port(),
            timeout
        ))
    }

    /// Find an available port for P2P.
    fn find_available_port() -> Result<u16, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to find available port: {}", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to get port: {}", e))?
            .port();
        drop(listener);
        Ok(port)
    }
}

impl Target<V1Transport> for LibbitcoinTarget {
    fn from_path(exe_path: &str) -> Result<Self, String> {
        // Create temporary data directory
        let datadir = std::env::temp_dir().join(format!("libbitcoin-fuzz-{}", std::process::id()));
        std::fs::create_dir_all(&datadir)
            .map_err(|e| format!("Failed to create datadir: {}", e))?;

        // Find available port for P2P
        let p2p_port = Self::find_available_port()?;
        let p2p_addr: SocketAddrV4 = format!("127.0.0.1:{}", p2p_port)
            .parse()
            .map_err(|e| format!("Failed to parse P2P address: {}", e))?;

        // Write configuration file
        let config_path = datadir.join("bs.cfg");
        let config = Self::generate_config(&datadir, p2p_port);
        std::fs::write(&config_path, &config)
            .map_err(|e| format!("Failed to write config: {}", e))?;

        // Create database directory
        std::fs::create_dir_all(datadir.join("database"))
            .map_err(|e| format!("Failed to create database directory: {}", e))?;

        // Spawn libbitcoin-server with --initchain to initialize if needed
        let process = Command::new(exe_path)
            .arg("--config")
            .arg(&config_path)
            .arg("--initchain")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn libbitcoin-server: {}", e))?;

        // Wait for P2P port to be ready
        Self::wait_for_p2p_ready(p2p_addr, Duration::from_secs(30))?;

        Ok(Self {
            process,
            p2p_addr,
            datadir,
        })
    }

    fn connect(
        &mut self,
        connection_type: ConnectionType,
    ) -> Result<Connection<V1Transport>, String> {
        match connection_type {
            ConnectionType::Inbound => {
                // Connect directly to libbitcoin's P2P port
                let socket = TcpStream::connect(self.p2p_addr)
                    .map_err(|e| format!("Failed to connect to P2P port: {}", e))?;

                // Disable Nagle's algorithm for low latency
                socket
                    .set_nodelay(true)
                    .map_err(|e| format!("Failed to set TCP_NODELAY: {}", e))?;

                Ok(Connection::new(connection_type, V1Transport { socket }))
            }
            ConnectionType::Outbound => {
                // libbitcoin does not support dynamic peer management
                // There's no ZeroMQ command or P2P mechanism to make it connect to us
                Err("Outbound connections not supported for libbitcoin (no dynamic peer management)".to_string())
            }
        }
    }

    fn connect_to<O: ConnectableTarget>(&mut self, _other: &O) -> Result<(), String> {
        // libbitcoin cannot initiate outbound connections dynamically
        Err("connect_to not supported for libbitcoin (no dynamic peer management)".to_string())
    }

    fn set_mocktime(&mut self, time: u64) -> Result<(), String> {
        // libbitcoin has no mocktime support - it uses std::chrono::system_clock::now() directly
        log::warn!(
            "set_mocktime({}) called but libbitcoin has no mocktime support",
            time
        );
        Ok(())
    }

    fn is_alive(&self) -> Result<(), String> {
        // Simple liveness check: can we connect to the P2P port?
        TcpStream::connect_timeout(&self.p2p_addr.into(), Duration::from_secs(5))
            .map_err(|e| format!("Node not responding on P2P port: {}", e))?;
        Ok(())
    }
}

impl ConnectableTarget for LibbitcoinTarget {
    fn get_addr(&self) -> Option<SocketAddrV4> {
        Some(self.p2p_addr)
    }

    fn is_connected_to<O: ConnectableTarget>(&self, _other: &O) -> bool {
        // libbitcoin has no RPC to query peer list
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires libbitcoin-server binary
    fn test_spawn_and_connect() {
        let path = std::env::var("LIBBITCOIN_PATH")
            .unwrap_or_else(|_| "/opt/libbitcoin/bin/bs".to_string());

        let mut target =
            LibbitcoinTarget::from_path(&path).expect("Failed to spawn libbitcoin-server");

        let _conn = target
            .connect(ConnectionType::Inbound)
            .expect("Failed to create inbound connection");

        target.is_alive().expect("Target should be alive");
    }

    #[test]
    #[ignore] // Requires libbitcoin-server binary
    fn test_outbound_not_supported() {
        let path = std::env::var("LIBBITCOIN_PATH")
            .unwrap_or_else(|_| "/opt/libbitcoin/bin/bs".to_string());

        let mut target =
            LibbitcoinTarget::from_path(&path).expect("Failed to spawn libbitcoin-server");

        let result = target.connect(ConnectionType::Outbound);
        assert!(result.is_err());
    }
}

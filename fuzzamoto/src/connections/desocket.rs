use std::collections::VecDeque;
use std::process::{Command, Stdio, Child};
use std::io::{Write, BufReader, BufRead};
use std::net::SocketAddr;
use log::{debug, warn};

use crate::connections::Transport;

/// DesocketTransport implements Transport using libdesock.so LD_PRELOAD
/// to redirect socket operations to stdin/stdout instead of real sockets.
#[cfg(feature = "desocket")]
pub struct DesocketTransport {
    /// The spawned process with libdesock preloaded
    process: Option<Child>,
    /// Buffer for received messages from the process stdout
    read_buffer: VecDeque<(String, Vec<u8>)>,
    /// Local address (simulated)
    local_address: SocketAddr,
    /// Path to libdesock.so library
    libdesock_path: String,
}

#[cfg(feature = "desocket")]
impl DesocketTransport {
    /// Create a new DesocketTransport that will spawn a process with libdesock preloaded
    pub fn new(libdesock_path: String, local_addr: SocketAddr) -> Self {
        Self {
            process: None,
            read_buffer: VecDeque::new(),
            local_address: local_addr,
            libdesock_path,
        }
    }

    /// Spawn a process with libdesock preloaded
    pub fn spawn_process(&mut self, command: &str, args: &[&str]) -> std::io::Result<()> {
        debug!("Spawning process with libdesock: {} {:?}", command, args);
        
        let mut cmd = Command::new(command);
        cmd.args(args)
           .env("LD_PRELOAD", &self.libdesock_path)
           .stdin(Stdio::piped())
           .stdout(Stdio::piped())
           .stderr(Stdio::piped());

        let child = cmd.spawn()?;
        self.process = Some(child);
        
        debug!("Process spawned successfully with PID: {:?}", 
               self.process.as_ref().map(|p| p.id()));
        Ok(())
    }

    /// Read any available data from the process stdout
    fn read_from_process(&mut self) -> std::io::Result<()> {
        if let Some(ref mut process) = self.process {
            if let Some(ref mut stdout) = process.stdout {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                
                // Try to read a line without blocking
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        // EOF - process may have terminated
                        debug!("Process stdout EOF");
                    }
                    Ok(_) => {
                        // Successfully read a line
                        let data = line.trim().as_bytes().to_vec();
                        let data_len = data.len();
                        self.read_buffer.push_back(("desocket".to_string(), data));
                        debug!("Read {} bytes from process", data_len);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // No data available right now
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(())
    }
}

#[cfg(feature = "desocket")]
impl Transport for DesocketTransport {
    fn send(&mut self, message: &(String, Vec<u8>)) -> std::io::Result<()> {
        debug!("DesocketTransport sending {} message with {} bytes", message.0, message.1.len());
        
        if let Some(ref mut process) = self.process {
            if let Some(ref mut stdin) = process.stdin {
                stdin.write_all(&message.1)?;
                stdin.flush()?;
                debug!("Successfully sent {} bytes to process stdin", message.1.len());
            } else {
                warn!("Process stdin not available");
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "Process stdin not available"
                ));
            }
        } else {
            warn!("No process available for sending data");
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No process spawned"
            ));
        }
        
        Ok(())
    }

    fn receive(&mut self) -> std::io::Result<Option<(String, Vec<u8>)>> {
        // First try to read any new data from the process
        self.read_from_process()?;
        
        // Then return buffered data if available
        Ok(self.read_buffer.pop_front())
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.local_address)
    }
}

#[cfg(feature = "desocket")]
impl Drop for DesocketTransport {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            debug!("Terminating process on drop");
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desocket_transport_creation() {
        let addr: SocketAddr = "127.0.0.1:8333".parse().unwrap();
        let transport = DesocketTransport::new(
            "/home/a/Fuzzomoto/fuzzamoto/libdesock.so".to_string(),
            addr
        );
        
        assert_eq!(transport.local_addr().unwrap(), addr);
    }

    #[test]
    fn test_desocket_send_without_process() {
        let addr: SocketAddr = "127.0.0.1:8333".parse().unwrap();
        let mut transport = DesocketTransport::new(
            "/home/a/Fuzzomoto/fuzzamoto/libdesock.so".to_string(),
            addr
        );
        
        // Should fail when no process is spawned
        let result = transport.send(&("test".to_string(), b"test data".to_vec()));
        assert!(result.is_err());
    }

    #[test]
    fn test_desocket_receive_empty() {
        let addr: SocketAddr = "127.0.0.1:8333".parse().unwrap();
        let mut transport = DesocketTransport::new(
            "/home/a/Fuzzomoto/fuzzamoto/libdesock.so".to_string(),
            addr
        );
        
        // Should return None when buffer is empty
        let result = transport.receive().unwrap();
        assert!(result.is_none());
    }
}

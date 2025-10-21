use std::net::SocketAddr;
use crate::connections::{Transport, DesocketTransport};

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
        let result = transport.send(b"test data");
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

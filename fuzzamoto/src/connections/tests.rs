#[cfg(test)]
mod tests {
    #[cfg(feature = "desocket")]
    use crate::connections::MockTransport;
    #[cfg(feature = "desocket")]
    use crate::connections::Transport;

    #[test]
    #[cfg(feature = "desocket")]
    fn test_mock_transport_basic() {
        let mut transport = MockTransport::new();
        
        // Test that local_addr works
        assert!(transport.local_addr().is_ok());
        
        // Test feeding and receiving messages
        transport.feed_message("ping".to_string(), vec![1, 2, 3, 4]);
        
        let received = transport.receive().unwrap().unwrap();
        assert_eq!(received.0, "ping");
        assert_eq!(received.1, vec![1, 2, 3, 4]);
        
        // Test that subsequent receive returns None when buffer is empty
        assert!(transport.receive().unwrap().is_none());
    }

    #[test]
    #[cfg(feature = "desocket")]
    fn test_mock_transport_send() {
        let mut transport = MockTransport::new();
        
        // Test that send works (even though it's a no-op for now)
        let message = ("test".to_string(), vec![5, 6, 7, 8]);
        assert!(transport.send(&message).is_ok());
    }
}

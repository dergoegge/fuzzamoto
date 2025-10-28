use std::process::{Command, Stdio};
use std::io::{Read, Write};

/// Test libdesock integration without modifying the existing Transport trait
#[cfg(test)]
mod libdesock_integration_tests {
    use super::*;

    #[test]
    fn test_libdesock_library_exists() {
        let libdesock_path = "/home/a/Fuzzomoto/fuzzamoto/libdesock.so";
        assert!(std::path::Path::new(libdesock_path).exists(), 
                "libdesock.so should be present in the project root");
    }

    #[test]
    fn test_spawn_process_with_libdesock() {
        let libdesock_path = "/home/a/Fuzzomoto/fuzzamoto/libdesock.so";
        
        // Test spawning a simple process with LD_PRELOAD
        let mut cmd = Command::new("echo")
            .arg("hello")
            .env("LD_PRELOAD", libdesock_path)
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to spawn process with libdesock");

        let status = cmd.wait().expect("Failed to wait for process");
        assert!(status.success(), "Process should complete successfully with libdesock");
    }

    #[test]
    fn test_libdesock_with_stdin_stdout() {
        let libdesock_path = "/home/a/Fuzzomoto/fuzzamoto/libdesock.so";
        
        // Spawn cat process with libdesock - it should echo stdin to stdout
        let mut child = Command::new("cat")
            .env("LD_PRELOAD", libdesock_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to spawn cat with libdesock");

        // Write test data to stdin
        let test_data = b"Hello libdesock!";
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(test_data).expect("Failed to write to stdin");
            // Close stdin to signal EOF
            drop(child.stdin.take());
        }

        // Read from stdout
        let mut output = Vec::new();
        if let Some(stdout) = child.stdout.as_mut() {
            stdout.read_to_end(&mut output).expect("Failed to read from stdout");
        }

        let status = child.wait().expect("Failed to wait for process");
        assert!(status.success(), "Cat process should succeed");
        assert_eq!(output, test_data, "Output should match input");
    }

    #[test] 
    fn test_mock_bitcoin_process() {
        let libdesock_path = "/home/a/Fuzzomoto/fuzzamoto/libdesock.so";
        
        // Create a simple script that simulates socket operations
        let script = r#"
import socket
import sys
try:
    # This would normally create a real socket, but libdesock intercepts it
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(('127.0.0.1', 8333))
    print("Socket operations completed", flush=True)
    sock.close()
except Exception as e:
    print(f"Error: {e}", flush=True)
    sys.exit(1)
"#;

        let child = Command::new("python3")
            .arg("-c")
            .arg(script)
            .env("LD_PRELOAD", libdesock_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn python with libdesock");

        let output = child.wait_with_output().expect("Failed to get output");
        
        // The process should complete without errors
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        
        println!("Python stdout: {}", stdout_str);
        println!("Python stderr: {}", stderr_str);
        
        // Process should complete successfully with libdesock handling socket operations
        assert!(output.status.success() || stdout_str.contains("Socket operations completed"), 
                "Python socket operations should work with libdesock");
    }
}

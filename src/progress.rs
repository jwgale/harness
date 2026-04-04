//! Real-time progress protocol over Unix domain sockets.
//!
//! The Telegram bridge (or any external watcher) creates a `ProgressListener`
//! which binds a Unix socket at `.harness/progress.sock`. It then spawns the
//! runner with `HARNESS_PROGRESS_SOCK` set to that path. The runner connects
//! via `ProgressSender` and pushes lines in real time.
//!
//! Message format (one per line, newline-terminated):
//!   EVENT:<content>       — lifecycle event (step start/complete, verdict, etc.)
//!   STDOUT:<agent>:<line> — raw agent stdout line
//!   DONE:<summary>        — workflow finished
//!
//! If the socket doesn't exist or connection fails, the runner falls back to
//! file-based `progress.log` only.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

/// Environment variable name for the progress socket path.
pub const PROGRESS_SOCK_ENV: &str = "HARNESS_PROGRESS_SOCK";

// ---------------------------------------------------------------------------
// Sender (used by the runner)
// ---------------------------------------------------------------------------

/// Sends progress messages over a Unix socket. Thread-safe via interior Mutex.
pub struct ProgressSender {
    stream: std::sync::Mutex<UnixStream>,
}

impl ProgressSender {
    /// Connect to an existing progress socket. Returns None if not available.
    pub fn connect_from_env() -> Option<Self> {
        let path = std::env::var(PROGRESS_SOCK_ENV).ok()?;
        Self::connect(Path::new(&path))
    }

    /// Connect to a specific socket path.
    pub fn connect(path: &Path) -> Option<Self> {
        let stream = UnixStream::connect(path).ok()?;
        // Set a short write timeout so we never block the runner
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(100)));
        Some(Self {
            stream: std::sync::Mutex::new(stream),
        })
    }

    /// Send a lifecycle event.
    pub fn event(&self, content: &str) {
        self.write_line(&format!("EVENT:{content}"));
    }

    /// Send a raw agent stdout line.
    pub fn stdout(&self, agent: &str, line: &str) {
        self.write_line(&format!("STDOUT:{agent}:{line}"));
    }

    /// Send workflow completion.
    pub fn done(&self, summary: &str) {
        self.write_line(&format!("DONE:{summary}"));
    }

    fn write_line(&self, msg: &str) {
        if let Ok(mut stream) = self.stream.lock() {
            // Best-effort write — never block or crash the runner
            let _ = stream.write_all(msg.as_bytes());
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
        }
    }
}

// ---------------------------------------------------------------------------
// Listener (used by the bridge / external watcher)
// ---------------------------------------------------------------------------

/// A parsed progress message from the runner.
#[derive(Debug, Clone)]
pub enum ProgressMsg {
    /// Lifecycle event (step start, completion, verdict, etc.)
    Event(String),
    /// Raw agent stdout line (agent_name, line)
    Stdout(String, String),
    /// Workflow finished with summary
    Done(String),
}

impl ProgressMsg {
    /// Parse a raw line into a ProgressMsg.
    pub fn parse(line: &str) -> Option<Self> {
        if let Some(rest) = line.strip_prefix("EVENT:") {
            Some(Self::Event(rest.to_string()))
        } else if let Some(rest) = line.strip_prefix("STDOUT:") {
            let (agent, content) = rest.split_once(':').unwrap_or(("?", rest));
            Some(Self::Stdout(agent.to_string(), content.to_string()))
        } else {
            line.strip_prefix("DONE:").map(|rest| Self::Done(rest.to_string()))
        }
    }

    /// Format for display in Telegram (compact).
    pub fn display_line(&self) -> String {
        match self {
            Self::Event(s) => s.clone(),
            Self::Stdout(agent, line) => format!("[{agent}] {line}"),
            Self::Done(s) => format!("Done: {s}"),
        }
    }
}

/// Creates a Unix socket listener, accepts one connection, and forwards
/// parsed messages to an mpsc channel. Runs in a background thread.
///
/// Returns (socket_path, message_receiver).
pub fn create_listener(
    harness_dir: &Path,
) -> Result<(PathBuf, mpsc::Receiver<ProgressMsg>), String> {
    let sock_path = harness_dir.join("progress.sock");

    // Remove stale socket if it exists
    let _ = std::fs::remove_file(&sock_path);

    let listener = UnixListener::bind(&sock_path)
        .map_err(|e| format!("Failed to create progress socket: {e}"))?;

    // Set a timeout so accept() doesn't block forever if the runner never connects
    let _ = listener.set_nonblocking(false);

    let (tx, rx) = mpsc::channel();
    let path_clone = sock_path.clone();

    thread::spawn(move || {
        // Accept one connection (from the runner)
        // Use a timeout so we give up if no connection arrives
        let _ = listener.set_nonblocking(true);

        let mut stream = None;
        // Try to accept for up to 30 seconds
        for _ in 0..300 {
            match listener.accept() {
                Ok((s, _)) => {
                    stream = Some(s);
                    break;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }

        let Some(stream) = stream else {
            // No connection — runner probably doesn't support progress socket
            let _ = std::fs::remove_file(&path_clone);
            return;
        };

        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Some(msg) = ProgressMsg::parse(&line)
                && tx.send(msg).is_err()
            {
                break; // receiver dropped
            }
        }

        // Clean up
        let _ = std::fs::remove_file(&path_clone);
    });

    Ok((sock_path, rx))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_event() {
        let msg = ProgressMsg::parse("EVENT:Step 1/3 started").unwrap();
        assert!(matches!(msg, ProgressMsg::Event(ref s) if s == "Step 1/3 started"));
    }

    #[test]
    fn test_parse_stdout() {
        let msg = ProgressMsg::parse("STDOUT:my-builder:Building module X").unwrap();
        if let ProgressMsg::Stdout(agent, line) = msg {
            assert_eq!(agent, "my-builder");
            assert_eq!(line, "Building module X");
        } else {
            panic!("Expected Stdout");
        }
    }

    #[test]
    fn test_parse_stdout_with_colons() {
        // Content can contain colons
        let msg = ProgressMsg::parse("STDOUT:agent:key: value: extra").unwrap();
        if let ProgressMsg::Stdout(agent, line) = msg {
            assert_eq!(agent, "agent");
            assert_eq!(line, "key: value: extra");
        } else {
            panic!("Expected Stdout");
        }
    }

    #[test]
    fn test_parse_done() {
        let msg = ProgressMsg::parse("DONE:completed").unwrap();
        assert!(matches!(msg, ProgressMsg::Done(ref s) if s == "completed"));
    }

    #[test]
    fn test_parse_unknown() {
        assert!(ProgressMsg::parse("GARBAGE:stuff").is_none());
        assert!(ProgressMsg::parse("").is_none());
    }

    #[test]
    fn test_display_line() {
        assert_eq!(
            ProgressMsg::Event("step done".into()).display_line(),
            "step done"
        );
        assert_eq!(
            ProgressMsg::Stdout("builder".into(), "compiling".into()).display_line(),
            "[builder] compiling"
        );
    }

    #[test]
    fn test_socket_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "harness-progress-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();

        let (sock_path, rx) = create_listener(&tmp).unwrap();

        // Give listener thread time to start
        thread::sleep(std::time::Duration::from_millis(50));

        // Connect and send
        let sender = ProgressSender::connect(&sock_path).expect("should connect");
        sender.event("step started");
        sender.stdout("builder", "compiling main.rs");
        sender.done("completed");
        drop(sender); // close connection

        // Read messages
        thread::sleep(std::time::Duration::from_millis(100));
        let mut msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }

        assert!(msgs.len() >= 3, "expected 3 msgs, got {}", msgs.len());
        assert!(matches!(&msgs[0], ProgressMsg::Event(s) if s == "step started"));
        assert!(matches!(&msgs[1], ProgressMsg::Stdout(a, l) if a == "builder" && l == "compiling main.rs"));
        assert!(matches!(&msgs[2], ProgressMsg::Done(s) if s == "completed"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

//! Real-time progress protocol over Unix domain sockets.
//!
//! The Telegram bridge (or any external watcher) creates a listener via
//! `create_listener()` which binds a Unix socket at `.harness/progress.sock`.
//! It spawns the runner with `HARNESS_PROGRESS_SOCK` set to that path. The
//! runner connects via `ProgressSender` and pushes lines in real time.
//!
//! Message format (one per line, newline-terminated):
//!   EVENT:<content>       — lifecycle event (step start/complete, verdict, etc.)
//!   STDOUT:<agent>:<line> — raw agent stdout line
//!   DONE:<summary>        — workflow finished
//!
//! The listener accepts multiple client connections, forwards parsed messages
//! to an mpsc channel, and writes all messages to `progress.log` as an audit
//! trail. The listener shuts down cleanly via an `Arc<AtomicBool>` shutdown
//! signal — no hard timeout.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
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
            let _ = stream.write_all(msg.as_bytes());
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
        }
    }
}

// ---------------------------------------------------------------------------
// Messages
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
            line.strip_prefix("DONE:")
                .map(|rest| Self::Done(rest.to_string()))
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

    /// Whether this is a significant event that should trigger an immediate Telegram send.
    pub fn is_significant(&self) -> bool {
        matches!(self, Self::Event(_) | Self::Done(_))
    }
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// Handle returned by `create_listener`. Drop it to signal shutdown.
pub struct ListenerHandle {
    shutdown: Arc<AtomicBool>,
    sock_path: PathBuf,
}

impl ListenerHandle {
    /// Signal the listener to shut down and clean up the socket.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// Get the socket path (for passing via env var).
    pub fn sock_path(&self) -> &Path {
        &self.sock_path
    }
}

impl Drop for ListenerHandle {
    fn drop(&mut self) {
        self.shutdown();
        // Give listener thread a moment to clean up
        thread::sleep(std::time::Duration::from_millis(100));
        // Ensure socket is removed even if listener thread didn't get to it
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

/// Creates a Unix socket listener that accepts multiple client connections,
/// forwards parsed messages to an mpsc channel, and writes all messages to
/// `progress.log` as an audit trail. Shuts down cleanly when the returned
/// `ListenerHandle` is dropped or `shutdown()` is called.
///
/// Returns (handle, message_receiver).
pub fn create_listener(
    harness_dir: &Path,
) -> Result<(ListenerHandle, mpsc::Receiver<ProgressMsg>), String> {
    let sock_path = harness_dir.join("progress.sock");
    let log_path = harness_dir.join("progress.log");

    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::write(&log_path, "");

    let listener = UnixListener::bind(&sock_path)
        .map_err(|e| format!("Failed to create progress socket: {e}"))?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();

    let shutdown_clone = Arc::clone(&shutdown);
    let path_clone = sock_path.clone();

    thread::spawn(move || {
        let _ = listener.set_nonblocking(true);
        let mut client_threads: Vec<thread::JoinHandle<()>> = Vec::new();

        loop {
            if shutdown_clone.load(Ordering::Acquire) {
                break;
            }

            match listener.accept() {
                Ok((stream, _)) => {
                    let tx = tx.clone();
                    let log = log_path.clone();
                    let shut = Arc::clone(&shutdown_clone);
                    let handle = thread::spawn(move || {
                        handle_client(stream, tx, &log, &shut);
                    });
                    client_threads.push(handle);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    client_threads.retain(|h| !h.is_finished());
                    thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }

        for h in client_threads {
            let _ = h.join();
        }
        let _ = std::fs::remove_file(&path_clone);
    });

    let handle = ListenerHandle {
        shutdown,
        sock_path,
    };

    Ok((handle, rx))
}

/// Handle a single client connection: read lines, parse, forward, and log.
fn handle_client(
    stream: UnixStream,
    tx: mpsc::Sender<ProgressMsg>,
    log_path: &Path,
    shutdown: &AtomicBool,
) {
    // Set a read timeout so we can check the shutdown flag periodically
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    let reader = BufReader::new(stream);

    for line in reader.lines() {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let Ok(line) = line else { break };

        append_to_log(log_path, &line);

        if let Some(msg) = ProgressMsg::parse(&line)
            && tx.send(msg).is_err()
        {
            break;
        }
    }
}

/// Append a raw line to the progress log file.
fn append_to_log(log_path: &Path, raw_line: &str) {
    let timestamp = chrono::Local::now().format("%H:%M:%S");
    let display = if let Some(msg) = ProgressMsg::parse(raw_line) {
        msg.display_line()
    } else {
        raw_line.to_string()
    };
    let entry = format!("[{timestamp}] {display}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| f.write_all(entry.as_bytes()));
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
    fn test_is_significant() {
        assert!(ProgressMsg::Event("step".into()).is_significant());
        assert!(ProgressMsg::Done("ok".into()).is_significant());
        assert!(!ProgressMsg::Stdout("a".into(), "b".into()).is_significant());
    }

    #[test]
    fn test_socket_roundtrip() {
        let tmp =
            std::env::temp_dir().join(format!("harness-progress-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let (handle, rx) = create_listener(&tmp).unwrap();

        thread::sleep(std::time::Duration::from_millis(50));

        let sender = ProgressSender::connect(handle.sock_path()).expect("should connect");
        sender.event("step started");
        sender.stdout("builder", "compiling main.rs");
        sender.done("completed");
        drop(sender);

        thread::sleep(std::time::Duration::from_millis(100));
        let mut msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }

        assert!(msgs.len() >= 3, "expected 3 msgs, got {}", msgs.len());
        assert!(matches!(&msgs[0], ProgressMsg::Event(s) if s == "step started"));
        assert!(
            matches!(&msgs[1], ProgressMsg::Stdout(a, l) if a == "builder" && l == "compiling main.rs")
        );
        assert!(matches!(&msgs[2], ProgressMsg::Done(s) if s == "completed"));

        // Verify progress.log was written
        let log = std::fs::read_to_string(tmp.join("progress.log")).unwrap_or_default();
        assert!(log.contains("step started"));
        assert!(log.contains("[builder] compiling main.rs"));

        // Clean shutdown
        handle.shutdown();
        thread::sleep(std::time::Duration::from_millis(200));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_multiple_clients() {
        let tmp =
            std::env::temp_dir().join(format!("harness-progress-multi-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let (handle, rx) = create_listener(&tmp).unwrap();
        thread::sleep(std::time::Duration::from_millis(50));

        let s1 = ProgressSender::connect(handle.sock_path()).expect("client 1");
        let s2 = ProgressSender::connect(handle.sock_path()).expect("client 2");

        s1.event("from client 1");
        s2.event("from client 2");
        drop(s1);
        drop(s2);

        thread::sleep(std::time::Duration::from_millis(200));
        let mut msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }

        assert!(msgs.len() >= 2, "expected 2 msgs, got {}", msgs.len());

        handle.shutdown();
        thread::sleep(std::time::Duration::from_millis(100));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_shutdown_cleans_socket() {
        let tmp =
            std::env::temp_dir().join(format!("harness-progress-shutdown-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let (handle, _rx) = create_listener(&tmp).unwrap();
        let sock = tmp.join("progress.sock");
        assert!(sock.exists(), "socket should exist after create");

        drop(handle); // triggers shutdown + cleanup
        thread::sleep(std::time::Duration::from_millis(200));
        assert!(!sock.exists(), "socket should be removed after drop");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

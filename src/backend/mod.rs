//! Pluggable agent backends for Harness.
//!
//! This module defines the `AgentBackend` trait and concrete implementations
//! for different execution environments (Claude Code, Codex, Mock, and future Grok Build).
//!
//! The goal of this abstraction (PR 1) is to eliminate the duplicated `match Backend`
//! logic that was previously scattered across `cli_backend.rs` and make it cheap to
//! add new backends (especially Grok-native execution).

use std::path::PathBuf;

use crate::xdg; // for cache paths used by some backends

/// The main trait for all agent backends.
///
/// Implementations are responsible for:
/// - Spawning / communicating with the underlying agent (Claude Code CLI, Codex CLI, Grok session, etc.)
/// - Handling model selection, timeouts, streaming, and final output extraction
/// - Any backend-specific features (SCL MCP injection for Claude, GitHub MCP for Grok, etc.)
pub trait AgentBackend: Send + Sync {
    /// Human-readable name of the backend (used in logs, doctor, UI).
    fn name(&self) -> &'static str;

    /// Returns whether this backend is currently available on the system.
    /// For Claude/Codex this typically means "is the binary in PATH and authenticated?"
    fn is_available(&self) -> Result<bool, String>;

    /// Short description shown by `harness doctor` and help text.
    fn description(&self) -> String;

    /// Run a one-shot prompt (used by planner and evaluator).
    fn run_oneshot(
        &self,
        model: &str,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<String, String>;

    /// Run the builder phase (longer-running, full project context, file I/O expected).
    fn run_builder(
        &self,
        model: &str,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<String, String>;

    /// Streaming variant of `run_oneshot` (for TUI live output).
    fn run_oneshot_streaming(
        &self,
        model: &str,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<StreamingProcess, String>;

    /// Streaming variant of `run_builder` (for TUI live output during build).
    fn run_builder_streaming(
        &self,
        model: &str,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<StreamingProcess, String>;
}

/// Handle to a streaming agent process.
/// Lines arrive on `lines`. Call `wait()` to get the final collected output.
pub struct StreamingProcess {
    pub lines: std::sync::mpsc::Receiver<String>,
    // Internal implementation details are backend-specific and hidden behind the trait.
    inner: Box<dyn std::any::Any + Send>,
    kill_fn: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
    wait_fn: Box<dyn FnOnce() -> Result<String, String> + Send>,
}

impl StreamingProcess {
    pub fn kill(&self) -> Result<(), String> {
        (self.kill_fn)()
    }

    pub fn wait(self) -> Result<String, String> {
        (self.wait_fn)()
    }

    /// Internal constructor used by concrete backends.
    pub(crate) fn new(
        lines: std::sync::mpsc::Receiver<String>,
        inner: Box<dyn std::any::Any + Send>,
        kill_fn: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
        wait_fn: Box<dyn FnOnce() -> Result<String, String> + Send>,
    ) -> Self {
        Self {
            lines,
            inner,
            kill_fn,
            wait_fn,
        }
    }
}

/// Factory to obtain a backend by name.
/// This replaces the old `Backend::from_str` + free functions.
pub fn get_backend(name: &str) -> Result<Box<dyn AgentBackend>, String> {
    match name {
        "claude" => Ok(Box::new(ClaudeBackend::new())),
        "codex" => Ok(Box::new(CodexBackend::new())),
        "mock" => Ok(Box::new(MockBackend::new())),
        "grok" => Ok(Box::new(GrokBackend::new())),
        _ => Err(format!(
            "Unknown backend: '{name}'. Supported: claude, codex, mock, grok (grok requires running inside a Grok Build session)."
        )),
    }
}

// Re-export concrete types so call sites can do `use crate::backend::{get_backend, StreamingProcess};`
mod claude;
mod codex;
mod mock;
mod grok; // stub for now

pub use claude::ClaudeBackend;
pub use codex::CodexBackend;
pub use mock::MockBackend;
pub use grok::GrokBackend;

// ============================================================================
// Shared helpers (used by multiple backends)
// These will be moved/refined as we fully extract logic from the old cli_backend.
// ============================================================================

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) fn wait_with_timeout(
    child: &mut Child,
    timeout_secs: u64,
) -> Result<std::process::Output, String> {
    let timeout = Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    std::io::Read::read_to_end(&mut out, &mut stdout)
                        .map_err(|e| format!("Failed to read stdout: {e}"))?;
                }
                if let Some(mut err) = child.stderr.take() {
                    std::io::Read::read_to_end(&mut err, &mut stderr)
                        .map_err(|e| format!("Failed to read stderr: {e}"))?;
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(format!("Process timed out after {timeout_secs}s"));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(e) => return Err(format!("Failed to wait for process: {e}")),
        }
    }
}

pub(crate) fn spawn_streaming_no_stdin(
    mut cmd: Command,
    timeout_secs: u64,
) -> Result<StreamingProcess, String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn process: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;

    let (tx, rx) = mpsc::channel();
    let reader_thread = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut all_lines = Vec::new();
        for line in reader.lines() {
            let Ok(raw) = line else { break };
            if !raw.trim().is_empty() {
                all_lines.push(raw.clone());
                let _ = tx.send(raw);
            }
        }
        all_lines
    });

    let child = Arc::new(Mutex::new(child));
    let child_for_kill = child.clone();
    let child_for_wait = child.clone();

    Ok(StreamingProcess::new(
        rx,
        Box::new(child),
        Box::new(move || {
            let mut c = child_for_kill.lock().map_err(|_| "Failed to lock child".to_string())?;
            c.kill().map_err(|e| format!("Failed to kill process: {e}"))
        }),
        Box::new(move || {
            let mut c = child_for_wait.lock().map_err(|_| "Failed to lock child".to_string())?;
            let status = c.wait().map_err(|e| format!("Failed to wait: {e}"))?;
            if !status.success() {
                return Err("Process failed".to_string());
            }
            Ok("".to_string())
        }),
    ))
}

// Temporary re-exports during migration so existing code continues to compile.
// These will be removed once all call sites are updated to the new trait.
pub mod legacy {
    pub use super::get_backend;
    pub use super::StreamingProcess;
    pub use super::AgentBackend;
}

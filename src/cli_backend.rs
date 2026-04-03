use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::global_config::{self, GlobalConfig};

pub enum Backend {
    Claude,
    Codex,
    Mock,
}

impl Backend {
    pub fn from_str(s: &str) -> Result<Backend, String> {
        match s {
            "claude" => Ok(Backend::Claude),
            "codex" => Ok(Backend::Codex),
            "mock" => Ok(Backend::Mock),
            _ => Err(format!("Unknown backend: {s}. Use 'claude', 'codex', or 'mock'.")),
        }
    }
}

/// Build the base claude command with model, permissions, and optional MCP config.
fn claude_cmd(model: &str) -> Command {
    let mut cmd = Command::new("claude");
    cmd.args(["--print", "--permission-mode", "bypassPermissions", "--model", model]);

    // Inject SCL MCP config if enabled and reachable
    let gc = GlobalConfig::load();
    if let Some(scl) = gc.scl()
        && global_config::check_scl_health(scl.url())
        && let Ok(mcp_path) = global_config::generate_mcp_config(scl.url())
    {
        cmd.arg("--mcp-config");
        cmd.arg(mcp_path);
        eprintln!("[scl] Connected: {}", scl.url());
    }

    cmd
}

/// Run a one-shot prompt through the backend and return the output.
pub fn run_oneshot(backend: &Backend, model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    match backend {
        Backend::Claude => run_claude_oneshot(model, prompt, timeout_secs),
        Backend::Codex => run_codex_oneshot(prompt, timeout_secs),
        Backend::Mock => Ok(mock_response("oneshot")),
    }
}

/// Run the builder (full agent session with file I/O).
pub fn run_builder(backend: &Backend, model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    match backend {
        Backend::Claude => run_claude_builder(model, prompt, timeout_secs),
        Backend::Codex => run_codex_builder(prompt, timeout_secs),
        Backend::Mock => Ok(mock_response("builder")),
    }
}

fn mock_response(phase: &str) -> String {
    format!("# Mock {phase} response\n\nThis is a mock response for testing.\n\nVERDICT: PASS\n\nSCORES:\n  functionality: 8/10\n  completeness: 8/10\n  code_quality: 8/10\n  design_quality: 7/10\n  robustness: 7/10\n")
}

fn run_claude_oneshot(model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    let mut cmd = claude_cmd(model);
    cmd.args(["-p", prompt]);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn claude: {e}"))?;

    let output = wait_with_timeout(&mut child, timeout_secs)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("claude exited with error: {stderr}"));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8 in claude output: {e}"))
}

fn run_claude_builder(model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    let mut cmd = claude_cmd(model);
    cmd.args(["-p", prompt]);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn claude builder: {e}"))?;

    let output = wait_with_timeout(&mut child, timeout_secs)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("claude builder exited with error: {stderr}"));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8 in builder output: {e}"))
}

fn run_codex_oneshot(prompt: &str, timeout_secs: u64) -> Result<String, String> {
    let mut child = Command::new("codex")
        .args(["exec", "-q"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn codex: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())
            .map_err(|e| format!("Failed to write to codex stdin: {e}"))?;
    }

    let output = wait_with_timeout(&mut child, timeout_secs)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("codex exited with error: {stderr}"));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8 in codex output: {e}"))
}

fn run_codex_builder(prompt: &str, timeout_secs: u64) -> Result<String, String> {
    let mut child = Command::new("codex")
        .args(["exec", "-q", "--full-auto"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn codex builder: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())
            .map_err(|e| format!("Failed to write to codex builder stdin: {e}"))?;
    }

    let output = wait_with_timeout(&mut child, timeout_secs)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("codex builder exited with error: {stderr}"));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8 in codex builder output: {e}"))
}

/// A handle to a streaming process. Lines arrive on the receiver.
/// Call `wait()` to get the final collected output.
pub struct StreamingProcess {
    pub lines: mpsc::Receiver<String>,
    child: std::process::Child,
    reader_thread: Option<std::thread::JoinHandle<Vec<String>>>,
    timeout_secs: u64,
}

impl StreamingProcess {
    /// Wait for process to finish, return the full collected output.
    pub fn wait(mut self) -> Result<String, String> {
        let all_lines = self.reader_thread.take()
            .expect("reader thread missing")
            .join()
            .map_err(|_| "Reader thread panicked".to_string())?;

        let timeout = Duration::from_secs(self.timeout_secs);
        let start = std::time::Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        let mut stderr_buf = Vec::new();
                        if let Some(mut err) = self.child.stderr.take() {
                            let _ = std::io::Read::read_to_end(&mut err, &mut stderr_buf);
                        }
                        let stderr = String::from_utf8_lossy(&stderr_buf);
                        return Err(format!("Process exited with error: {stderr}"));
                    }
                    return Ok(all_lines.join("\n"));
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = self.child.kill();
                        return Err(format!("Process timed out after {}s", self.timeout_secs));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(format!("Failed to wait for process: {e}")),
            }
        }
    }
}

/// Spawn a command with no stdin and stream stdout line-by-line.
fn spawn_streaming_no_stdin(mut cmd: Command, timeout_secs: u64) -> Result<StreamingProcess, String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn process: {e}"))?;

    let stdout = child.stdout.take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;

    let (tx, rx) = mpsc::channel();
    let reader_thread = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut all_lines = Vec::new();
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    all_lines.push(l.clone());
                    let _ = tx.send(l);
                }
                Err(_) => break,
            }
        }
        all_lines
    });

    Ok(StreamingProcess {
        lines: rx,
        child,
        reader_thread: Some(reader_thread),
        timeout_secs,
    })
}

/// Spawn a command and stream its stdout line-by-line.
fn spawn_streaming(mut cmd: Command, prompt: &str, timeout_secs: u64) -> Result<StreamingProcess, String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn process: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())
            .map_err(|e| format!("Failed to write to stdin: {e}"))?;
    }

    let stdout = child.stdout.take()
        .ok_or_else(|| "Failed to capture stdout".to_string())?;

    let (tx, rx) = mpsc::channel();
    let reader_thread = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut all_lines = Vec::new();
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    all_lines.push(l.clone());
                    let _ = tx.send(l);
                }
                Err(_) => break,
            }
        }
        all_lines
    });

    Ok(StreamingProcess {
        lines: rx,
        child,
        reader_thread: Some(reader_thread),
        timeout_secs,
    })
}

/// Streaming variant of run_oneshot.
pub fn run_oneshot_streaming(backend: &Backend, model: &str, prompt: &str, timeout_secs: u64) -> Result<StreamingProcess, String> {
    match backend {
        Backend::Claude => {
            let mut cmd = claude_cmd(model);
            cmd.args(["-p", prompt]);
            spawn_streaming_no_stdin(cmd, timeout_secs)
        }
        Backend::Codex => {
            let mut cmd = Command::new("codex");
            cmd.args(["exec", "-q"]);
            spawn_streaming(cmd, prompt, timeout_secs)
        }
        Backend::Mock => spawn_mock_streaming("oneshot"),
    }
}

/// Streaming variant of run_builder.
pub fn run_builder_streaming(backend: &Backend, model: &str, prompt: &str, timeout_secs: u64) -> Result<StreamingProcess, String> {
    match backend {
        Backend::Claude => {
            let mut cmd = claude_cmd(model);
            cmd.args(["-p", prompt]);
            spawn_streaming_no_stdin(cmd, timeout_secs)
        }
        Backend::Codex => {
            let mut cmd = Command::new("codex");
            cmd.args(["exec", "-q", "--full-auto"]);
            spawn_streaming(cmd, prompt, timeout_secs)
        }
        Backend::Mock => spawn_mock_streaming("builder"),
    }
}

fn spawn_mock_streaming(phase: &str) -> Result<StreamingProcess, String> {
    let mut cmd = Command::new("echo");
    cmd.arg(mock_response(phase));
    spawn_streaming_no_stdin(cmd, 10)
}

fn wait_with_timeout(child: &mut std::process::Child, timeout_secs: u64) -> Result<std::process::Output, String> {
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

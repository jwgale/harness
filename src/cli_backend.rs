use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub enum Backend {
    Claude,
    Codex,
}

impl Backend {
    pub fn from_str(s: &str) -> Result<Backend, String> {
        match s {
            "claude" => Ok(Backend::Claude),
            "codex" => Ok(Backend::Codex),
            _ => Err(format!("Unknown backend: {s}. Use 'claude' or 'codex'.")),
        }
    }
}

/// Run a one-shot prompt through the backend and return the output.
/// Used for planner and evaluator.
pub fn run_oneshot(backend: &Backend, model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    match backend {
        Backend::Claude => run_claude_oneshot(model, prompt, timeout_secs),
        Backend::Codex => run_codex_oneshot(prompt, timeout_secs),
    }
}

/// Run the builder (full agent session with file I/O).
/// The builder reads/writes files directly in the working directory.
pub fn run_builder(backend: &Backend, model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    match backend {
        Backend::Claude => run_claude_builder(model, prompt, timeout_secs),
        Backend::Codex => run_codex_builder(prompt, timeout_secs),
    }
}

fn run_claude_oneshot(model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    let mut child = Command::new("claude")
        .args(["--print", "--permission-mode", "bypassPermissions", "--model", model, "-p", prompt])
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
    // Builder uses --print mode with full agent capabilities
    // Pass prompt as -p argument, null stdin to prevent blocking
    let mut child = Command::new("claude")
        .args(["--print", "--permission-mode", "bypassPermissions", "--model", model, "-p", prompt])
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
                        // Collect stderr
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
                    // If receiver is dropped, keep reading to drain the pipe
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
    let cmd = match backend {
        Backend::Claude => {
            let mut c = Command::new("claude");
            c.args(["--print", "--permission-mode", "bypassPermissions", "--model", model, "-p"]);
            c
        }
        Backend::Codex => {
            let mut c = Command::new("codex");
            c.args(["exec", "-q"]);
            c
        }
    };
    spawn_streaming(cmd, prompt, timeout_secs)
}

/// Streaming variant of run_builder.
pub fn run_builder_streaming(backend: &Backend, model: &str, prompt: &str, timeout_secs: u64) -> Result<StreamingProcess, String> {
    let cmd = match backend {
        Backend::Claude => {
            let mut c = Command::new("claude");
            c.args(["--print", "--permission-mode", "bypassPermissions", "--model", model, "-p"]);
            c
        }
        Backend::Codex => {
            let mut c = Command::new("codex");
            c.args(["exec", "-q", "--full-auto"]);
            c
        }
    };
    spawn_streaming(cmd, prompt, timeout_secs)
}

fn wait_with_timeout(child: &mut std::process::Child, timeout_secs: u64) -> Result<std::process::Output, String> {
    let timeout = Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                // Process finished, collect output
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
                    status: _status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                // Still running
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

use std::io::Write;
use std::process::{Command, Stdio};
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
        .args(["--print", "--dangerously-skip-permissions", "--model", model, "-p"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn claude: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())
            .map_err(|e| format!("Failed to write to claude stdin: {e}"))?;
    }

    let output = wait_with_timeout(&mut child, timeout_secs)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("claude exited with error: {stderr}"));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8 in claude output: {e}"))
}

fn run_claude_builder(model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    // Builder also uses --print mode but with full agent capabilities
    // Claude Code in --print mode with --dangerously-skip-permissions can still
    // read/write files and execute commands
    let mut child = Command::new("claude")
        .args(["--print", "--dangerously-skip-permissions", "--model", model, "-p"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn claude builder: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())
            .map_err(|e| format!("Failed to write to claude builder stdin: {e}"))?;
    }

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

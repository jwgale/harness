use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::global_config::GlobalConfig;
use crate::scl;
use crate::xdg;

#[derive(Debug, Clone, Copy)]
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
            _ => Err(format!(
                "Unknown backend: {s}. Use 'claude', 'codex', or 'mock'."
            )),
        }
    }
}

#[derive(Clone, Copy)]
enum CodexMode {
    OneShot,
    Builder,
}

#[derive(Clone, Copy)]
enum StreamFormat {
    Plain,
    CodexJson,
}

/// Build the base Codex exec command.
///
/// Codex CLI 0.124 does not accept the historical `-q` flag for `exec`.
/// We use explicit non-interactive flags and `--output-last-message` so callers
/// can depend on the final assistant message instead of scraping terminal text.
fn codex_cmd(mode: CodexMode, model: &str, json: bool, final_output: Option<&Path>) -> Command {
    let mut cmd = Command::new("codex");
    cmd.args([
        "exec",
        "--skip-git-repo-check",
        "--ask-for-approval",
        "never",
    ]);
    if let Ok(cwd) = std::env::current_dir() {
        cmd.arg("--cd");
        cmd.arg(cwd);
    }
    if should_pass_codex_model(model) {
        cmd.args(["--model", model]);
    }
    if json {
        cmd.arg("--json");
    }
    if let Some(path) = final_output {
        cmd.arg("--output-last-message");
        cmd.arg(path);
    }
    if matches!(mode, CodexMode::Builder) {
        if codex_dangerous_enabled() {
            cmd.arg("--dangerously-bypass-approvals-and-sandbox");
        } else {
            cmd.arg("--full-auto");
        }
    }
    cmd
}

fn should_pass_codex_model(model: &str) -> bool {
    let model = model.trim();
    !model.is_empty() && model != "default" && !model.starts_with("claude-")
}

fn codex_dangerous_enabled() -> bool {
    std::env::var("HARNESS_CODEX_DANGEROUS")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn codex_last_message_path(label: &str) -> Result<PathBuf, String> {
    let dir = xdg::cache_dir().join("codex");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create Codex cache directory: {e}"))?;
    let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    Ok(dir.join(format!("{label}-{}-{stamp}.md", std::process::id())))
}

fn read_final_output(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok();
    let _ = std::fs::remove_file(path);
    content.filter(|s| !s.trim().is_empty())
}

fn claude_should_pass_model(model: &str) -> bool {
    let model = model.trim();
    !model.is_empty() && model != "default"
}

fn codex_json_display_line(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "exec_command_begin" | "exec_command.start" | "command_started" => {
            find_string(&value, &["command", "cmd"]).map(|cmd| format!("$ {cmd}"))
        }
        "exec_command_end" | "exec_command.finish" | "command_finished" => {
            let code =
                find_string(&value, &["exit_code", "status"]).unwrap_or_else(|| "done".to_string());
            Some(format!("command finished: {code}"))
        }
        "exec_command_output_delta" | "command_output_delta" => {
            find_string(&value, &["delta", "chunk", "text"])
        }
        "agent_message" | "assistant_message" | "message" => {
            find_string(&value, &["text", "content", "message"])
        }
        "error" => find_string(&value, &["message", "error"]).map(|msg| format!("error: {msg}")),
        _ => find_string(&value, &["message", "text", "delta"]).filter(|s| !s.trim().is_empty()),
    }
}

fn find_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(s) = map.get(*key).and_then(value_to_string) {
                    return Some(s);
                }
            }
            for child in map.values() {
                if matches!(
                    child,
                    serde_json::Value::Object(_) | serde_json::Value::Array(_)
                ) && let Some(found) = find_string(child, keys)
                {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(items) => {
            for child in items {
                if let Some(found) = find_string(child, keys) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Build the base claude command with model, permissions, and optional MCP config.
fn claude_cmd(model: &str) -> Command {
    let mut cmd = Command::new("claude");
    cmd.args(["--print", "--permission-mode", "bypassPermissions"]);
    if claude_should_pass_model(model) {
        cmd.args(["--model", model]);
    }

    // Inject SCL MCP config if enabled and reachable
    let gc = GlobalConfig::load();
    if let Some(scl_cfg) = gc.scl()
        && scl::is_healthy(scl_cfg.url())
        && let Ok(mcp_path) = scl::generate_mcp_config(scl_cfg.url())
    {
        cmd.arg("--mcp-config");
        cmd.arg(mcp_path);
        eprintln!("[scl] Connected: {}", scl_cfg.url());
    }

    cmd
}

/// Run a one-shot prompt through the backend and return the output.
pub fn run_oneshot(
    backend: &Backend,
    model: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    match backend {
        Backend::Claude => run_claude_oneshot(model, prompt, timeout_secs),
        Backend::Codex => run_codex_oneshot(model, prompt, timeout_secs),
        Backend::Mock => Ok(mock_response("oneshot")),
    }
}

/// Run the builder (full agent session with file I/O).
pub fn run_builder(
    backend: &Backend,
    model: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    match backend {
        Backend::Claude => run_claude_builder(model, prompt, timeout_secs),
        Backend::Codex => run_codex_builder(model, prompt, timeout_secs),
        Backend::Mock => Ok(mock_response("builder")),
    }
}

fn mock_response(phase: &str) -> String {
    format!(
        "# Mock {phase} response\n\nThis is a mock response for testing.\n\nVERDICT: PASS\n\nSCORES:\n  functionality: 8/10\n  completeness: 8/10\n  code_quality: 8/10\n  design_quality: 7/10\n  robustness: 7/10\n"
    )
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

    String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8 in claude output: {e}"))
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

    String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8 in builder output: {e}"))
}

fn run_codex_oneshot(model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    run_codex_blocking(CodexMode::OneShot, "oneshot", model, prompt, timeout_secs)
}

fn run_codex_builder(model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    run_codex_blocking(CodexMode::Builder, "builder", model, prompt, timeout_secs)
}

fn run_codex_blocking(
    mode: CodexMode,
    label: &str,
    model: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let final_path = codex_last_message_path(label)?;
    let mut child = codex_cmd(mode, model, false, Some(&final_path))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn codex: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .map_err(|e| format!("Failed to write to codex stdin: {e}"))?;
    }

    let output = wait_with_timeout(&mut child, timeout_secs)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let _ = std::fs::remove_file(&final_path);
        return Err(format!("codex exited with error: {stderr}{stdout}"));
    }

    if let Some(final_output) = read_final_output(&final_path) {
        return Ok(final_output);
    }

    String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8 in codex output: {e}"))
}

/// A handle to a streaming process. Lines arrive on the receiver.
/// Call `wait()` to get the final collected output.
pub struct StreamingProcess {
    pub lines: mpsc::Receiver<String>,
    child: Arc<Mutex<std::process::Child>>,
    reader_thread: Option<std::thread::JoinHandle<Vec<String>>>,
    timeout_secs: u64,
    final_output_path: Option<PathBuf>,
}

impl StreamingProcess {
    pub fn kill(&self) -> Result<(), String> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| "Failed to lock child process".to_string())?;
        child
            .kill()
            .map_err(|e| format!("Failed to kill process: {e}"))
    }

    /// Wait for process to finish, return the final assistant message when available.
    pub fn wait(mut self) -> Result<String, String> {
        let timeout = Duration::from_secs(self.timeout_secs);
        let start = std::time::Instant::now();
        let status = loop {
            let mut child = self
                .child
                .lock()
                .map_err(|_| "Failed to lock child process".to_string())?;
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        drop(child);
                        if let Some(reader) = self.reader_thread.take() {
                            let _ = reader.join();
                        }
                        return Err(format!("Process timed out after {}s", self.timeout_secs));
                    }
                    drop(child);
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(format!("Failed to wait for process: {e}")),
            }
        };

        let all_lines = self
            .reader_thread
            .take()
            .expect("reader thread missing")
            .join()
            .map_err(|_| "Reader thread panicked".to_string())?;

        if !status.success() {
            let mut stderr_buf = Vec::new();
            let mut child = self
                .child
                .lock()
                .map_err(|_| "Failed to lock child process".to_string())?;
            if let Some(mut err) = child.stderr.take() {
                let _ = std::io::Read::read_to_end(&mut err, &mut stderr_buf);
            }
            let stderr = String::from_utf8_lossy(&stderr_buf);
            return Err(format!("Process exited with error: {stderr}"));
        }

        if let Some(path) = self.final_output_path.take()
            && let Some(final_output) = read_final_output(&path)
        {
            return Ok(final_output);
        }

        Ok(all_lines.join("\n"))
    }
}

/// Spawn a command with no stdin and stream stdout line-by-line.
fn spawn_streaming_no_stdin(
    mut cmd: Command,
    timeout_secs: u64,
) -> Result<StreamingProcess, String> {
    spawn_streaming_inner(&mut cmd, None, timeout_secs, StreamFormat::Plain, None)
}

/// Spawn a command and stream its stdout line-by-line.
fn spawn_streaming(
    mut cmd: Command,
    prompt: &str,
    timeout_secs: u64,
    format: StreamFormat,
    final_output_path: Option<PathBuf>,
) -> Result<StreamingProcess, String> {
    spawn_streaming_inner(
        &mut cmd,
        Some(prompt),
        timeout_secs,
        format,
        final_output_path,
    )
}

fn spawn_streaming_inner(
    cmd: &mut Command,
    prompt: Option<&str>,
    timeout_secs: u64,
    format: StreamFormat,
    final_output_path: Option<PathBuf>,
) -> Result<StreamingProcess, String> {
    let stdin = if prompt.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = cmd
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn process: {e}"))?;

    if let Some(prompt) = prompt
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin
            .write_all(prompt.as_bytes())
            .map_err(|e| format!("Failed to write to stdin: {e}"))?;
    }

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
            let display = match format {
                StreamFormat::Plain => Some(raw),
                StreamFormat::CodexJson => codex_json_display_line(&raw),
            };
            if let Some(line) = display
                && !line.trim().is_empty()
            {
                all_lines.push(line.clone());
                let _ = tx.send(line);
            }
        }
        all_lines
    });

    Ok(StreamingProcess {
        lines: rx,
        child: Arc::new(Mutex::new(child)),
        reader_thread: Some(reader_thread),
        timeout_secs,
        final_output_path,
    })
}

/// Streaming variant of run_oneshot.
pub fn run_oneshot_streaming(
    backend: &Backend,
    model: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<StreamingProcess, String> {
    match backend {
        Backend::Claude => {
            let mut cmd = claude_cmd(model);
            cmd.args(["-p", prompt]);
            spawn_streaming_no_stdin(cmd, timeout_secs)
        }
        Backend::Codex => {
            let final_path = codex_last_message_path("oneshot-stream")?;
            let cmd = codex_cmd(CodexMode::OneShot, model, true, Some(&final_path));
            spawn_streaming(
                cmd,
                prompt,
                timeout_secs,
                StreamFormat::CodexJson,
                Some(final_path),
            )
        }
        Backend::Mock => spawn_mock_streaming("oneshot"),
    }
}

/// Streaming variant of run_builder.
pub fn run_builder_streaming(
    backend: &Backend,
    model: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<StreamingProcess, String> {
    match backend {
        Backend::Claude => {
            let mut cmd = claude_cmd(model);
            cmd.args(["-p", prompt]);
            spawn_streaming_no_stdin(cmd, timeout_secs)
        }
        Backend::Codex => {
            let final_path = codex_last_message_path("builder-stream")?;
            let cmd = codex_cmd(CodexMode::Builder, model, true, Some(&final_path));
            spawn_streaming(
                cmd,
                prompt,
                timeout_secs,
                StreamFormat::CodexJson,
                Some(final_path),
            )
        }
        Backend::Mock => spawn_mock_streaming("builder"),
    }
}

fn spawn_mock_streaming(phase: &str) -> Result<StreamingProcess, String> {
    let mut cmd = Command::new("echo");
    cmd.arg(mock_response(phase));
    spawn_streaming_no_stdin(cmd, 10)
}

fn wait_with_timeout(
    child: &mut std::process::Child,
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

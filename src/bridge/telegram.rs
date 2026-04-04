//! Telegram bot bridge for controlling Harness via chat.
//!
//! Uses the Telegram Bot API via direct HTTP (long polling with curl).
//! Credentials are resolved exclusively from SanctumAI vault.
//! Permission checks use vault policies before executing commands.

use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::global_config::GlobalConfig;
use crate::progress::{self, ProgressMsg};
use crate::scl_lifecycle;
use crate::vault;
use crate::workflows;
use crate::xdg;

const POLL_TIMEOUT_SECS: u64 = 30;
const WORKFLOW_POLL_INTERVAL_SECS: u64 = 10;

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// Resolved Telegram bridge credentials.
struct BotCredentials {
    bot_token: String,
    chat_id: String,
}

/// Resolve bot token and chat ID from vault (required — no config fallback for bridge).
fn resolve_credentials() -> Result<BotCredentials, String> {
    let vc = vault::load_config();
    if !vc.enabled {
        return Err(
            "Vault must be enabled for Telegram bridge.\n\
             Run: harness vault init\n\
             Then: harness vault add notifications/telegram/bot-token\n\
             And:  harness vault add notifications/telegram/chat-id"
                .to_string(),
        );
    }

    let bot_token = vault::get_credential_string(&vc, "notifications/telegram/bot-token")
        .map_err(|e| format!("Failed to get bot token from vault: {e}"))?;
    let chat_id = vault::get_credential_string(&vc, "notifications/telegram/chat-id")
        .map_err(|e| format!("Failed to get chat ID from vault: {e}"))?;

    if bot_token.is_empty() {
        return Err(
            "Bot token is empty. Run: harness vault add notifications/telegram/bot-token"
                .to_string(),
        );
    }
    if chat_id.is_empty() {
        return Err(
            "Chat ID is empty. Run: harness vault add notifications/telegram/chat-id".to_string(),
        );
    }

    Ok(BotCredentials { bot_token, chat_id })
}

// ---------------------------------------------------------------------------
// Telegram Markdown escaping
// ---------------------------------------------------------------------------

/// Escape special characters for Telegram MarkdownV1.
///
/// Telegram's MarkdownV1 mode treats `_`, `*`, `` ` ``, and `[` as formatting.
/// We escape them so arbitrary output renders safely. Backtick-delimited code
/// spans in the input are preserved (contents not escaped).
fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut in_code = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // Toggle code span tracking on backtick
        if ch == '`' {
            in_code = !in_code;
            out.push(ch);
            i += 1;
            continue;
        }

        if in_code {
            // Inside code span — pass through unescaped
            out.push(ch);
        } else {
            // Outside code span — escape markdown-sensitive characters
            match ch {
                '_' | '*' | '[' => {
                    out.push('\\');
                    out.push(ch);
                }
                _ => out.push(ch),
            }
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Telegram API helpers
// ---------------------------------------------------------------------------

/// Send a text message to a Telegram chat. Tries Markdown first, falls back to plain text.
fn send_message(creds: &BotCredentials, text: &str) -> Result<(), String> {
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        creds.bot_token
    );
    // Truncate to Telegram's 4096-char limit
    let escaped = escape_markdown(text);
    let display_text = if escaped.len() > 4000 {
        format!("{}...\n(truncated)", &escaped[..4000])
    } else {
        escaped
    };

    // Try with Markdown first
    let payload = serde_json::json!({
        "chat_id": creds.chat_id,
        "text": display_text,
        "parse_mode": "Markdown"
    });
    let body = payload.to_string();
    let output = Command::new("curl")
        .args([
            "-s", "-w", "\n%{http_code}", "-X", "POST", "-H",
            "Content-Type: application/json",
            "--max-time", "10", "-d", &body, &url,
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    let raw = String::from_utf8_lossy(&output.stdout);
    let code = raw.lines().last().unwrap_or("").trim();

    if code.starts_with('2') {
        return Ok(());
    }

    // Markdown parse failed — retry as plain text (no escaping needed)
    let plain_text = if text.len() > 4000 {
        format!("{}...\n(truncated)", &text[..4000])
    } else {
        text.to_string()
    };
    let plain_payload = serde_json::json!({
        "chat_id": creds.chat_id,
        "text": plain_text,
    });
    let plain_body = plain_payload.to_string();
    let plain_output = Command::new("curl")
        .args([
            "-s", "-o", "/dev/null", "-w", "%{http_code}", "-X", "POST", "-H",
            "Content-Type: application/json",
            "--max-time", "10", "-d", &plain_body, &url,
        ])
        .output()
        .map_err(|e| format!("curl plain fallback failed: {e}"))?;

    let plain_code = String::from_utf8_lossy(&plain_output.stdout)
        .trim()
        .to_string();
    if plain_code.starts_with('2') {
        Ok(())
    } else {
        Err(format!("Telegram API returned HTTP {plain_code}"))
    }
}

/// Long-poll for updates from the Telegram Bot API.
fn get_updates(bot_token: &str, offset: i64) -> Result<Vec<Value>, String> {
    let url = format!(
        "https://api.telegram.org/bot{bot_token}/getUpdates?offset={offset}&timeout={POLL_TIMEOUT_SECS}"
    );
    let output = Command::new("curl")
        .args([
            "-s",
            "--max-time",
            &(POLL_TIMEOUT_SECS + 5).to_string(),
            &url,
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    let body = String::from_utf8_lossy(&output.stdout);
    let parsed: Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse Telegram response: {e}"))?;

    if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(format!(
            "Telegram API error: {}",
            parsed
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        ));
    }

    Ok(parsed
        .get("result")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Configurable timeout
// ---------------------------------------------------------------------------

/// Resolve workflow timeout in seconds. Priority: workflow TOML > global config > 30 min default.
fn resolve_timeout_secs(workflow_name: &str) -> u64 {
    // Check per-workflow timeout
    if let Ok(wf) = workflows::load(workflow_name)
        && let Some(mins) = wf.timeout_minutes
    {
        return mins * 60;
    }
    // Fall back to global bridge config
    let gc = GlobalConfig::load();
    gc.bridge().workflow_timeout_minutes() * 60
}

// ---------------------------------------------------------------------------
// Vault policy checks
// ---------------------------------------------------------------------------

/// Check if the vault grants the given policy (e.g. "harness:bridge:telegram:run").
/// Returns Ok(()) if allowed, Err(message) if denied.
///
/// Three config flags control behavior:
/// - `strict_policy_mode`: when true, missing policy responses deny by default
/// - `require_policy_endpoint`: when true, vault must implement `_policy`;
///   when false (default), a missing endpoint is silently allowed
///
/// The `_policy` vault endpoint is optional. Most SanctumAI vaults don't
/// implement it yet. Set `require_policy_endpoint = true` only if your
/// vault supports it.
fn check_policy(policy: &str) -> Result<(), String> {
    let vc = vault::load_config();
    if !vc.enabled {
        return Ok(()); // vault disabled → no policy enforcement
    }

    let gc = GlobalConfig::load();
    let bridge_cfg = gc.bridge();
    let strict = bridge_cfg.strict_policy_mode();
    let require_endpoint = bridge_cfg.require_policy_endpoint();

    let params = serde_json::json!({ "policy": policy });
    match vault::use_credential(&vc, "_policy", "check", params) {
        Ok(result) => {
            let allowed = result
                .get("allowed")
                .and_then(|v| v.as_bool())
                .unwrap_or(!strict); // strict → deny when field missing
            if allowed {
                Ok(())
            } else {
                let reason = result
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("policy denied");
                Err(format!("Permission denied: {reason}\nPolicy: `{policy}`"))
            }
        }
        Err(e) if require_endpoint => {
            // User explicitly requires _policy — treat errors as denials
            Err(format!(
                "Permission denied: policy check failed ({e})\n\
                 Policy: `{policy}`\n\
                 Hint: set `require_policy_endpoint = false` if your vault doesn't support policies"
            ))
        }
        Err(_) if strict => {
            // Strict mode but _policy is optional — deny but note it's optional
            Err(format!(
                "Permission denied: vault policy check unavailable (strict mode)\nPolicy: `{policy}`"
            ))
        }
        // Non-strict, endpoint not required: silently allow
        Err(_) => Ok(()),
    }
}

/// Map a command name to its policy string.
fn policy_for_command(cmd: &str) -> &str {
    match cmd {
        "/run" => "harness:bridge:telegram:run",
        "/status" => "harness:bridge:telegram:status",
        "/agent" => "harness:bridge:telegram:agent",
        "/vault" => "harness:bridge:telegram:vault",
        _ => "harness:bridge:telegram:other",
    }
}

// ---------------------------------------------------------------------------
// Command parsing
// ---------------------------------------------------------------------------

/// Parse a Telegram command from message text. Returns (command, args).
fn parse_command(text: &str) -> Option<(&str, &str)> {
    let text = text.trim();
    if !text.starts_with('/') {
        return None;
    }
    // Strip @botname suffix from commands like /status@mybot
    let first_space = text.find(' ').unwrap_or(text.len());
    let cmd = &text[..first_space];
    let cmd = cmd.split('@').next().unwrap_or(cmd);
    let args = if first_space < text.len() {
        text[first_space..].trim()
    } else {
        ""
    };
    Some((cmd, args))
}

/// Execute a bridge command and return the response text.
/// If the command is /run with --wait or a completion callback, it returns
/// an initial ack and the callback sender handles the follow-up message.
fn handle_command(cmd: &str, args: &str, creds: &Arc<BotCredentials>) -> String {
    // Policy check
    let policy = policy_for_command(cmd);
    if let Err(deny) = check_policy(policy) {
        return deny;
    }

    match cmd {
        "/run" => cmd_run(args, creds),
        "/status" => cmd_status(),
        "/agent" => cmd_agent(args),
        "/vault" => cmd_vault(args),
        "/help" | "/start" => cmd_help(),
        _ => format!("Unknown command: {cmd}\nType /help for available commands."),
    }
}

// ---------------------------------------------------------------------------
// /run command with completion callback and --wait mode
// ---------------------------------------------------------------------------

/// /run <workflow-name> [--wait] — trigger a workflow with optional wait.
fn cmd_run(args: &str, creds: &Arc<BotCredentials>) -> String {
    // Parse --wait flag
    let (workflow, wait_mode) = parse_run_args(args);

    if workflow.is_empty() {
        return "Usage: /run <workflow-name> [--wait]\n\n\
                /run my-workflow — start and get result when done\n\
                /run my-workflow --wait — block with progress updates\n\n\
                List workflows with /status."
            .to_string();
    }

    // Check if workflow exists
    let wf_dir = xdg::config_dir().join("workflows");
    let wf_path = wf_dir.join(format!("{workflow}.toml"));
    if !wf_path.exists() {
        return format!(
            "Workflow '{workflow}' not found.\n\nAvailable workflows:\n{}",
            list_workflows()
        );
    }

    // Find a workspace with .harness/ to run in
    let work_dir = find_active_workspace().unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
    });

    // Create progress socket for real-time updates
    let harness_dir = work_dir.join(".harness");
    let progress_rx = progress::create_listener(&harness_dir).ok();
    let (sock_path, progress_receiver) = match progress_rx {
        Some((p, r)) => (Some(p), Some(r)),
        None => (None, None),
    };

    // Launch workflow via harness CLI
    let binary = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => return format!("Failed to find harness binary: {e}"),
    };

    let mut cmd = Command::new(&binary);
    cmd.args(["run", "--workflow", &workflow, "--no-tui"])
        .current_dir(&work_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Pass progress socket path to runner via env var
    if let Some(ref sp) = sock_path {
        cmd.env(progress::PROGRESS_SOCK_ENV, sp);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return format!("Failed to start workflow: {e}"),
    };

    let pid = child.id();
    let wf_name = workflow.to_string();
    let creds_clone = Arc::clone(creds);
    let work_dir_display = work_dir.display().to_string();
    let timeout_secs = resolve_timeout_secs(&workflow);

    if wait_mode {
        // --wait: block this thread, send progress updates, then final result
        let ack = format!(
            "Workflow '{wf_name}' started (PID {pid})\nWaiting for completion (timeout: {}m)...",
            timeout_secs / 60
        );
        let _ = send_message(&creds_clone, &ack);

        return wait_for_workflow(
            &mut child, &wf_name, &creds_clone, &work_dir, timeout_secs, progress_receiver,
        );
    }

    // Default: fire-and-forget with completion callback thread
    let callback_creds = Arc::clone(creds);
    thread::spawn(move || {
        workflow_completion_callback(
            child, &wf_name, &callback_creds, &work_dir_display, timeout_secs, progress_receiver,
        );
    });

    format!(
        "Workflow '{workflow}' started (PID {pid}) in `{}`\nYou'll get a result when it finishes (timeout: {}m).",
        work_dir.display(),
        timeout_secs / 60,
    )
}

/// Parse /run args into (workflow_name, wait_mode).
fn parse_run_args(args: &str) -> (String, bool) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    let mut workflow = String::new();
    let mut wait = false;

    for part in &parts {
        if *part == "--wait" {
            wait = true;
        } else if workflow.is_empty() {
            workflow = part.to_string();
        }
    }

    (workflow, wait)
}

/// Minimum interval between Telegram messages (rate limit).
const MIN_SEND_INTERVAL_SECS: u64 = 6;

/// Wait for a workflow child process with event-driven Telegram updates.
///
/// Instead of a fixed timer, we send updates immediately on significant events
/// (step changes, verdicts, loop iterations) while rate-limiting to at most
/// one message every 6 seconds. Stdout lines are batched and included in the
/// next send.
fn wait_for_workflow(
    child: &mut std::process::Child,
    workflow: &str,
    creds: &BotCredentials,
    work_dir: &std::path::Path,
    timeout_secs: u64,
    progress_rx: Option<std::sync::mpsc::Receiver<ProgressMsg>>,
) -> String {
    let start = std::time::Instant::now();
    let harness_dir = work_dir.join(".harness");

    // Buffered lines and event-trigger flag (shared with reader thread)
    let state = Arc::new(Mutex::new(ProgressBatchState::new()));

    // Spawn reader thread that buffers messages and flags significant events
    let state_clone = Arc::clone(&state);
    if let Some(rx) = progress_rx {
        thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                if let Ok(mut s) = state_clone.lock() {
                    if msg.is_significant() {
                        s.significant_pending = true;
                    }
                    s.push(msg.display_line());
                }
            }
        });
    }

    let mut last_send = std::time::Instant::now() - Duration::from_secs(MIN_SEND_INTERVAL_SECS);

    loop {
        // Check if process finished
        match child.try_wait() {
            Ok(Some(status)) => {
                return format_completion_result(workflow, &start, status.success(), child, &harness_dir);
            }
            Ok(None) => {}
            Err(e) => {
                return format!("Error waiting for workflow: {e}");
            }
        }

        // Timeout check
        if start.elapsed().as_secs() > timeout_secs {
            let _ = child.kill();
            return format!(
                "Workflow '{workflow}' timed out after {}m. Process killed.",
                timeout_secs / 60
            );
        }

        // Decide whether to send an update
        let should_send = if let Ok(s) = state.lock() {
            let rate_ok = last_send.elapsed().as_secs() >= MIN_SEND_INTERVAL_SECS;
            let has_data = !s.lines.is_empty();
            // Send immediately on significant events (if rate allows) or every 30s as fallback
            (s.significant_pending && rate_ok)
                || (has_data && last_send.elapsed().as_secs() >= 30)
        } else {
            false
        };

        if should_send {
            last_send = std::time::Instant::now();
            let msg = format_batch_update(workflow, &start, &state, &harness_dir);
            let _ = send_message(creds, &msg);
        }

        thread::sleep(Duration::from_millis(500));
    }
}

/// Batched progress state shared between reader thread and send loop.
struct ProgressBatchState {
    lines: Vec<String>,
    significant_pending: bool,
}

impl ProgressBatchState {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            significant_pending: false,
        }
    }

    fn push(&mut self, line: String) {
        self.lines.push(line);
        // Keep last 20 lines
        if self.lines.len() > 20 {
            let drain = self.lines.len() - 20;
            self.lines.drain(..drain);
        }
    }

    /// Take a snapshot of recent lines and clear the significant flag.
    fn take_snapshot(&mut self) -> Vec<String> {
        self.significant_pending = false;
        self.lines.clone()
    }
}

/// Format a batched Telegram update from the current state.
fn format_batch_update(
    workflow: &str,
    start: &std::time::Instant,
    state: &Arc<Mutex<ProgressBatchState>>,
    harness_dir: &std::path::Path,
) -> String {
    let elapsed = start.elapsed().as_secs();
    let mut out = vec![format!("Workflow '{workflow}' running... ({elapsed}s)")];

    let snapshot = if let Ok(mut s) = state.lock() {
        s.take_snapshot()
    } else {
        Vec::new()
    };

    if snapshot.is_empty() {
        // Fall back to file-based progress
        return collect_rich_progress(workflow, start, harness_dir);
    }

    // Show last 8 lines from the batch
    out.push(String::new());
    let start_idx = if snapshot.len() > 8 {
        snapshot.len() - 8
    } else {
        0
    };
    for line in &snapshot[start_idx..] {
        out.push(truncate_line(line, 120));
    }

    out.join("\n")
}

/// Background thread: wait for workflow to finish and send result to Telegram.
fn workflow_completion_callback(
    mut child: std::process::Child,
    workflow: &str,
    creds: &BotCredentials,
    work_dir: &str,
    timeout_secs: u64,
    progress_rx: Option<std::sync::mpsc::Receiver<ProgressMsg>>,
) {
    let start = std::time::Instant::now();
    let harness_dir = std::path::PathBuf::from(work_dir).join(".harness");

    // Drain progress messages in background (we don't send them for non-wait mode,
    // but we need to drain the channel so the sender doesn't block)
    if let Some(rx) = progress_rx {
        thread::spawn(move || {
            while rx.recv().is_ok() {} // drain until sender disconnects
        });
    }

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let result = format_completion_result(
                    workflow, &start, status.success(), &mut child, &harness_dir,
                );
                scl_lifecycle::record_bridge_response("telegram", "/run", &result);
                let _ = send_message(creds, &result);
                return;
            }
            Ok(None) => {
                if start.elapsed().as_secs() > timeout_secs {
                    let _ = child.kill();
                    let msg = format!(
                        "Workflow '{workflow}' timed out after {}m. Process killed.",
                        timeout_secs / 60
                    );
                    let _ = send_message(creds, &msg);
                    return;
                }
            }
            Err(e) => {
                let msg = format!("Error waiting for workflow '{workflow}': {e}");
                let _ = send_message(creds, &msg);
                return;
            }
        }

        thread::sleep(Duration::from_secs(WORKFLOW_POLL_INTERVAL_SECS));
    }
}

/// Format completion result with verdict, evaluation summary, and stderr.
fn format_completion_result(
    workflow: &str,
    start: &std::time::Instant,
    success: bool,
    child: &mut std::process::Child,
    harness_dir: &std::path::Path,
) -> String {
    let elapsed = start.elapsed().as_secs();
    let outcome = if success { "completed" } else { "failed" };
    let mut result = format!("Workflow '{workflow}' {outcome} ({elapsed}s)");

    // Read evaluation.md for verdict
    let eval_path = harness_dir.join("evaluation.md");
    if let Ok(eval) = fs::read_to_string(&eval_path) {
        let verdict = extract_verdict(&eval);
        if !verdict.is_empty() {
            result.push_str(&format!("\nVerdict: {verdict}"));
        }
        let summary = eval.lines().take(10).collect::<Vec<_>>().join("\n");
        if !summary.is_empty() {
            result.push_str(&format!("\n\n{summary}"));
        }
    }

    // Include agent completion summary if multi-agent
    let agent_summary = collect_agent_summary(harness_dir);
    if !agent_summary.is_empty() {
        result.push_str(&format!("\n\n{agent_summary}"));
    }

    // Capture stderr on failure
    if !success
        && let Some(stderr) = child.stderr.take()
    {
        use std::io::Read;
        let mut buf = String::new();
        let mut reader = std::io::BufReader::new(stderr);
        let _ = reader.read_to_string(&mut buf);
        if !buf.is_empty() {
            let snippet = if buf.len() > 500 {
                &buf[buf.len() - 500..]
            } else {
                &buf
            };
            result.push_str(&format!("\n\nError output:\n```\n{snippet}\n```"));
        }
    }

    result
}

/// Collect rich progress info from the harness directory (file-based fallback).
///
/// Reads the progress.log (written by the runner in real time), per-agent
/// status files, and feedback rounds for a detailed progress message.
fn collect_rich_progress(
    workflow: &str,
    start: &std::time::Instant,
    harness_dir: &std::path::Path,
) -> String {
    let elapsed = start.elapsed().as_secs();
    let mut lines = vec![format!("Workflow '{workflow}' running... ({elapsed}s)")];

    // Read recent lines from progress.log (real-time agent output)
    let progress_path = harness_dir.join("progress.log");
    if let Ok(content) = fs::read_to_string(&progress_path) {
        let recent: Vec<&str> = content
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if !recent.is_empty() {
            lines.push(String::new()); // blank line separator
            for line in &recent {
                lines.push(truncate_line(line, 120));
            }
        }
    }

    // Check per-agent status files in .harness/agents/*/status.md
    let agents_dir = harness_dir.join("agents");
    if agents_dir.exists()
        && let Ok(entries) = fs::read_dir(&agents_dir)
    {
        let mut agent_lines = Vec::new();
        for entry in entries.flatten() {
            let agent_dir = entry.path();
            if !agent_dir.is_dir() {
                continue;
            }
            let agent_name = agent_dir
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let agent_status = agent_dir.join("status.md");
            if let Ok(content) = fs::read_to_string(&agent_status)
                && let Some(last) = content.lines().rev().find(|l| !l.trim().is_empty())
            {
                agent_lines.push(format!(
                    "  `{agent_name}`: {}",
                    truncate_line(last, 80)
                ));
            }
        }
        if !agent_lines.is_empty() {
            lines.push(String::new());
            lines.push("Agents:".to_string());
            lines.extend(agent_lines);
        }
    }

    lines.join("\n")
}

/// Summarize per-agent status files for the completion message.
fn collect_agent_summary(harness_dir: &std::path::Path) -> String {
    let agents_dir = harness_dir.join("agents");
    if !agents_dir.exists() {
        return String::new();
    }
    let Ok(entries) = fs::read_dir(&agents_dir) else {
        return String::new();
    };

    let mut lines = Vec::new();
    for entry in entries.flatten() {
        let agent_dir = entry.path();
        if !agent_dir.is_dir() {
            continue;
        }
        let agent_name = agent_dir
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let agent_status = agent_dir.join("status.md");
        if agent_status.exists() {
            let size = fs::metadata(&agent_status)
                .map(|m| m.len())
                .unwrap_or(0);
            if size > 0 {
                lines.push(format!("  `{agent_name}`: output ({size} bytes)"));
            }
        }
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("Agents:\n{}", lines.join("\n"))
    }
}

/// Truncate a line to max chars, appending "..." if truncated.
fn truncate_line(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max {
        trimmed.to_string()
    } else {
        let mut end = max;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &trimmed[..end])
    }
}

/// Extract verdict line from evaluation.md content.
fn extract_verdict(eval_content: &str) -> String {
    for line in eval_content.lines() {
        let lower = line.to_lowercase();
        if lower.contains("verdict:") || lower.contains("## verdict") {
            return line.trim().to_string();
        }
        // Look for standalone PASS/FAIL/REVISE
        let trimmed = line.trim().to_uppercase();
        if trimmed == "PASS" || trimmed == "FAIL" || trimmed == "REVISE" {
            return trimmed;
        }
    }
    String::new()
}

/// Find the first registered workspace that has a .harness/ directory.
fn find_active_workspace() -> Option<std::path::PathBuf> {
    let ws_dir = xdg::data_dir().join("workspaces");
    let entries = fs::read_dir(&ws_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "path")
            && let Ok(ws_path) = fs::read_to_string(&path)
        {
            let p = std::path::PathBuf::from(ws_path.trim());
            if p.join(".harness").exists() {
                return Some(p);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// /status command
// ---------------------------------------------------------------------------

/// /status — show workspaces, schedules, and workflows.
fn cmd_status() -> String {
    let mut lines = vec!["*Harness Status*".to_string()];

    // Workspaces
    let ws_dir = xdg::data_dir().join("workspaces");
    if ws_dir.exists()
        && let Ok(entries) = fs::read_dir(&ws_dir)
    {
        let workspaces: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "path"))
            .collect();
        if !workspaces.is_empty() {
            lines.push(format!("\nWorkspaces ({})", workspaces.len()));
            for ws in &workspaces {
                let name = ws
                    .path()
                    .file_stem()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                if let Ok(path) = fs::read_to_string(ws.path()) {
                    let p = path.trim();
                    let has_harness = std::path::Path::new(p).join(".harness").exists();
                    let tag = if has_harness { "active" } else { "no .harness/" };
                    lines.push(format!("  `{name}`: {p} [{tag}]"));
                }
            }
        } else {
            lines.push("\nNo workspaces registered.".to_string());
        }
    }

    // Schedules
    let schedules = crate::commands::schedule::load_schedules();
    if !schedules.is_empty() {
        lines.push(format!("\nSchedules ({})", schedules.len()));
        for (name, cron, cmd) in &schedules {
            lines.push(format!("  `{name}`: [{cron}] `{cmd}`"));
        }
    } else {
        lines.push("\nNo schedules.".to_string());
    }

    // Workflows
    let wf_list = list_workflows();
    if wf_list.is_empty() {
        lines.push("\nNo workflows defined.".to_string());
    } else {
        lines.push(format!("\n*Workflows*\n{wf_list}"));
    }

    // Daemon status
    let daemon_active = Command::new("systemctl")
        .args(["--user", "is-active", "harness"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
        .unwrap_or(false);
    let bridge_active = Command::new("systemctl")
        .args(["--user", "is-active", "harness-telegram"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
        .unwrap_or(false);

    lines.push(format!(
        "\nDaemon: {} | Bridge: {}",
        if daemon_active { "running" } else { "stopped" },
        if bridge_active { "running" } else { "stopped" },
    ));

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// /agent list
// ---------------------------------------------------------------------------

/// /agent list — list defined agents.
fn cmd_agent(args: &str) -> String {
    let sub = args.trim();
    if sub != "list" && !sub.is_empty() {
        return format!("Usage: /agent list\n\nUnknown subcommand: '{sub}'");
    }

    let agents_dir = xdg::config_dir().join("agents");
    if !agents_dir.exists() {
        return "No agents defined.\n\nDefine agents with:\n`harness agent add <name> --role <role> --backend <backend>`".to_string();
    }

    let entries = match fs::read_dir(&agents_dir) {
        Ok(e) => e,
        Err(_) => return "No agents defined.".to_string(),
    };

    let mut lines = vec!["*Agents*".to_string()];
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml")
            && let Ok(content) = fs::read_to_string(&path)
            && let Ok(parsed) = content.parse::<toml::Value>()
        {
            let name = parsed
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let role = parsed
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let backend = parsed
                .get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            lines.push(format!("  `{name}` -- {role} ({backend})"));
            count += 1;
        }
    }

    if count == 0 {
        return "No agents defined.".to_string();
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// /vault status
// ---------------------------------------------------------------------------

/// /vault status — show vault health.
fn cmd_vault(args: &str) -> String {
    let sub = args.trim();
    if sub != "status" && !sub.is_empty() {
        return format!("Usage: /vault status\n\nUnknown subcommand: '{sub}'");
    }

    let vc = vault::load_config();
    if !vc.enabled {
        return "Vault: *disabled*\n\nEnable with: `harness vault init`".to_string();
    }

    let healthy = vault::is_healthy(&vc);
    let mut lines = vec![
        "*Vault Status*".to_string(),
        format!("  Address: `{}`", vc.addr),
        format!("  Agent: `{}`", vc.agent_name),
        format!(
            "  Status: {}",
            if healthy { "connected" } else { "unreachable" }
        ),
    ];

    if healthy {
        match vault::list_credentials(&vc) {
            Ok(creds) => {
                lines.push(format!("  Credentials: {}", creds.len()));
                for (path, desc) in &creds {
                    let d = desc.as_deref().unwrap_or("");
                    if d.is_empty() {
                        lines.push(format!("    `{path}`"));
                    } else {
                        lines.push(format!("    `{path}` -- {d}"));
                    }
                }
            }
            Err(e) => lines.push(format!("  (list failed: {e})")),
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// /help
// ---------------------------------------------------------------------------

/// /help — list available commands.
fn cmd_help() -> String {
    "*Harness Telegram Bridge*\n\n\
     Available commands:\n\
     /run <workflow> -- Start a workflow (result sent on completion)\n\
     /run <workflow> --wait -- Start and stream progress updates\n\
     /status -- Show workspaces, schedules, workflows\n\
     /agent list -- List defined agents\n\
     /vault status -- Show vault health\n\
     /help -- Show this message\n\n\
     All commands require vault policy authorization."
        .to_string()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// List available workflows as formatted text.
fn list_workflows() -> String {
    let wf_dir = xdg::config_dir().join("workflows");
    if !wf_dir.exists() {
        return String::new();
    }
    let Ok(entries) = fs::read_dir(&wf_dir) else {
        return String::new();
    };

    let mut lines = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml")
            && let Ok(content) = fs::read_to_string(&path)
            && let Ok(parsed) = content.parse::<toml::Value>()
        {
            let name = parsed
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let desc = parsed
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if desc.is_empty() {
                lines.push(format!("  `{name}`"));
            } else {
                lines.push(format!("  `{name}` -- {desc}"));
            }
        }
    }
    lines.join("\n")
}

/// Path to the file tracking the last update_id.
fn offset_file() -> std::path::PathBuf {
    xdg::data_dir().join("telegram-offset")
}

/// Load the last update offset (so we don't re-process old messages).
fn load_offset() -> i64 {
    fs::read_to_string(offset_file())
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Save the update offset.
fn save_offset(offset: i64) {
    let _ = fs::write(offset_file(), offset.to_string());
}

// ---------------------------------------------------------------------------
// Main listener loop
// ---------------------------------------------------------------------------

/// Run the Telegram bot listener loop. Blocks forever (meant for systemd).
pub fn run_listener() -> Result<(), String> {
    let creds = resolve_credentials()?;

    // Validate credentials with a getMe call
    let me_url = format!(
        "https://api.telegram.org/bot{}/getMe",
        creds.bot_token
    );
    let output = Command::new("curl")
        .args(["-s", "--max-time", "10", &me_url])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;
    let me_body = String::from_utf8_lossy(&output.stdout);
    let me: Value = serde_json::from_str(&me_body)
        .map_err(|_| "Failed to validate bot token -- invalid response from Telegram API".to_string())?;
    if me.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err("Invalid bot token -- Telegram API rejected it.".to_string());
    }
    let bot_name = me
        .pointer("/result/username")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    eprintln!("[telegram] Bot @{bot_name} authenticated");
    eprintln!("[telegram] Listening for commands in chat {}", creds.chat_id);

    // Record bridge start to SCL
    scl_lifecycle::record_bridge_event(
        "telegram",
        "start",
        &format!("Bot @{bot_name} listening"),
    );

    // Wrap creds in Arc for sharing with callback threads
    let creds = Arc::new(creds);
    let mut offset = load_offset();

    // Track active workflow threads for cleanup
    let active_threads: Arc<Mutex<Vec<thread::JoinHandle<()>>>> =
        Arc::new(Mutex::new(Vec::new()));

    loop {
        // Clean up finished threads
        if let Ok(mut threads) = active_threads.lock() {
            threads.retain(|t| !t.is_finished());
        }

        match get_updates(&creds.bot_token, offset) {
            Ok(updates) => {
                for update in &updates {
                    // Advance offset past this update
                    if let Some(uid) = update.get("update_id").and_then(|v| v.as_i64()) {
                        offset = uid + 1;
                        save_offset(offset);
                    }

                    // Extract message text and chat_id
                    let msg = update
                        .get("message")
                        .or_else(|| update.get("edited_message"));
                    let Some(msg) = msg else { continue };

                    let msg_chat_id = msg
                        .pointer("/chat/id")
                        .and_then(|v| v.as_i64())
                        .map(|v| v.to_string())
                        .unwrap_or_default();

                    // Only respond to messages from the configured chat
                    if msg_chat_id != creds.chat_id {
                        continue;
                    }

                    let Some(text) = msg.get("text").and_then(|v| v.as_str()) else {
                        continue;
                    };

                    let Some((cmd, args)) = parse_command(text) else {
                        continue;
                    };

                    let user = msg
                        .pointer("/from/username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");

                    eprintln!("[telegram] @{user}: {cmd} {args}");

                    // Record command to SCL
                    scl_lifecycle::record_bridge_command("telegram", user, cmd, args);

                    // Execute command
                    let response = handle_command(cmd, args, &creds);

                    // Record response to SCL
                    scl_lifecycle::record_bridge_response("telegram", cmd, &response);

                    // Send response back to Telegram
                    if let Err(e) = send_message(&creds, &response) {
                        eprintln!("[telegram] Failed to send response: {e}");
                    }
                }
            }
            Err(e) => {
                eprintln!("[telegram] Poll error: {e}");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public helpers for bridge_cmd
// ---------------------------------------------------------------------------

/// Check if the bridge service is running.
pub fn is_running() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "harness-telegram"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
        .unwrap_or(false)
}

/// Verify credentials are valid without starting the listener.
pub fn check_credentials() -> Result<String, String> {
    let creds = resolve_credentials()?;
    let me_url = format!(
        "https://api.telegram.org/bot{}/getMe",
        creds.bot_token
    );
    let output = Command::new("curl")
        .args(["-s", "--max-time", "10", &me_url])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;
    let me_body = String::from_utf8_lossy(&output.stdout);
    let me: Value = serde_json::from_str(&me_body)
        .map_err(|_| "Failed to parse Telegram API response".to_string())?;
    if me.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err("Invalid bot token".to_string());
    }
    let bot_name = me
        .pointer("/result/username")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    Ok(format!("@{bot_name}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_basic() {
        assert_eq!(
            parse_command("/run my-workflow"),
            Some(("/run", "my-workflow"))
        );
        assert_eq!(parse_command("/status"), Some(("/status", "")));
        assert_eq!(parse_command("/agent list"), Some(("/agent", "list")));
        assert_eq!(parse_command("/vault status"), Some(("/vault", "status")));
    }

    #[test]
    fn test_parse_command_with_botname() {
        assert_eq!(parse_command("/status@mybot"), Some(("/status", "")));
        assert_eq!(
            parse_command("/run@mybot my-workflow"),
            Some(("/run", "my-workflow"))
        );
    }

    #[test]
    fn test_parse_command_not_command() {
        assert_eq!(parse_command("hello"), None);
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("  no slash"), None);
    }

    #[test]
    fn test_cmd_help_contains_commands() {
        let help = cmd_help();
        assert!(help.contains("/run"));
        assert!(help.contains("/status"));
        assert!(help.contains("/agent"));
        assert!(help.contains("/vault"));
        assert!(help.contains("/help"));
        assert!(help.contains("--wait"));
    }

    #[test]
    fn test_escape_markdown_basic() {
        assert_eq!(escape_markdown("hello"), "hello");
        assert_eq!(escape_markdown("hello_world"), "hello\\_world");
        assert_eq!(escape_markdown("*bold*"), "\\*bold\\*");
        assert_eq!(escape_markdown("[link]"), "\\[link]");
    }

    #[test]
    fn test_escape_markdown_preserves_code_spans() {
        assert_eq!(escape_markdown("`code_here`"), "`code_here`");
        assert_eq!(
            escape_markdown("text `code_span` more_text"),
            "text `code_span` more\\_text"
        );
    }

    #[test]
    fn test_escape_markdown_no_double_escape() {
        // Already-escaped content shouldn't double-escape
        assert_eq!(escape_markdown("a\\b"), "a\\b");
    }

    #[test]
    fn test_parse_run_args() {
        assert_eq!(parse_run_args("my-workflow"), ("my-workflow".to_string(), false));
        assert_eq!(
            parse_run_args("my-workflow --wait"),
            ("my-workflow".to_string(), true)
        );
        assert_eq!(
            parse_run_args("--wait my-workflow"),
            ("my-workflow".to_string(), true)
        );
        assert_eq!(parse_run_args(""), (String::new(), false));
    }

    #[test]
    fn test_extract_verdict() {
        assert_eq!(extract_verdict("Verdict: PASS"), "Verdict: PASS");
        assert_eq!(extract_verdict("## Verdict\nPASS"), "## Verdict");
        assert_eq!(extract_verdict("Some text\nPASS\nMore"), "PASS");
        assert_eq!(extract_verdict("no verdict here"), "");
    }

    #[test]
    fn test_policy_for_command() {
        assert_eq!(policy_for_command("/run"), "harness:bridge:telegram:run");
        assert_eq!(
            policy_for_command("/status"),
            "harness:bridge:telegram:status"
        );
        assert_eq!(
            policy_for_command("/unknown"),
            "harness:bridge:telegram:other"
        );
    }

    #[test]
    fn test_truncate_line() {
        assert_eq!(truncate_line("hello", 10), "hello");
        assert_eq!(truncate_line("  hello  ", 10), "hello");
        assert_eq!(truncate_line("hello world this is long", 10), "hello worl...");
    }

    #[test]
    fn test_collect_rich_progress_empty_dir() {
        let tmp = std::env::temp_dir().join(format!("harness-test-progress-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let start = std::time::Instant::now();
        let progress = collect_rich_progress("test-wf", &start, &tmp);
        assert!(progress.contains("test-wf"));
        assert!(progress.contains("running"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_collect_rich_progress_with_progress_log() {
        let tmp = std::env::temp_dir().join(format!("harness-test-progress2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(
            tmp.join("progress.log"),
            "[12:00:01] Step 1/3: agent 'planner' started\n[12:00:30] Planner 'my-planner' done\n[12:01:00] Step 2/3: agent 'builder' started\n",
        ).unwrap();
        let start = std::time::Instant::now();
        let progress = collect_rich_progress("test-wf", &start, &tmp);
        assert!(progress.contains("builder"), "should contain builder line: {progress}");
        assert!(progress.contains("planner"), "should contain planner line: {progress}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_collect_agent_summary_with_agents() {
        let tmp = std::env::temp_dir().join(format!("harness-test-agents-{}", std::process::id()));
        let agent_dir = tmp.join("agents/my-builder");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("status.md"), "some output here").unwrap();
        let summary = collect_agent_summary(&tmp);
        assert!(summary.contains("my-builder"));
        assert!(summary.contains("output"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_collect_agent_summary_empty() {
        let tmp = std::env::temp_dir().join(format!("harness-test-noagents-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let summary = collect_agent_summary(&tmp);
        assert!(summary.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

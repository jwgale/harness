//! Telegram bot bridge for controlling Harness via chat.
//!
//! Uses the Telegram Bot API via direct HTTP (long polling with curl).
//! Credentials are resolved from SanctumAI vault first, then config fallback.

use std::fs;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::scl_lifecycle;
use crate::vault;
use crate::xdg;

const POLL_TIMEOUT_SECS: u64 = 30;

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
        return Err("Bot token is empty. Run: harness vault add notifications/telegram/bot-token".to_string());
    }
    if chat_id.is_empty() {
        return Err("Chat ID is empty. Run: harness vault add notifications/telegram/chat-id".to_string());
    }

    Ok(BotCredentials { bot_token, chat_id })
}

/// Send a text message to the configured Telegram chat.
fn send_message(creds: &BotCredentials, text: &str) -> Result<(), String> {
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        creds.bot_token
    );
    // Truncate to Telegram's 4096-char limit
    let text = if text.len() > 4000 {
        format!("{}…\n(truncated)", &text[..4000])
    } else {
        text.to_string()
    };
    let payload = serde_json::json!({
        "chat_id": creds.chat_id,
        "text": text,
        "parse_mode": "Markdown"
    });
    let body = payload.to_string();
    let output = Command::new("curl")
        .args([
            "-s", "-o", "/dev/null", "-w", "%{http_code}",
            "-X", "POST",
            "-H", "Content-Type: application/json",
            "--max-time", "10",
            "-d", &body,
            &url,
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    let code = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if code.starts_with('2') {
        Ok(())
    } else {
        Err(format!("Telegram API returned HTTP {code}"))
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
    let parsed: Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse Telegram response: {e}"))?;

    if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(format!(
            "Telegram API error: {}",
            parsed.get("description").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }

    Ok(parsed
        .get("result")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default())
}

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
fn handle_command(cmd: &str, args: &str) -> String {
    match cmd {
        "/run" => cmd_run(args),
        "/status" => cmd_status(),
        "/agent" => cmd_agent(args),
        "/vault" => cmd_vault(args),
        "/help" => cmd_help(),
        "/start" => cmd_help(), // Telegram sends /start on first interaction
        _ => format!("Unknown command: {cmd}\nType /help for available commands."),
    }
}

/// /run <workflow-name> — trigger a workflow.
fn cmd_run(args: &str) -> String {
    let workflow = args.trim();
    if workflow.is_empty() {
        return "Usage: /run <workflow-name>\n\nRuns a named workflow. List workflows with /status.".to_string();
    }

    // Check if workflow exists
    let wf_dir = xdg::config_dir().join("workflows");
    let wf_path = wf_dir.join(format!("{workflow}.toml"));
    if !wf_path.exists() {
        return format!("Workflow '{workflow}' not found.\n\nAvailable workflows:\n{}", list_workflows());
    }

    // Launch workflow in background via harness CLI
    let binary = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => return format!("Failed to find harness binary: {e}"),
    };

    match Command::new(&binary)
        .args(["run", "--workflow", workflow, "--no-tui"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => {
            format!("Workflow '{workflow}' started (PID {})", child.id())
        }
        Err(e) => format!("Failed to start workflow: {e}"),
    }
}

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
            lines.push(format!("\n📁 *Workspaces* ({})", workspaces.len()));
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
            lines.push("\n📁 No workspaces registered.".to_string());
        }
    }

    // Schedules
    let schedules = crate::commands::schedule::load_schedules();
    if !schedules.is_empty() {
        lines.push(format!("\n⏰ *Schedules* ({})", schedules.len()));
        for (name, cron, cmd) in &schedules {
            lines.push(format!("  `{name}`: [{cron}] `{cmd}`"));
        }
    } else {
        lines.push("\n⏰ No schedules.".to_string());
    }

    // Workflows
    let wf_list = list_workflows();
    if wf_list.is_empty() {
        lines.push("\n📋 No workflows defined.".to_string());
    } else {
        lines.push(format!("\n📋 *Workflows*\n{wf_list}"));
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
        "\n🔧 Daemon: {} | Bridge: {}",
        if daemon_active { "running" } else { "stopped" },
        if bridge_active { "running" } else { "stopped" },
    ));

    lines.join("\n")
}

/// /agent list — list defined agents.
fn cmd_agent(args: &str) -> String {
    let sub = args.trim();
    if sub != "list" && !sub.is_empty() {
        return format!("Usage: /agent list\n\nUnknown subcommand: '{sub}'");
    }

    let agents_dir = xdg::config_dir().join("agents");
    if !agents_dir.exists() {
        return "No agents defined.\n\nDefine agents with: `harness agent add <name> --role <role> --backend <backend>`".to_string();
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
            lines.push(format!("  `{name}` — {role} ({backend})"));
            count += 1;
        }
    }

    if count == 0 {
        return "No agents defined.".to_string();
    }
    lines.join("\n")
}

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
            if healthy { "connected ✅" } else { "unreachable ❌" }
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
                        lines.push(format!("    `{path}` — {d}"));
                    }
                }
            }
            Err(e) => lines.push(format!("  (list failed: {e})")),
        }
    }

    lines.join("\n")
}

/// /help — list available commands.
fn cmd_help() -> String {
    "*Harness Telegram Bridge*\n\n\
     Available commands:\n\
     /run <workflow> — Start a workflow\n\
     /status — Show workspaces, schedules, workflows\n\
     /agent list — List defined agents\n\
     /vault status — Show vault health\n\
     /help — Show this message"
        .to_string()
}

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
                lines.push(format!("  `{name}` — {desc}"));
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
        .map_err(|_| "Failed to validate bot token — invalid response from Telegram API".to_string())?;
    if me.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err("Invalid bot token — Telegram API rejected it.".to_string());
    }
    let bot_name = me
        .pointer("/result/username")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    eprintln!("[telegram] Bot @{bot_name} authenticated");
    eprintln!("[telegram] Listening for commands in chat {}", creds.chat_id);

    // Record bridge start to SCL
    scl_lifecycle::record_bridge_event("telegram", "start", &format!("Bot @{bot_name} listening"));

    let mut offset = load_offset();

    loop {
        match get_updates(&creds.bot_token, offset) {
            Ok(updates) => {
                for update in &updates {
                    // Advance offset past this update
                    if let Some(uid) = update.get("update_id").and_then(|v| v.as_i64()) {
                        offset = uid + 1;
                        save_offset(offset);
                    }

                    // Extract message text and chat_id
                    let msg = update.get("message").or_else(|| update.get("edited_message"));
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
                    let response = handle_command(cmd, args);

                    // Record response to SCL
                    scl_lifecycle::record_bridge_response("telegram", cmd, &response);

                    // Send response back to Telegram
                    if let Err(e) = send_message(&creds, &response) {
                        eprintln!("[telegram] Failed to send response: {e}");
                        // Try again without markdown in case of parse errors
                        let plain_payload = serde_json::json!({
                            "chat_id": creds.chat_id,
                            "text": response,
                        });
                        let url = format!(
                            "https://api.telegram.org/bot{}/sendMessage",
                            creds.bot_token
                        );
                        let body = plain_payload.to_string();
                        let _ = Command::new("curl")
                            .args([
                                "-s", "-o", "/dev/null",
                                "-X", "POST",
                                "-H", "Content-Type: application/json",
                                "--max-time", "10",
                                "-d", &body,
                                &url,
                            ])
                            .output();
                    }
                }
            }
            Err(e) => {
                eprintln!("[telegram] Poll error: {e}");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_basic() {
        assert_eq!(parse_command("/run my-workflow"), Some(("/run", "my-workflow")));
        assert_eq!(parse_command("/status"), Some(("/status", "")));
        assert_eq!(parse_command("/agent list"), Some(("/agent", "list")));
        assert_eq!(parse_command("/vault status"), Some(("/vault", "status")));
    }

    #[test]
    fn test_parse_command_with_botname() {
        assert_eq!(parse_command("/status@mybot"), Some(("/status", "")));
        assert_eq!(parse_command("/run@mybot my-workflow"), Some(("/run", "my-workflow")));
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
    }
}

//! External notification system.
//!
//! Notification plugins are TOML files in ~/.config/harness/plugins/ with a [notifications]
//! section. They fire on evaluator and schedule lifecycle events.

use serde::Deserialize;
use std::fs;
use std::process::Command;

use crate::commands::evaluate::Verdict;
use crate::scl_lifecycle;
use crate::vault;
use crate::xdg;

/// Notification events that can trigger a notification.
#[derive(Debug, Clone, Copy)]
pub enum NotifyEvent {
    EvalPass,
    EvalFail,
    EvalRevise,
    ScheduleComplete,
}

impl NotifyEvent {
    pub fn label(self) -> &'static str {
        match self {
            NotifyEvent::EvalPass => "on_eval_pass",
            NotifyEvent::EvalFail => "on_eval_fail",
            NotifyEvent::EvalRevise => "on_eval_revise",
            NotifyEvent::ScheduleComplete => "on_schedule_complete",
        }
    }
}

/// A notification plugin manifest with a [notifications] section.
#[derive(Debug, Deserialize)]
pub struct NotificationPlugin {
    pub name: String,
    #[allow(dead_code)]
    pub description: Option<String>,
    pub notifications: Option<NotificationConfig>,
}

#[derive(Debug, Deserialize)]
pub struct NotificationConfig {
    /// Strategy: slack, telegram, email, webhook
    pub strategy: String,
    /// Webhook URL (for slack, telegram, webhook strategies)
    pub url: Option<String>,
    /// Email recipient (for email strategy)
    pub to: Option<String>,
    /// Email from address (for email strategy)
    pub from: Option<String>,
    /// Telegram bot token (for telegram strategy)
    pub bot_token: Option<String>,
    /// Telegram chat ID (for telegram strategy)
    pub chat_id: Option<String>,
    /// Which events to fire on (default: all)
    pub events: Option<Vec<String>>,
}

impl NotificationConfig {
    /// Check if this config wants to fire on the given event.
    fn wants_event(&self, event: NotifyEvent) -> bool {
        match &self.events {
            None => true, // fire on all events by default
            Some(events) => events.iter().any(|e| e == event.label()),
        }
    }
}

/// Fire notification events for an eval verdict.
pub fn fire_eval_event(verdict: &Verdict, project: &str, round: u32) {
    let event = match verdict {
        Verdict::Pass => NotifyEvent::EvalPass,
        Verdict::Fail => NotifyEvent::EvalFail,
        Verdict::Revise => NotifyEvent::EvalRevise,
    };
    let message = format!("Harness [{project}] round {round}: {verdict:?}");
    fire(event, &message);
}

/// Fire notification for schedule completion.
pub fn fire_schedule_complete(schedule_name: &str, success: bool) {
    let status = if success { "succeeded" } else { "failed" };
    let message = format!("Harness schedule '{schedule_name}' {status}");
    fire(NotifyEvent::ScheduleComplete, &message);
}

/// Discover notification plugins and fire matching events.
fn fire(event: NotifyEvent, message: &str) {
    let plugins = discover_notification_plugins();
    for plugin in &plugins {
        let Some(notif) = &plugin.notifications else {
            continue;
        };
        if !notif.wants_event(event) {
            continue;
        }
        let result = send(notif, message);
        match &result {
            Ok(_) => {
                eprintln!(
                    "[notify:{}] {} -> sent via {}",
                    plugin.name,
                    event.label(),
                    notif.strategy
                );
            }
            Err(e) => {
                eprintln!("[notify:{}] {} -> FAILED: {e}", plugin.name, event.label());
            }
        }
        // Record to SCL
        scl_lifecycle::record_notification(
            &plugin.name,
            &notif.strategy,
            event.label(),
            result.is_ok(),
        );
    }
}

/// Send a notification via the configured strategy.
fn send(config: &NotificationConfig, message: &str) -> Result<(), String> {
    match config.strategy.as_str() {
        "slack" => send_slack(config, message),
        "telegram" => send_telegram(config, message),
        "email" => send_email(config, message),
        "webhook" => send_webhook(config, message),
        other => Err(format!("Unknown notification strategy: '{other}'")),
    }
}

/// Resolve a credential: try vault first (path = "notifications/<key>"), then fall back to config value.
fn resolve_credential(config_value: Option<&str>, vault_path: &str) -> Option<String> {
    // Try vault first
    let vc = vault::load_config();
    if vc.enabled
        && let Ok(val) = vault::get_credential_string(&vc, vault_path)
    {
        return Some(val);
    }
    // Fall back to plaintext config
    config_value.map(|s| s.to_string())
}

/// Send via Slack incoming webhook.
fn send_slack(config: &NotificationConfig, message: &str) -> Result<(), String> {
    let url = resolve_credential(config.url.as_deref(), "notifications/slack/webhook-url")
        .ok_or_else(|| "Slack notification requires 'url' or vault credential 'notifications/slack/webhook-url'".to_string())?;
    let payload = serde_json::json!({ "text": message });
    curl_post_json(&url, &payload)
}

/// Send via Telegram Bot API.
fn send_telegram(config: &NotificationConfig, message: &str) -> Result<(), String> {
    let token = resolve_credential(
        config.bot_token.as_deref(),
        "notifications/telegram/bot-token",
    )
    .ok_or_else(|| {
        "Telegram requires 'bot_token' or vault credential 'notifications/telegram/bot-token'"
            .to_string()
    })?;
    let chat_id = resolve_credential(config.chat_id.as_deref(), "notifications/telegram/chat-id")
        .ok_or_else(|| {
        "Telegram requires 'chat_id' or vault credential 'notifications/telegram/chat-id'"
            .to_string()
    })?;
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "text": message,
        "parse_mode": "Markdown"
    });
    curl_post_json(&url, &payload)
}

/// Send via email using the local `sendmail` or `mail` command.
fn send_email(config: &NotificationConfig, message: &str) -> Result<(), String> {
    let to = resolve_credential(config.to.as_deref(), "notifications/email/to")
        .ok_or_else(|| "Email notification requires 'to' address".to_string())?;
    let from = resolve_credential(config.from.as_deref(), "notifications/email/from")
        .unwrap_or_else(|| "harness@localhost".to_string());

    let output = Command::new("mail")
        .args(["-s", "Harness Notification", "-r", &from, &to])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(message.as_bytes());
            }
            child.wait_with_output()
        })
        .map_err(|e| format!("Failed to send email: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Email send failed: {stderr}"))
    }
}

/// Send via generic webhook (POST JSON body with message field).
fn send_webhook(config: &NotificationConfig, message: &str) -> Result<(), String> {
    let url = resolve_credential(config.url.as_deref(), "notifications/webhook/url").ok_or_else(
        || "Webhook requires 'url' or vault credential 'notifications/webhook/url'".to_string(),
    )?;
    let payload = serde_json::json!({
        "event": "harness_notification",
        "message": message,
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    curl_post_json(&url, &payload)
}

/// POST JSON to a URL using curl.
fn curl_post_json(url: &str, payload: &serde_json::Value) -> Result<(), String> {
    let body = payload.to_string();
    let output = Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "--max-time",
            "10",
            "-d",
            &body,
            url,
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    let code = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if code.starts_with('2') {
        Ok(())
    } else {
        Err(format!("HTTP {code}"))
    }
}

/// Discover plugins that have a [notifications] section.
fn discover_notification_plugins() -> Vec<NotificationPlugin> {
    let dir = xdg::plugins_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut plugins = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml")
            && let Ok(content) = fs::read_to_string(&path)
            && let Ok(plugin) = toml::from_str::<NotificationPlugin>(&content)
            && plugin.notifications.is_some()
        {
            plugins.push(plugin);
        }
    }
    plugins
}

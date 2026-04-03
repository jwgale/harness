//! Direct MCP client for the Shared Context Layer.
//! Uses HTTP JSON-RPC with session management — no Claude Code session needed.

use std::fs;
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

use crate::xdg;

static HEALTH_CACHE: Mutex<Option<(bool, Instant)>> = Mutex::new(None);
const HEALTH_CACHE_TTL_SECS: u64 = 60;

/// Check SCL health with a 60-second cache.
pub fn is_healthy(url: &str) -> bool {
    let mut cache = HEALTH_CACHE.lock().unwrap();
    if let Some((result, when)) = cache.as_ref()
        && when.elapsed().as_secs() < HEALTH_CACHE_TTL_SECS
    {
        return *result;
    }

    let health_url = url.trim_end_matches("/mcp").to_string() + "/health";
    let healthy = Command::new("curl")
        .args(["-fsSL", "--max-time", "2", &health_url])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    *cache = Some((healthy, Instant::now()));
    healthy
}

/// Perform a full MCP handshake and tool call. Returns the text content from the response.
pub fn call_tool(url: &str, tool: &str, args: &serde_json::Value) -> Result<String, String> {
    // Step 1: Initialize and get session ID
    let init_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "harness", "version": env!("CARGO_PKG_VERSION") }
        }
    });

    let init_output = curl_post_with_headers(url, &init_body)?;
    let session_id = init_output.session_id
        .ok_or_else(|| "No Mcp-Session-Id in initialize response".to_string())?;

    // Step 2: Send initialized notification
    let notif_body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let _ = curl_post(url, &notif_body, &session_id);

    // Step 3: Call the tool
    let call_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tool,
            "arguments": args
        }
    });

    let resp = curl_post(url, &call_body, &session_id)?;

    // Parse response
    let val: serde_json::Value = serde_json::from_str(&resp)
        .map_err(|e| format!("Failed to parse MCP response: {e}"))?;

    if let Some(err) = val.get("error") {
        return Err(format!("MCP error: {}", err));
    }

    // Extract text from result.content[].text
    if let Some(content) = val.pointer("/result/content")
        && let Some(arr) = content.as_array()
    {
        let texts: Vec<&str> = arr.iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect();
        if !texts.is_empty() {
            return Ok(texts.join("\n"));
        }
    }

    // Fallback: return the whole result
    if let Some(result) = val.get("result") {
        return Ok(serde_json::to_string_pretty(result).unwrap_or(resp));
    }

    Ok(resp)
}

/// Query the SCL.
pub fn query(url: &str, query_text: &str) -> Result<String, String> {
    call_tool(url, "context_query", &serde_json::json!({ "query": query_text }))
}

/// Record an entry to the SCL.
/// `kind` should be one of: architecture, decision, convention, active_work, insight, gotcha
pub fn record(url: &str, kind: &str, content: &str) -> Result<String, String> {
    call_tool(url, "context_record", &serde_json::json!({
        "kind": kind,
        "content": content,
        "author": "harness",
        "source": "agent_session"
    }))
}

struct CurlResponse {
    #[allow(dead_code)]
    body: String,
    session_id: Option<String>,
}

fn curl_post_with_headers(url: &str, body: &serde_json::Value) -> Result<CurlResponse, String> {
    let body_str = body.to_string();
    let output = Command::new("curl")
        .args(["-si", "-X", "POST",
               "-H", "Content-Type: application/json",
               "-H", "Accept: application/json, text/event-stream",
               "-d", &body_str,
               url])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    let full = String::from_utf8_lossy(&output.stdout).to_string();

    // Split headers from body (blank line separator)
    let (headers, body) = full.split_once("\r\n\r\n")
        .or_else(|| full.split_once("\n\n"))
        .unwrap_or(("", &full));

    let session_id = headers.lines()
        .find(|l| l.to_lowercase().starts_with("mcp-session-id:"))
        .map(|l| l.split_once(':').unwrap().1.trim().to_string());

    Ok(CurlResponse {
        body: body.to_string(),
        session_id,
    })
}

fn curl_post(url: &str, body: &serde_json::Value, session_id: &str) -> Result<String, String> {
    let body_str = body.to_string();
    let session_header = format!("Mcp-Session-Id: {session_id}");
    let output = Command::new("curl")
        .args(["-s", "-X", "POST",
               "-H", "Content-Type: application/json",
               "-H", "Accept: application/json, text/event-stream",
               "-H", &session_header,
               "-d", &body_str,
               url])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Generate a temporary MCP config JSON for Claude Code.
pub fn generate_mcp_config(scl_url: &str) -> Result<std::path::PathBuf, String> {
    let config = serde_json::json!({
        "mcpServers": {
            "shared-context-layer": {
                "url": scl_url
            }
        }
    });

    let mcp_dir = xdg::cache_dir().join("mcp");
    fs::create_dir_all(&mcp_dir)
        .map_err(|e| format!("Failed to create MCP cache dir: {e}"))?;

    let path = mcp_dir.join("scl-config.json");
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize MCP config: {e}"))?;
    fs::write(&path, json)
        .map_err(|e| format!("Failed to write MCP config: {e}"))?;

    Ok(path)
}

/// Record a harness lifecycle event to SCL (fire-and-forget, errors logged to stderr).
pub fn auto_record(url: &str, kind: &str, content: &str) {
    match record(url, kind, content) {
        Ok(_) => eprintln!("[scl] Recorded: [{kind}] {}", truncate(content, 80)),
        Err(e) => eprintln!("[scl] Auto-record failed: {e}"),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

/// Path to the last-recorded event file (for status display).
pub fn last_event_path() -> std::path::PathBuf {
    xdg::cache_dir().join("scl-last-event.txt")
}

/// Save the last recorded event description.
pub fn save_last_event(desc: &str) {
    let _ = fs::write(last_event_path(), desc);
}

/// Load the last recorded event description.
pub fn load_last_event() -> Option<String> {
    fs::read_to_string(last_event_path()).ok().filter(|s| !s.is_empty())
}

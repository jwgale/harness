use std::process::Command;

use crate::global_config::{self, GlobalConfig};

pub fn status() -> Result<(), String> {
    let gc = GlobalConfig::load();

    match gc.scl() {
        None => {
            println!("Shared Context Layer: disabled");
            println!();
            println!("Enable in ~/.config/harness/config.toml:");
            println!("  [shared_context]");
            println!("  enabled = true");
            println!("  url = \"http://127.0.0.1:3100/mcp\"");
        }
        Some(scl) => {
            let url = scl.url();
            let healthy = global_config::check_scl_health(url);
            let status = if healthy { "connected" } else { "unreachable" };
            println!("Shared Context Layer: {status}");
            println!("  URL: {url}");
            if !healthy {
                println!();
                println!("The SCL server is not responding. Check that it's running:");
                let health_url = url.trim_end_matches("/mcp").to_string() + "/health";
                println!("  curl {health_url}");
            } else {
                println!();
                println!("All Claude Code sessions will automatically have SCL access.");
                println!("Available MCP tools: context_init, context_query, context_record, context_update");
            }
        }
    }
    Ok(())
}

pub fn query(query_text: &str) -> Result<(), String> {
    let gc = GlobalConfig::load();
    let scl = gc.scl()
        .ok_or_else(|| "Shared Context Layer is not enabled. Edit ~/.config/harness/config.toml".to_string())?;

    let url = scl.url();
    if !global_config::check_scl_health(url) {
        return Err(format!("SCL server is not reachable at {url}"));
    }

    // SCL uses SSE-based MCP transport — direct queries go through Claude Code sessions.
    // For CLI convenience, we use a one-shot claude session with the SCL MCP attached.
    println!("Querying SCL for: {query_text}");
    println!();

    let prompt = format!(
        "Use the context_query tool from the shared-context-layer MCP server to find information about: {query_text}\n\nPrint the results clearly."
    );

    let mcp_path = global_config::generate_mcp_config(url)?;
    let output = Command::new("claude")
        .args(["--print", "--permission-mode", "bypassPermissions",
               "--mcp-config", &mcp_path.to_string_lossy(),
               "-p", &prompt])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to run claude for SCL query: {e}"))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("{stdout}");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("SCL query failed: {stderr}"));
    }
    Ok(())
}

pub fn record(entry_type: &str, content: &str) -> Result<(), String> {
    let gc = GlobalConfig::load();
    let scl = gc.scl()
        .ok_or_else(|| "Shared Context Layer is not enabled. Edit ~/.config/harness/config.toml".to_string())?;

    let url = scl.url();
    if !global_config::check_scl_health(url) {
        return Err(format!("SCL server is not reachable at {url}"));
    }

    let project = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    let prompt = format!(
        "Use the context_record tool from the shared-context-layer MCP server to record this:\n\
         entry_type: {entry_type}\n\
         content: {content}\n\
         source: harness:{project}\n\n\
         Confirm the recording was successful."
    );

    let mcp_path = global_config::generate_mcp_config(url)?;
    let output = Command::new("claude")
        .args(["--print", "--permission-mode", "bypassPermissions",
               "--mcp-config", &mcp_path.to_string_lossy(),
               "-p", &prompt])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to run claude for SCL record: {e}"))?;

    if output.status.success() {
        println!("Recorded to SCL: [{entry_type}] {content}");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("SCL record failed: {stderr}"));
    }
    Ok(())
}

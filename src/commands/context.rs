use crate::global_config::GlobalConfig;
use crate::scl;

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
        Some(scl_cfg) => {
            let url = scl_cfg.url();
            let healthy = scl::is_healthy(url);
            let conn_status = if healthy { "connected" } else { "unreachable" };
            let auto = if scl_cfg.auto_record() { "on" } else { "off" };

            println!("Shared Context Layer: {conn_status}");
            println!("  URL:         {url}");
            println!("  Auto-record: {auto}");

            if let Some(last) = scl::load_last_event() {
                println!("  Last event:  {last}");
            }

            if !healthy {
                println!();
                println!("The SCL server is not responding. Check that it's running:");
                let health_url = url.trim_end_matches("/mcp").to_string() + "/health";
                println!("  curl {health_url}");
            } else {
                println!();
                println!("All Claude Code sessions will automatically have SCL access.");
                println!(
                    "Available tools: context_init, context_query, context_record, context_update"
                );
            }
        }
    }
    Ok(())
}

pub fn query(query_text: &str) -> Result<(), String> {
    let gc = GlobalConfig::load();
    let scl_cfg = gc.scl().ok_or_else(|| {
        "Shared Context Layer is not enabled. Edit ~/.config/harness/config.toml".to_string()
    })?;

    let url = scl_cfg.url();
    if !scl::is_healthy(url) {
        return Err(format!("SCL server is not reachable at {url}"));
    }

    let result = scl::query(url, query_text)?;
    println!("{result}");
    Ok(())
}

pub fn record(kind: &str, content: &str) -> Result<(), String> {
    let gc = GlobalConfig::load();
    let scl_cfg = gc.scl().ok_or_else(|| {
        "Shared Context Layer is not enabled. Edit ~/.config/harness/config.toml".to_string()
    })?;

    let url = scl_cfg.url();
    if !scl::is_healthy(url) {
        return Err(format!("SCL server is not reachable at {url}"));
    }

    let result = scl::record(url, kind, content)?;
    println!("Recorded to SCL: [{kind}] {content}");
    if !result.is_empty() {
        eprintln!("[scl] {result}");
    }
    Ok(())
}

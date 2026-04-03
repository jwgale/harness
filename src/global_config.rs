use serde::Deserialize;
use std::fs;

use crate::xdg;

#[derive(Debug, Deserialize, Default)]
pub struct GlobalConfig {
    pub shared_context: Option<SharedContextConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SharedContextConfig {
    pub enabled: Option<bool>,
    pub url: Option<String>,
}

impl SharedContextConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn url(&self) -> &str {
        self.url.as_deref().unwrap_or("http://127.0.0.1:3100/mcp")
    }
}

impl GlobalConfig {
    pub fn load() -> Self {
        let path = xdg::config_dir().join("config.toml");
        if !path.exists() {
            return Self::default();
        }
        fs::read_to_string(&path)
            .ok()
            .and_then(|c| toml::from_str(&c).ok())
            .unwrap_or_default()
    }

    pub fn scl(&self) -> Option<&SharedContextConfig> {
        self.shared_context.as_ref().filter(|s| s.is_enabled())
    }
}

/// Check if the SCL server is reachable.
pub fn check_scl_health(url: &str) -> bool {
    // Derive health URL from MCP URL: http://host:port/mcp -> http://host:port/health
    let health_url = url.trim_end_matches("/mcp").to_string() + "/health";

    std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "2", &health_url])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Generate a temporary MCP config JSON for Claude Code that includes the SCL server.
/// Returns the path to the temp file.
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

/// Write a default global config if none exists.
pub fn ensure_global_config() -> Result<(), String> {
    let path = xdg::config_dir().join("config.toml");
    if path.exists() {
        return Ok(());
    }

    xdg::ensure_dirs()?;

    let default = r#"# Harness global configuration

[shared_context]
enabled = true
url = "http://127.0.0.1:3100/mcp"
"#;

    fs::write(&path, default)
        .map_err(|e| format!("Failed to write default config: {e}"))?;
    Ok(())
}

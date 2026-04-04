use serde::Deserialize;
use std::fs;

use crate::xdg;

#[derive(Debug, Deserialize, Default)]
pub struct GlobalConfig {
    pub shared_context: Option<SharedContextConfig>,
    pub bridge: Option<BridgeConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SharedContextConfig {
    pub enabled: Option<bool>,
    pub url: Option<String>,
    pub auto_record: Option<bool>,
}

impl SharedContextConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn url(&self) -> &str {
        self.url.as_deref().unwrap_or("http://127.0.0.1:3100/mcp")
    }

    pub fn auto_record(&self) -> bool {
        self.auto_record.unwrap_or(true)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BridgeConfig {
    /// When true, unknown/unimplemented policies deny by default (default: false)
    pub strict_policy_mode: Option<bool>,
    /// When true, require vault to implement the _policy endpoint (default: false).
    pub require_policy_endpoint: Option<bool>,
    /// Default workflow timeout in minutes for bridge-triggered runs (default: 30)
    pub workflow_timeout_minutes: Option<u64>,
    /// Max lines in the progress buffer for Telegram updates (default: 50)
    pub progress_buffer_size: Option<usize>,
}

impl BridgeConfig {
    pub fn strict_policy_mode(&self) -> bool {
        self.strict_policy_mode.unwrap_or(false)
    }

    pub fn require_policy_endpoint(&self) -> bool {
        self.require_policy_endpoint.unwrap_or(false)
    }

    pub fn workflow_timeout_minutes(&self) -> u64 {
        self.workflow_timeout_minutes.unwrap_or(30)
    }

    pub fn progress_buffer_size(&self) -> usize {
        self.progress_buffer_size.unwrap_or(50)
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

    pub fn bridge(&self) -> BridgeConfig {
        self.bridge.clone().unwrap_or(BridgeConfig {
            strict_policy_mode: None,
            require_policy_endpoint: None,
            workflow_timeout_minutes: None,
            progress_buffer_size: None,
        })
    }
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
auto_record = true

# [bridge]
# strict_policy_mode = false        # deny unknown policies by default
# require_policy_endpoint = false   # require vault _policy endpoint (optional by default)
# workflow_timeout_minutes = 30     # max runtime for bridge-triggered workflows
# progress_buffer_size = 50         # max lines in progress buffer for Telegram updates
"#;

    fs::write(&path, default)
        .map_err(|e| format!("Failed to write default config: {e}"))?;
    Ok(())
}

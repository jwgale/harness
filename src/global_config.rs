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
"#;

    fs::write(&path, default)
        .map_err(|e| format!("Failed to write default config: {e}"))?;
    Ok(())
}

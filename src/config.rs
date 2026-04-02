use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub backend: String,
    pub model: String,
    pub project_name: String,
    pub max_eval_rounds: u32,
    pub builder_timeout_seconds: u64,
    pub evaluator_timeout_seconds: u64,
    pub created_at: String,
}

impl Config {
    pub fn new(project_name: &str) -> Config {
        Config {
            backend: "claude".to_string(),
            model: "claude-opus-4-6".to_string(),
            project_name: project_name.to_string(),
            max_eval_rounds: 3,
            builder_timeout_seconds: 1800,
            evaluator_timeout_seconds: 600,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn load(harness_dir: &Path) -> Result<Config, String> {
        let path = harness_dir.join("config.json");
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read config: {e}"))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {e}"))
    }

    pub fn save(&self, harness_dir: &Path) -> Result<(), String> {
        let path = harness_dir.join("config.json");
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        fs::write(&path, json)
            .map_err(|e| format!("Failed to write config: {e}"))
    }
}

//! Agent definitions for multi-agent orchestration.
//!
//! Agents are TOML files in ~/.config/harness/agents/ that define named agent
//! configurations with a role, backend, optional prompt template, and tool list.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::xdg;

/// An agent definition loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    pub name: String,
    /// Role: planner, builder, evaluator, or custom
    pub role: String,
    /// Backend: claude, codex, or mock
    pub backend: String,
    /// Optional model override (defaults to project config model)
    pub model: Option<String>,
    /// Optional custom prompt template (inline or file path)
    pub prompt_template: Option<String>,
    /// Optional list of tools/capabilities this agent has access to
    pub tools: Option<Vec<String>>,
    /// Optional timeout override in seconds
    pub timeout_seconds: Option<u64>,
    /// Optional description
    pub description: Option<String>,
}

/// Discover all agent definitions from ~/.config/harness/agents/*.toml
pub fn discover() -> Vec<AgentDef> {
    let dir = xdg::agents_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut agents = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml")
            && let Ok(content) = fs::read_to_string(&path)
        {
            match toml::from_str::<AgentDef>(&content) {
                Ok(agent) => agents.push(agent),
                Err(e) => {
                    eprintln!("Warning: failed to parse agent {}: {e}", path.display());
                }
            }
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

/// Load a specific agent by name.
pub fn load(name: &str) -> Result<AgentDef, String> {
    let path = agent_path(name);
    if !path.exists() {
        return Err(format!("Agent '{name}' not found. Use `harness agent list` to see available agents."));
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read agent '{name}': {e}"))?;
    toml::from_str(&content)
        .map_err(|e| format!("Failed to parse agent '{name}': {e}"))
}

/// Create a new agent definition file.
pub fn add(name: &str, role: &str, backend: &str, description: Option<&str>) -> Result<(), String> {
    xdg::ensure_dirs()?;
    let path = agent_path(name);
    if path.exists() {
        return Err(format!("Agent '{name}' already exists. Remove it first."));
    }

    let agent = AgentDef {
        name: name.to_string(),
        role: role.to_string(),
        backend: backend.to_string(),
        model: None,
        prompt_template: None,
        tools: None,
        timeout_seconds: None,
        description: description.map(|s| s.to_string()),
    };

    let content = toml::to_string_pretty(&agent)
        .map_err(|e| format!("Failed to serialize agent: {e}"))?;
    fs::write(&path, content)
        .map_err(|e| format!("Failed to write agent file: {e}"))?;

    Ok(())
}

/// Remove an agent definition.
pub fn remove(name: &str) -> Result<(), String> {
    let path = agent_path(name);
    if !path.exists() {
        return Err(format!("Agent '{name}' not found."));
    }
    fs::remove_file(&path)
        .map_err(|e| format!("Failed to remove agent '{name}': {e}"))
}

/// Get the file path for an agent TOML.
fn agent_path(name: &str) -> PathBuf {
    xdg::agents_dir().join(format!("{name}.toml"))
}

/// Map a role string to the kind of CLI invocation needed.
/// - "builder" uses builder mode (full file I/O agent session)
/// - everything else uses oneshot
#[allow(dead_code)]
pub fn is_builder_role(role: &str) -> bool {
    role == "builder"
}

/// Built-in role descriptions for display.
pub fn role_description(role: &str) -> &'static str {
    match role {
        "planner" => "Generates spec.md from the project goal",
        "builder" => "Implements the spec with full file I/O access",
        "evaluator" => "Assesses the build quality and produces a verdict",
        _ => "Custom agent role",
    }
}

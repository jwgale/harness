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
    /// Backend: claude, codex, mock, or grok
    pub backend: String,
    /// Optional model override (defaults to project config model)
    pub model: Option<String>,
    /// Optional custom prompt template (inline or file path)
    pub prompt_template: Option<String>,
    /// Optional list of tools/capabilities this agent has access to
    pub tools: Option<Vec<String>>,
    /// Optional domain specializations, e.g. frontend, backend, testing
    pub specializations: Option<Vec<String>>,
    /// Optional context scopes to query from the Shared Context Layer
    pub context_scopes: Option<Vec<String>>,
    /// Optional specialization aliases this agent should be selected for
    pub default_for: Option<Vec<String>>,
    /// Optional timeout override in seconds
    pub timeout_seconds: Option<u64>,
    /// Optional description
    pub description: Option<String>,

    // === Grok-Native Future Extensions (Malleable) ===
    /// Marks this agent as a Supervisor / Super Agent with elevated authority.
    /// Supervisor agents can detect drift and have permission to rewrite/replan work.
    #[serde(default)]
    pub supervisor: bool,

    /// Authority granted to this agent when acting as a supervisor.
    /// Only meaningful if `supervisor = true`.
    #[serde(default)]
    pub authority: Vec<SupervisorAuthority>,

    /// Whether this agent is allowed to enter Plan Mode for high-ambiguity or drift-correction tasks.
    #[serde(default)]
    pub allow_plan_mode: bool,
}

/// Authority levels for Supervisor ("Super") agents.
/// This enum is designed to be extended over time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorAuthority {
    /// Can read artifacts and outputs from other agents in the current workflow.
    Read,
    /// Can directly rewrite or modify artifacts produced by other agents.
    Rewrite,
    /// Can trigger replanning (enter Plan Mode) for sections of the workflow.
    Replan,
    /// Can spawn additional specialist subagents during execution.
    Spawn,
    /// Can escalate issues or terminate underperforming subagents.
    Escalate,
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

/// Returns true if this agent definition is marked as a supervisor (either via the
/// `supervisor` field or has non-empty authority).
pub fn is_supervisor(agent: &AgentDef) -> bool {
    agent.supervisor || !agent.authority.is_empty()
}

/// Returns the effective authorities for an agent (combining explicit authority + defaults).
pub fn effective_authority(agent: &AgentDef) -> Vec<SupervisorAuthority> {
    if agent.authority.is_empty() && agent.supervisor {
        // Default supervisor authorities if none specified
        vec![
            SupervisorAuthority::Read,
            SupervisorAuthority::Rewrite,
            SupervisorAuthority::Replan,
        ]
    } else {
        agent.authority.clone()
    }
}

/// Load a specific agent by name.
pub fn load(name: &str) -> Result<AgentDef, String> {
    let path = agent_path(name);
    if !path.exists() {
        return Err(format!(
            "Agent '{name}' not found. Use `harness agent list` to see available agents."
        ));
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("Failed to read agent '{name}': {e}"))?;
    toml::from_str(&content).map_err(|e| format!("Failed to parse agent '{name}': {e}"))
}

/// Resolve an agent reference. Plain names load exact agent definitions.
/// References starting with '@' select an agent by specialization/default alias.
pub fn resolve(reference: &str) -> Result<AgentDef, String> {
    if let Some(selector) = reference.strip_prefix('@') {
        return resolve_selector(selector);
    }

    load(reference)
}

fn resolve_selector(selector: &str) -> Result<AgentDef, String> {
    let selector = normalize_tag(selector);
    if selector.is_empty() {
        return Err("Agent selector '@' is empty.".to_string());
    }

    let agents = discover();
    let mut default_matches = Vec::new();
    let mut specialization_matches = Vec::new();

    for agent in agents {
        if agent.default_for_tags().iter().any(|tag| tag == &selector) {
            default_matches.push(agent.clone());
        }
        if agent.supports(&selector) {
            specialization_matches.push(agent);
        }
    }

    match default_matches.len() {
        1 => return Ok(default_matches.remove(0)),
        n if n > 1 => {
            let names = default_matches
                .iter()
                .map(|agent| agent.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Agent selector '@{selector}' is ambiguous: multiple defaults match ({names})."
            ));
        }
        _ => {}
    }

    match specialization_matches.len() {
        1 => Ok(specialization_matches.remove(0)),
        0 => Err(format!(
            "No agent matches selector '@{selector}'. Add `specializations = [\"{selector}\"]` or `default_for = [\"{selector}\"]` to an agent."
        )),
        _ => {
            let names = specialization_matches
                .iter()
                .map(|agent| agent.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "Agent selector '@{selector}' is ambiguous: {names}. Set `default_for = [\"{selector}\"]` on one agent."
            ))
        }
    }
}

/// Create a new agent definition file.
pub fn add(
    name: &str,
    role: &str,
    backend: &str,
    description: Option<&str>,
    specializations: Option<Vec<String>>,
    context_scopes: Option<Vec<String>>,
    default_for: Option<Vec<String>>,
) -> Result<(), String> {
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
        specializations,
        context_scopes,
        default_for,
        timeout_seconds: None,
        description: description.map(|s| s.to_string()),
        // New Grok-native fields (defaults for now)
        supervisor: false,
        authority: vec![],
        allow_plan_mode: false,
    };

    let content =
        toml::to_string_pretty(&agent).map_err(|e| format!("Failed to serialize agent: {e}"))?;
    fs::write(&path, content).map_err(|e| format!("Failed to write agent file: {e}"))?;

    Ok(())
}

/// Remove an agent definition.
pub fn remove(name: &str) -> Result<(), String> {
    let path = agent_path(name);
    if !path.exists() {
        return Err(format!("Agent '{name}' not found."));
    }
    fs::remove_file(&path).map_err(|e| format!("Failed to remove agent '{name}': {e}"))
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

impl AgentDef {
    pub fn specialization_tags(&self) -> Vec<String> {
        normalized_tags(self.specializations.as_deref().unwrap_or(&[]))
    }

    pub fn context_scope_tags(&self) -> Vec<String> {
        normalized_tags(self.context_scopes.as_deref().unwrap_or(&[]))
    }

    pub fn default_for_tags(&self) -> Vec<String> {
        normalized_tags(self.default_for.as_deref().unwrap_or(&[]))
    }

    pub fn supports(&self, requirement: &str) -> bool {
        let requirement = normalize_tag(requirement);
        if requirement.is_empty() {
            return true;
        }

        normalize_tag(&self.role) == requirement
            || self
                .specialization_tags()
                .iter()
                .chain(self.default_for_tags().iter())
                .any(|tag| tag == &requirement)
    }

    pub fn missing_requirements(&self, requirements: &[String]) -> Vec<String> {
        requirements
            .iter()
            .filter_map(|requirement| {
                let normalized = normalize_tag(requirement);
                if normalized.is_empty() || self.supports(&normalized) {
                    None
                } else {
                    Some(requirement.clone())
                }
            })
            .collect()
    }

    pub fn identity_summary(&self) -> Option<String> {
        let mut parts = Vec::new();

        let specializations = self.specialization_tags();
        if !specializations.is_empty() {
            parts.push(format!("specializations: {}", specializations.join(", ")));
        }

        let tools = self.tools.as_deref().unwrap_or(&[]);
        if !tools.is_empty() {
            parts.push(format!("tools: {}", tools.join(", ")));
        }

        let context_scopes = self.context_scope_tags();
        if !context_scopes.is_empty() {
            parts.push(format!("context scopes: {}", context_scopes.join(", ")));
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }
}

fn normalized_tags(tags: &[String]) -> Vec<String> {
    tags.iter()
        .map(|tag| normalize_tag(tag))
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().to_ascii_lowercase().replace(' ', "-")
}

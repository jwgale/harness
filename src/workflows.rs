//! Workflow definitions for multi-agent orchestration.
//!
//! Workflows are TOML files in ~/.config/harness/workflows/ that define
//! a named sequence of agents to run.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::xdg;

/// A workflow step — references an agent by name with optional overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Agent name (must match a defined agent)
    pub agent: String,
    /// Optional prompt override for this step
    pub prompt: Option<String>,
    /// Optional: artifact to write the output to (e.g. "spec.md")
    pub output_artifact: Option<String>,
}

/// A workflow definition loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    pub description: Option<String>,
    /// Maximum rounds for evaluator loops (default: 3)
    pub max_rounds: Option<u32>,
    /// Ordered list of steps to execute
    pub steps: Vec<WorkflowStep>,
}

/// Discover all workflows from ~/.config/harness/workflows/*.toml
#[allow(dead_code)]
pub fn discover() -> Vec<WorkflowDef> {
    let dir = xdg::workflows_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut workflows = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml")
            && let Ok(content) = fs::read_to_string(&path)
        {
            match toml::from_str::<WorkflowDef>(&content) {
                Ok(wf) => workflows.push(wf),
                Err(e) => {
                    eprintln!("Warning: failed to parse workflow {}: {e}", path.display());
                }
            }
        }
    }
    workflows.sort_by(|a, b| a.name.cmp(&b.name));
    workflows
}

/// Load a specific workflow by name.
pub fn load(name: &str) -> Result<WorkflowDef, String> {
    let path = workflow_path(name);
    if !path.exists() {
        return Err(format!(
            "Workflow '{name}' not found in {}. Use `harness agent list` to see available agents.",
            xdg::workflows_dir().display()
        ));
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read workflow '{name}': {e}"))?;
    toml::from_str(&content)
        .map_err(|e| format!("Failed to parse workflow '{name}': {e}"))
}

fn workflow_path(name: &str) -> PathBuf {
    xdg::workflows_dir().join(format!("{name}.toml"))
}

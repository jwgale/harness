//! Workflow definitions for multi-agent orchestration.
//!
//! Workflows are TOML files in ~/.config/harness/workflows/ that define
//! a named sequence of agents to run, with support for parallel execution
//! and iterative build-evaluate loops.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::agents;
use crate::xdg;

/// A workflow step — references an agent by name with optional overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Agent name, or '@specialization' selector
    pub agent: String,
    /// Required agent specializations for this step
    #[serde(default)]
    pub requires: Vec<String>,
    /// Optional prompt override for this step
    pub prompt: Option<String>,
    /// Optional: artifact to write the output to (e.g. "spec.md")
    pub output_artifact: Option<String>,
    /// Run this step in parallel with adjacent parallel steps (default: false)
    #[serde(default)]
    pub parallel: bool,
    /// Loop this step until the evaluator passes (only meaningful for evaluator role).
    /// When set on a builder step, the next evaluator step forms a build-evaluate loop.
    pub loop_until: Option<String>,
    /// Max iterations for loop_until (default: 3)
    pub max_rounds: Option<u32>,
}

/// A workflow definition loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    pub description: Option<String>,
    /// Default maximum rounds for iterative loops (default: 3)
    pub max_rounds: Option<u32>,
    /// Timeout in minutes for bridge-triggered runs (overrides global config)
    pub timeout_minutes: Option<u64>,
    /// Ordered list of steps to execute
    pub steps: Vec<WorkflowStep>,
}

/// A group of steps to execute — either a single sequential step,
/// a parallel batch, or an iterative loop pair.
#[derive(Debug)]
pub enum StepGroup {
    /// Single sequential step
    Single(WorkflowStep),
    /// Group of steps to run concurrently
    Parallel(Vec<WorkflowStep>),
    /// Iterative loop: builder(s) + evaluator, repeating until pass or max rounds
    Loop {
        body: Vec<WorkflowStep>,
        evaluator: WorkflowStep,
        max_rounds: u32,
    },
}

/// Parse workflow steps into execution groups (sequential, parallel batches, loops).
pub fn plan_execution(wf: &WorkflowDef) -> Vec<StepGroup> {
    let default_max = wf.max_rounds.unwrap_or(3);
    let mut groups: Vec<StepGroup> = Vec::new();
    let steps = &wf.steps;
    let mut i = 0;

    while i < steps.len() {
        let step = &steps[i];

        // Check for loop_until on this step — collect body steps until we find an evaluator
        if step.loop_until.is_some() {
            let max = step.max_rounds.unwrap_or(default_max);
            let mut body = vec![step.clone()];
            i += 1;

            // Scan forward for evaluator
            let mut evaluator_idx = None;
            for (j, candidate_step) in steps.iter().enumerate().skip(i) {
                let candidate = agents::resolve(&candidate_step.agent).ok();
                if candidate
                    .as_ref()
                    .map(|a| a.role == "evaluator")
                    .unwrap_or(false)
                {
                    evaluator_idx = Some(j);
                    break;
                }
            }

            if let Some(eval_i) = evaluator_idx {
                // Collect body steps between current and evaluator
                while i < eval_i {
                    body.push(steps[i].clone());
                    i += 1;
                }
                groups.push(StepGroup::Loop {
                    body,
                    evaluator: steps[eval_i].clone(),
                    max_rounds: max,
                });
                i = eval_i + 1;
            } else {
                // No evaluator found — treat as sequential
                for s in body {
                    groups.push(StepGroup::Single(s));
                }
            }
            continue;
        }

        // Check for parallel group — collect adjacent parallel: true steps
        if step.parallel {
            let mut batch = vec![step.clone()];
            i += 1;
            while i < steps.len() && steps[i].parallel {
                batch.push(steps[i].clone());
                i += 1;
            }
            groups.push(StepGroup::Parallel(batch));
            continue;
        }

        // Regular sequential step
        groups.push(StepGroup::Single(step.clone()));
        i += 1;
    }

    groups
}

/// Validate a workflow definition. Returns a list of errors (empty = valid).
pub fn validate(wf: &WorkflowDef) -> Vec<String> {
    let mut errors = Vec::new();

    if wf.steps.is_empty() {
        errors.push("Workflow has no steps".to_string());
        return errors;
    }

    let valid_backends = ["claude", "codex", "mock"];
    let valid_roles = ["planner", "builder", "evaluator", "custom"];

    for (i, step) in wf.steps.iter().enumerate() {
        let step_label = format!("Step {} (agent '{}')", i + 1, step.agent);

        // Check agent exists
        match agents::resolve(&step.agent) {
            Ok(agent) => {
                // Validate backend
                if !valid_backends.contains(&agent.backend.as_str()) {
                    errors.push(format!(
                        "{step_label}: invalid backend '{}'. Use one of: {}",
                        agent.backend,
                        valid_backends.join(", ")
                    ));
                }
                // Validate role
                if !valid_roles.contains(&agent.role.as_str()) {
                    errors.push(format!(
                        "{step_label}: invalid role '{}'. Use one of: {}",
                        agent.role,
                        valid_roles.join(", ")
                    ));
                }
                let missing = agent.missing_requirements(&step.requires);
                if !missing.is_empty() {
                    errors.push(format!(
                        "{step_label}: agent '{}' does not satisfy required specialization(s): {}",
                        agent.name,
                        missing.join(", ")
                    ));
                }
                // Validate loop_until
                if let Some(until) = &step.loop_until {
                    if until != "pass" && until != "evaluator_pass" {
                        errors.push(format!(
                            "{step_label}: invalid loop_until '{until}'. Use 'pass' or 'evaluator_pass'."
                        ));
                    }
                    // loop_until should be on a builder, not evaluator
                    if agent.role == "evaluator" {
                        errors.push(format!(
                            "{step_label}: loop_until should be on a builder step, not an evaluator."
                        ));
                    }
                }
            }
            Err(e) => {
                errors.push(format!("{step_label}: {e}"));
            }
        }
    }

    // Check for loop_until without a following evaluator
    for (i, step) in wf.steps.iter().enumerate() {
        if step.loop_until.is_some() {
            let has_evaluator_after = wf.steps[i + 1..].iter().any(|s| {
                agents::resolve(&s.agent)
                    .map(|a| a.role == "evaluator")
                    .unwrap_or(false)
            });
            if !has_evaluator_after {
                errors.push(format!(
                    "Step {} (agent '{}'): loop_until requires a subsequent evaluator step.",
                    i + 1,
                    step.agent
                ));
            }
        }
    }

    errors
}

/// Resolve workflow step agent references to concrete agent names.
pub fn resolved_agent_names(wf: &WorkflowDef) -> Result<Vec<String>, String> {
    wf.steps
        .iter()
        .map(|step| agents::resolve(&step.agent).map(|agent| agent.name))
        .collect()
}

/// Discover all workflows from ~/.config/harness/workflows/*.toml
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
            "Workflow '{name}' not found in {}.",
            xdg::workflows_dir().display()
        ));
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("Failed to read workflow '{name}': {e}"))?;
    toml::from_str(&content).map_err(|e| format!("Failed to parse workflow '{name}': {e}"))
}

fn workflow_path(name: &str) -> PathBuf {
    xdg::workflows_dir().join(format!("{name}.toml"))
}

//! Request / Response protocol between Harness and Grok Build sessions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The phase of work being requested from Grok.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Planner,
    Builder,
    Evaluator,
}

/// Metadata attached to every Grok request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokRequestMetadata {
    /// Unique ID for this request (used for response matching)
    pub request_id: String,
    /// Project name
    pub project_name: String,
    /// Original goal
    pub goal: String,
    /// Model hint (if any)
    pub model: Option<String>,
    /// Timestamp (ISO 8601)
    pub created_at: String,
    /// Additional context (spec, status, feedback, file listings, etc.)
    #[serde(default)]
    pub context: HashMap<String, String>,  // Using String for simplicity in v1
}

/// A request sent from Harness to a Grok Build session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokRequest {
    pub phase: Phase,
    pub metadata: GrokRequestMetadata,
    /// The fully assembled prompt (including spec, feedback, file listings, etc.)
    pub prompt: String,
    /// Timeout in seconds
    pub timeout_seconds: u64,
    /// Whether this is part of an iterative loop (builder after feedback)
    #[serde(default)]
    pub is_revision: bool,
    /// Current round number (for builder/evaluator loops)
    #[serde(default)]
    pub round: u32,
}

/// The response written back by the Grok session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokResponse {
    pub request_id: String,
    pub phase: Phase,
    /// Whether the Grok session successfully completed the work
    pub success: bool,
    /// The main output (spec.md content, builder status, evaluation, etc.)
    pub output: String,
    /// Optional error message
    pub error: Option<String>,
    /// Rich trace of what Grok actually did (subagents, tools used, plan_mode entries, etc.)
    pub trace: Option<serde_json::Value>,
    /// Timing information
    pub duration_ms: Option<u64>,
    /// Any files Grok explicitly wants to highlight (e.g. new modules created)
    #[serde(default)]
    pub artifacts_written: Vec<String>,
}

impl GrokRequest {
    pub fn new(
        phase: Phase,
        project_name: String,
        goal: String,
        prompt: String,
        timeout_seconds: u64,
    ) -> Self {
        let request_id = format!(
            "{}-{}",
            chrono::Utc::now().timestamp_millis(),
            rand::random::<u32>()
        );

        Self {
            phase,
            metadata: GrokRequestMetadata {
                request_id,
                project_name,
                goal,
                model: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                context: HashMap::new(),
            },
            prompt,
            timeout_seconds,
            is_revision: false,
            round: 1,
        }
    }

    /// Create a request for the evaluator phase with extra context.
    pub fn new_evaluator(
        project_name: String,
        goal: String,
        prompt: String,
        timeout_seconds: u64,
        round: u32,
    ) -> Self {
        let mut req = Self::new(Phase::Evaluator, project_name, goal, prompt, timeout_seconds);
        req.round = round;
        req
    }

    /// Populate rich context from .harness/ artifacts.
    /// This makes the request much more useful for a Grok session.
    pub fn enrich_from_artifacts(&mut self) -> Result<(), String> {
        let harness = crate::artifacts::harness_dir();

        // Always include goal if not already in context
        if !self.metadata.context.contains_key("goal") {
            if let Ok(goal) = crate::artifacts::read_artifact("goal.md") {
                self.metadata.context.insert("goal".to_string(), goal);
            }
        }

        // Include spec if it exists (important for builder and evaluator)
        if crate::artifacts::artifact_exists("spec.md") {
            if let Ok(spec) = crate::artifacts::read_artifact("spec.md") {
                self.metadata.context.insert("spec".to_string(), spec);
            }
        }

        // Include latest status for builder/evaluator
        if crate::artifacts::artifact_exists("status.md") {
            if let Ok(status) = crate::artifacts::read_artifact("status.md") {
                self.metadata.context.insert("status".to_string(), status);
            }
        }

        // Include latest feedback if in revision
        if self.is_revision {
            let feedback_num = crate::artifacts::next_feedback_number().saturating_sub(1);
            let feedback_path = format!("feedback/round-{:03}.md", feedback_num);
            if crate::artifacts::artifact_exists(&feedback_path) {
                if let Ok(feedback) = crate::artifacts::read_artifact(&feedback_path) {
                    self.metadata.context.insert("latest_feedback".to_string(), feedback);
                }
            }
        }

        // Include project file listing (very useful for Grok)
        let files = crate::artifacts::list_project_files();
        if !files.is_empty() {
            self.metadata.context.insert("project_files".to_string(), files);
        }

        // Include endpoints.json if present (for curl evaluator strategy)
        if crate::artifacts::artifact_exists("endpoints.json") {
            if let Ok(endpoints) = crate::artifacts::read_artifact("endpoints.json") {
                self.metadata.context.insert("endpoints".to_string(), endpoints);
            }
        }

        Ok(())
    }
}

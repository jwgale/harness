//! Master TODO — the central coordination mechanism for Grok-native multi-agent workflows.
//!
//! In the future Grok Build model, complex or long-running workflows are not driven
//! purely by prompt context. Instead, a top-level Orchestrator / Supervisor Subagent
//! maintains a **Master TODO** that tracks the entire workflow state.
//!
//! Individual subagents (and supervisor agents) update this TODO as they work.
//! This provides much better state management, drift detection, and self-correction
//! than stuffing everything into the LLM context window.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Status of an individual task in the Master TODO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Done,
    Blocked,
    Failed,
}

/// A single task in the Master TODO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoTask {
    pub id: String,
    pub description: String,
    pub status: TodoStatus,
    /// Which subagent (or "supervisor", "orchestrator") is responsible
    pub assigned_to: Option<String>,
    /// Other task IDs this depends on
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Free-form notes, blockers, decisions, etc.
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The Master TODO for an entire workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterTodo {
    /// Unique ID of the workflow run this TODO belongs to
    pub workflow_run_id: String,
    pub workflow_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    /// All tasks in the workflow
    pub tasks: HashMap<String, TodoTask>,

    /// High-level status of the whole workflow
    pub overall_status: TodoStatus,

    /// Free-form context / decisions made by the orchestrator or supervisor
    #[serde(default)]
    pub orchestrator_notes: Vec<String>,

    // === New: Structured orchestration decisions (DeviationAnalysis + OverrideProposal) ===
    /// A log of structured orchestration decisions made during the workflow.
    /// This is the primary place where DeviationAnalysis and OverrideProposal live.
    #[serde(default)]
    pub decisions: Vec<OrchestrationDecisionRecord>,
}

/// A record of a high-level orchestration decision (for machine/agent reasoning).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationDecisionRecord {
    pub id: String,
    pub decision_type: String, // "deviation_analysis", "override_proposal", "replan", etc.
    pub deviation_analysis: Option<crate::grok::DeviationAnalysis>,
    pub override_proposal: Option<crate::grok::OverrideProposal>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub notes: Option<String>,
}

impl MasterTodo {
    pub fn new(workflow_name: String, run_id: String) -> Self {
        let now = Utc::now();
        Self {
            workflow_run_id: run_id,
            workflow_name,
            created_at: now,
            updated_at: now,
            tasks: HashMap::new(),
            overall_status: TodoStatus::Pending,
            orchestrator_notes: Vec::new(),
            decisions: Vec::new(),
        }
    }

    pub fn add_task(
        &mut self,
        id: String,
        description: String,
        assigned_to: Option<String>,
        depends_on: Vec<String>,
    ) -> &mut TodoTask {
        let now = Utc::now();
        let task = TodoTask {
            id: id.clone(),
            description,
            status: TodoStatus::Pending,
            assigned_to,
            depends_on,
            notes: None,
            created_at: now,
            updated_at: now,
        };
        self.tasks.insert(id.clone(), task);
        self.updated_at = now;
        self.tasks.get_mut(&id).unwrap()
    }

    pub fn update_status(&mut self, task_id: &str, status: TodoStatus) -> Result<(), String> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task '{}' not found in Master TODO", task_id))?;

        task.status = status;
        task.updated_at = Utc::now();
        self.updated_at = task.updated_at;

        self.recalculate_overall_status();
        Ok(())
    }

    pub fn add_note(&mut self, task_id: &str, note: String) -> Result<(), String> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;

        let note_entry = format!("[{}] {}", Utc::now().to_rfc3339(), note);
        match &mut task.notes {
            Some(existing) => {
                existing.push('\n');
                existing.push_str(&note_entry);
            }
            None => task.notes = Some(note_entry),
        }

        task.updated_at = Utc::now();
        self.updated_at = task.updated_at;
        Ok(())
    }

    pub fn add_orchestrator_note(&mut self, note: String) {
        let note_entry = format!("[{}] {}", Utc::now().to_rfc3339(), note);
        self.orchestrator_notes.push(note_entry);
        self.updated_at = Utc::now();
    }

    fn recalculate_overall_status(&mut self) {
        let has_in_progress = self.tasks.values().any(|t| t.status == TodoStatus::InProgress);
        let has_pending = self.tasks.values().any(|t| t.status == TodoStatus::Pending);
        let has_blocked = self.tasks.values().any(|t| t.status == TodoStatus::Blocked);
        let has_failed = self.tasks.values().any(|t| t.status == TodoStatus::Failed);

        self.overall_status = if has_failed {
            TodoStatus::Failed
        } else if has_blocked {
            TodoStatus::Blocked
        } else if has_in_progress || has_pending {
            TodoStatus::InProgress
        } else {
            TodoStatus::Done
        };
    }

    /// Get the next actionable task (simple heuristic: first pending task whose dependencies are met).
    pub fn get_next_actionable_task(&self) -> Option<&TodoTask> {
        self.tasks
            .values()
            .filter(|t| t.status == TodoStatus::Pending)
            .find(|t| {
                t.depends_on
                    .iter()
                    .all(|dep_id| self.tasks.get(dep_id).map_or(false, |dep| dep.status == TodoStatus::Done))
            })
    }
}
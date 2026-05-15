//! Master Agent Orchestrator (Supervisor Subagent) — Fleshing Out the Logic
//!
//! This is where the real power of Grok-native workflows will live.
//!
//! The `WorkflowOrchestrator` represents the **Master Agent** / top-level
//! Supervisor Subagent that is responsible for the entire workflow.
//!
//! Unlike regular specialist subagents, the Orchestrator has a broader view
//! (the full Master TODO + original goal + execution history) and is allowed
//! to make high-impact decisions, including rewriting previous work.

use crate::agents::SupervisorAuthority;
use crate::grok::MasterTodo;
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// The Master Orchestrator Subagent for a Grok-native workflow.
///
/// In a real run, this logic would be executed by a powerful Grok subagent
/// (or the current Grok Build session acting in the orchestrator role).
pub struct WorkflowOrchestrator {
    pub workflow_name: String,
    pub run_id: String,
    pub master_todo: MasterTodo,
    /// The original high-level goal of the workflow (very important for drift detection)
    pub original_goal: Option<String>,
}

impl WorkflowOrchestrator {
    pub fn new(workflow_name: String, run_id: String, master_todo: MasterTodo) -> Self {
        // Try to load the original goal for better replanning context
        let original_goal = crate::artifacts::read_artifact("goal.md").ok();

        Self {
            workflow_name,
            run_id,
            master_todo,
            original_goal,
        }
    }

    /// The core decision-making method for the Orchestrator.
    ///
    /// This is the heart of the Master Agent. A powerful Grok subagent (or the
    /// current Grok Build session) would run this logic with full context:
    /// the Master TODO, original goal, execution history, and rich trace.
    pub fn decide_next_action(&self) -> OrchestratorDecision {
        // First, evaluate drift. This is a key signal for supervisor-level thinking.
        let drift = self.evaluate_drift();

        // === Priority 1: Serious problems that require replanning ===
        if drift.severity >= DriftSeverity::High {
            return OrchestratorDecision::EnterPlanMode {
                reason: format!(
                    "High-severity drift detected: {}. The Orchestrator should enter plan_mode to replan the affected work.",
                    drift.reasons.join("; ")
                ),
            };
        }

        // === Priority 2: Blocked or Failed tasks (medium severity drift) ===
        if let Some(failed_task) = self.master_todo.tasks.values()
            .find(|t| t.status == crate::grok::TodoStatus::Failed)
        {
            return OrchestratorDecision::EnterPlanMode {
                reason: format!(
                    "Task '{}' has failed. Supervisor intervention and replanning are required.",
                    failed_task.description
                ),
            };
        }

        if let Some(blocked_task) = self.master_todo.tasks.values()
            .find(|t| t.status == crate::grok::TodoStatus::Blocked)
        {
            // If drift is already Medium or higher, we escalate to plan mode.
            // Otherwise, we still recommend investigation.
            if drift.severity >= DriftSeverity::Medium {
                return OrchestratorDecision::EnterPlanMode {
                    reason: format!(
                        "Task '{}' is blocked and medium/high drift is present. Replanning is recommended.",
                        blocked_task.description
                    ),
                };
            } else {
                return OrchestratorDecision::EnterPlanMode {
                    reason: format!(
                        "Task '{}' is blocked. The Orchestrator should investigate root cause and decide on next steps.",
                        blocked_task.description
                    ),
                };
            }
        }

        // === Priority 3: Look for long-running / stuck tasks ===
        // (In a real system we would track start times. For now we use a simple heuristic.)
        let stuck_tasks: Vec<_> = self.master_todo.tasks.values()
            .filter(|t| t.status == crate::grok::TodoStatus::InProgress)
            .collect();

        if stuck_tasks.len() >= 2 {
            return OrchestratorDecision::EnterPlanMode {
                reason: format!(
                    "{} tasks are currently InProgress. This may indicate the workflow is losing focus or has dependency issues. Consider replanning.",
                    stuck_tasks.len()
                ),
            };
        }

        // === Priority 4: Normal forward progress ===
        if let Some(next_task) = self.master_todo.get_next_actionable_task() {
            return OrchestratorDecision::AssignTask {
                task_id: next_task.id.clone(),
                description: next_task.description.clone(),
                suggested_agent: next_task.assigned_to.clone(),
            };
        }

        // === Priority 5: Workflow appears complete — final supervisor review ===
        let all_done = self.master_todo.tasks.values().all(|t| t.status == crate::grok::TodoStatus::Done);
        if all_done {
            if self.master_todo.overall_status != crate::grok::TodoStatus::Done {
                return OrchestratorDecision::EnterPlanMode {
                    reason: "All tasks are marked Done, but the overall status is not yet confirmed. The Orchestrator should perform a final quality review (potentially using plan_mode) before closing the workflow.".to_string(),
                };
            } else {
                return OrchestratorDecision::NoActionableWork;
            }
        }

        // === Fallback ===
        OrchestratorDecision::NoActionableWork
    }

    /// Detect whether the current state of the work has drifted from the original intent.
    ///
    /// In the future, a real Grok subagent would use this + the original goal + spec
    /// to decide whether to trigger a replan or rewrite.
    pub fn evaluate_drift(&self) -> DriftReport {
        let mut report = DriftReport {
            has_drift: false,
            severity: DriftSeverity::None,
            reasons: vec![],
        };

        // Very basic drift heuristics for the prototype.
        // Real version would compare against spec.md, original goal, and rich execution trace.

        let blocked_count = self.master_todo.tasks.values()
            .filter(|t| t.status == crate::grok::TodoStatus::Blocked)
            .count();

        let failed_count = self.master_todo.tasks.values()
            .filter(|t| t.status == crate::grok::TodoStatus::Failed)
            .count();

        if failed_count > 0 {
            report.has_drift = true;
            report.severity = DriftSeverity::High;
            report.reasons.push(format!("{} task(s) have failed", failed_count));
        }

        if blocked_count > 1 {
            report.has_drift = true;
            report.severity = DriftSeverity::Medium;
            report.reasons.push(format!("Multiple tasks ({}) are blocked", blocked_count));
        }

        // Long-running in-progress tasks could also be a signal
        // (we could add age tracking later)

        report
    }

    /// Suggest a replanning action (this would typically trigger `plan_mode` in the current Grok session).
    pub fn propose_replan(&self) -> Option<ReplanDirective> {
        let drift = self.evaluate_drift();

        if !drift.has_drift {
            return None;
        }

        Some(ReplanDirective {
            affected_tasks: self.master_todo.tasks.keys().cloned().collect(),
            reason: drift.reasons.join("; "),
            proposed_changes: "The Orchestrator recommends entering plan_mode to reassess the approach and potentially rewrite or reprioritize sections of the work.".to_string(),
            severity: drift.severity,
        })
    }

    /// Generate a high-quality prompt that the current Grok session can use to enter `plan_mode`
    /// when the Orchestrator has decided that replanning is needed.
    ///
    /// This is one of the most powerful integration points between the Harness orchestration layer
    /// and Grok's native capabilities.
    pub fn generate_plan_mode_prompt(&self, reason: &str) -> String {
        let mut prompt = String::new();

        prompt.push_str("You are now acting as the Master Workflow Orchestrator for a complex, long-running multi-agent build.\n\n");

        prompt.push_str("## Current Situation\n\n");
        prompt.push_str(&format!("**Workflow:** {}\n", self.workflow_name));
        prompt.push_str(&format!("**Run ID:** {}\n\n", self.run_id));

        if let Some(goal) = &self.original_goal {
            prompt.push_str(&format!("**Original Goal:**\n{}\n\n", goal));
        }

        prompt.push_str(&format!("**Why Replanning is Needed:**\n{}\n\n", reason));

        // Include drift report if relevant
        let drift = self.evaluate_drift();
        if drift.has_drift {
            prompt.push_str("**Drift Analysis:**\n");
            prompt.push_str(&format!("Severity: {:?}\n", drift.severity));
            for r in &drift.reasons {
                prompt.push_str(&format!("- {}\n", r));
            }
            prompt.push_str("\n");
        }

        // === Rich Context: Original Specification ===
        if let Ok(spec) = crate::artifacts::read_artifact("spec.md") {
            prompt.push_str("## Original Product Specification\n\n");
            // Include a substantial portion (first ~4000 chars to keep context reasonable)
            let spec_excerpt = if spec.len() > 4000 {
                format!("{}...\n\n[Spec truncated for prompt length. Full spec available in .harness/spec.md]", &spec[..4000])
            } else {
                spec
            };
            prompt.push_str(&spec_excerpt);
            prompt.push_str("\n\n");
        }

        // Snapshot of the current Master TODO
        prompt.push_str("## Current Master TODO State\n\n");
        prompt.push_str(&format!("Overall Status: {:?}\n\n", self.master_todo.overall_status));

        for (id, task) in &self.master_todo.tasks {
            prompt.push_str(&format!(
                "- [{}] {} — {:?} (assigned to: {})\n",
                id,
                task.description,
                task.status,
                task.assigned_to.as_deref().unwrap_or("unassigned")
            ));
            if let Some(notes) = &task.notes {
                prompt.push_str(&format!("    Notes: {}\n", notes.replace('\n', "\n    ")));
            }
        }

        if !self.master_todo.orchestrator_notes.is_empty() {
            prompt.push_str("\n**Orchestrator Notes So Far:**\n");
            for note in &self.master_todo.orchestrator_notes {
                prompt.push_str(&format!("- {}\n", note));
            }
            prompt.push_str("\n");
        }

        // === Automated SCL Context Pull ===
        // We automatically query the Shared Context Layer for relevant past work
        // (previous automated builds, design decisions, architecture, gotchas, etc.)
        // so the replanning session has rich historical context.
        if let Ok(scl_context) = self.query_relevant_scl_context() {
            if !scl_context.trim().is_empty() {
                prompt.push_str("## Relevant Past Work from Shared Context Layer\n\n");
                prompt.push_str(&scl_context);
                prompt.push_str("\n\n");
            }
        } else {
            prompt.push_str("## Relevant Past Work from Shared Context Layer\n\n");
            prompt.push_str("[SCL query attempted but no additional historical context was retrieved.]\n\n");
        }

        prompt.push_str("## Your Task as Orchestrator\n\n");
        prompt.push_str("You should now enter `plan_mode` to carefully reassess the situation.\n\n");
        prompt.push_str("Your goals during this planning session:\n");
        prompt.push_str("1. Understand why the current approach is struggling or has drifted (compare against the original spec above).\n");
        prompt.push_str("2. Decide whether specific tasks need to be rewritten, reprioritized, or broken down further.\n");
        prompt.push_str("3. Consider whether new specialist subagents should be spawned.\n");
        prompt.push_str("4. Update the Master TODO with a revised plan (including new tasks, changed assignments, or dependency adjustments).\n");
        prompt.push_str("5. Be willing to have previous work reworked if it no longer serves the original goal.\n\n");

        prompt.push_str("After exiting plan mode, you should:\n");
        prompt.push_str("- Clearly document the replanning decisions in the Master TODO (as orchestrator notes).\n");
        prompt.push_str("- Update task statuses and assignments as needed.\n");
        prompt.push_str("- Continue orchestrating the workflow using the improved plan.\n\n");

        prompt.push_str("Remember: You have supervisor authority. You are allowed (and expected) to propose significant changes, including rewriting earlier work, when it serves the long-term success of the project.\n");

        prompt
    }

    /// Automatically queries the Shared Context Layer for relevant historical context
    /// related to previous automated builds, design decisions, architecture, and gotchas.
    fn query_relevant_scl_context(&self) -> Result<String, String> {
        let gc = crate::global_config::GlobalConfig::load();
        let Some(scl_cfg) = gc.scl() else {
            return Err("No SCL configuration found".to_string());
        };

        if !crate::scl::is_healthy(scl_cfg.url()) {
            return Err("SCL is not healthy".to_string());
        }

        // Query for past automated build work, design decisions, and architecture
        // scoped to "harness" (or related scopes) with focus on L0/L1 knowledge.
        let args = serde_json::json!({
            "query": "automated build OR harness workflow OR grok-native OR design decision OR architecture decision OR gotcha OR drift OR replan",
            "scope": ["harness", "build", "grok", "workflow"],
            "tiers": ["L0", "L1"],
            "limit": 10,
            "retrieval_mode": "balanced"
        });

        let result = crate::scl::call_tool(scl_cfg.url(), "context_query", &args)?;

        // The result is usually JSON or text. We try to make it readable.
        if result.trim().starts_with('{') || result.trim().starts_with('[') {
            // Try to pretty-print if it's JSON
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result) {
                if let Some(items) = parsed.get("items").and_then(|v| v.as_array()) {
                    let mut formatted = String::new();
                    for item in items.iter().take(6) {
                        if let Some(content) = item.get("content").and_then(|c| c.as_str()) {
                            let kind = item.get("kind").and_then(|k| k.as_str()).unwrap_or("unknown");
                            formatted.push_str(&format!("- [{}] {}\n", kind, content));
                        }
                    }
                    if !formatted.is_empty() {
                        return Ok(formatted);
                    }
                }
            }
        }

        Ok(result)
    }

    /// Returns rich advisory output from the Orchestrator to the Supervisor.
    ///
    /// This is the evolved, more intelligent form of decision making. The orchestrator
    /// now acts as a proper advisor that can proactively suggest when the supervisor
    /// should consider deviating from the declared plan using structured data.
    pub fn advise(&self) -> OrchestratorAdvice {
        let decision = self.decide_next_action();
        let analysis = self.build_deviation_analysis(&decision);

        // Check if there's already a high-confidence pending OverrideProposal in the MasterTodo
        let suggested_override = self.master_todo.decisions.iter().rev().find_map(|record| {
            if let Some(proposal) = &record.override_proposal {
                if proposal.status == OverrideStatus::Pending && proposal.confidence >= 0.70 {
                    return Some(proposal.clone());
                }
            }
            None
        });

        let should_enter_plan_mode = matches!(decision, OrchestratorDecision::EnterPlanMode { .. });

        let reasoning = if let Some(analysis) = &analysis {
            if analysis.leverage_of_changing_course > 0.65 && analysis.strategic_alignment < 0.55 {
                format!(
                    "Significant deviation detected (leverage={:.2}, alignment={:.2}). Supervisor should consider overriding the current plan.",
                    analysis.leverage_of_changing_course, analysis.strategic_alignment
                )
            } else {
                "Current plan is reasonably aligned. Proceeding with orchestrator recommendation.".to_string()
            }
        } else {
            "No active supervisor detected. Following standard orchestration logic.".to_string()
        };

        OrchestratorAdvice {
            recommended_action: decision,
            deviation_analysis: analysis,
            suggested_override,
            should_enter_plan_mode,
            reasoning,
        }
    }

    /// Legacy method — kept for backward compatibility during the transition.
    /// New code should prefer `advise()`.
    pub fn decide_with_deviation_analysis(&self) -> (OrchestratorDecision, Option<DeviationAnalysis>) {
        let decision = self.decide_next_action();
        let analysis = self.build_deviation_analysis(&decision);
        (decision, analysis)
    }

    /// Builds a structured deviation analysis.
    /// This is currently heuristic-based but designed to be replaced by a powerful
    /// Grok subagent that can reason over the full Master TODO + history + SCL.
    fn build_deviation_analysis(&self, decision: &OrchestratorDecision) -> Option<DeviationAnalysis> {
        if !self.has_active_supervisor() {
            return None;
        }

        let now = chrono::Utc::now();
        let mut reasons: Vec<StructuredReason> = Vec::new();
        let mut drift_risk = 0.0;
        let mut execution_risk = 0.0;
        let mut leverage: f32 = 0.3;
        let mut alignment = 0.85;

        let drift = self.evaluate_drift();
        if drift.has_drift {
            drift_risk = match drift.severity {
                DriftSeverity::High => 0.85,
                DriftSeverity::Medium => 0.6,
                DriftSeverity::Low => 0.35,
                DriftSeverity::None => 0.1,
            };

            for r in drift.reasons {
                reasons.push(StructuredReason::DriftDetected {
                    severity: drift.severity,
                    affected_tasks: vec![],
                    description: r,
                });
            }
        }

        let has_blocked = self.master_todo.tasks.values().any(|t| t.status == crate::grok::TodoStatus::Blocked);
        let has_failed = self.master_todo.tasks.values().any(|t| t.status == crate::grok::TodoStatus::Failed);

        if has_failed {
            execution_risk = 0.9;
            leverage = 0.8;
            alignment = 0.4;
            reasons.push(StructuredReason::QualitySignal {
                source: "master_todo".to_string(),
                score: 0.2,
                description: "One or more tasks have failed.".to_string(),
            });
        } else if has_blocked {
            execution_risk = 0.65;
            leverage = 0.65;
            alignment = 0.55;
        }

        if matches!(decision, OrchestratorDecision::EnterPlanMode { .. }) {
            leverage = (leverage.max(0.75_f32)).min(1.0);
            reasons.push(StructuredReason::StrategicMisalignment {
                original_goal: self.original_goal.clone().unwrap_or_default(),
                current_trajectory: "Current path is hitting significant blockers or drift.".to_string(),
                gap_description: "Orchestrator has already recommended entering plan mode.".to_string(),
            });
        }

        if reasons.is_empty() {
            return Some(DeviationAnalysis {
                current_plan_alignment: 0.9,
                strategic_alignment: 0.88,
                drift_risk: 0.15,
                execution_risk: 0.2,
                leverage_of_changing_course: 0.25,
                reasons: vec![],
                confidence: 0.7,
                analyzed_at: now,
            });
        }

        Some(DeviationAnalysis {
            current_plan_alignment: alignment,
            strategic_alignment: alignment,
            drift_risk,
            execution_risk,
            leverage_of_changing_course: leverage,
            reasons,
            confidence: 0.75,
            analyzed_at: now,
        })
    }

    fn has_active_supervisor(&self) -> bool {
        self.master_todo.tasks.values().any(|t| {
            if let Some(name) = &t.assigned_to {
                name.contains("architect") 
                    || name.contains("supervisor") 
                    || name.contains("orchestrator")
                    || t.id == "orchestrator"   // Treat the top-level orchestrator task as supervisor mode
            } else {
                false
            }
        })
    }

    /// Generates an `OverrideProposal` if the current `DeviationAnalysis` indicates
    /// that the supervisor should seriously consider deviating from the orchestrator's plan.
    ///
    /// This is a key step in making the supervisor-orchestrator relationship machine-native.
    pub fn generate_override_proposal(&self, analysis: &DeviationAnalysis) -> Option<OverrideProposal> {
        // Simple threshold-based policy for the prototype.
        // In a more advanced version, this would be configurable or learned.
        let should_propose_override = analysis.leverage_of_changing_course > 0.60
            && (analysis.strategic_alignment < 0.55 || analysis.execution_risk > 0.65);

        if !should_propose_override {
            return None;
        }

        let decision = self.decide_next_action();

        // Determine the proposed alternative action
        let proposed_action = match &decision {
            OrchestratorDecision::EnterPlanMode { .. } => SupervisorAction::Replan,
            OrchestratorDecision::RewriteArtifact { artifact, .. } => {
                SupervisorAction::RewriteArtifact { artifact: artifact.clone() }
            }
            _ => SupervisorAction::Replan, // Default to replan for now
        };

        Some(OverrideProposal {
            id: format!("override-{}", Utc::now().timestamp()),
            workflow_run_id: self.run_id.clone(),
            from_plan: format!("{:?}", decision),
            to_plan: "Supervisor-proposed alternative (see DeviationAnalysis)".to_string(),
            deviation_analysis: analysis.clone(),
            authority_used: vec![SupervisorAuthority::Replan, SupervisorAuthority::Rewrite],
            proposed_action,
            confidence: analysis.confidence,
            created_by: "workflow-orchestrator".to_string(),
            status: OverrideStatus::Pending,
            created_at: Utc::now(),
            notes: Some(format!(
                "Auto-generated because leverage={:.2}, strategic_alignment={:.2}, execution_risk={:.2}",
                analysis.leverage_of_changing_course,
                analysis.strategic_alignment,
                analysis.execution_risk
            )),
        })
    }
}

/// High-level decisions the Orchestrator / Supervisor Subagent can make.
#[derive(Debug, Clone)]
pub enum OrchestratorDecision {
    /// Assign the next piece of work to a subagent
    AssignTask {
        task_id: String,
        description: String,
        suggested_agent: Option<String>,
    },
    /// The situation requires entering Plan Mode for replanning or major rewrites
    EnterPlanMode {
        reason: String,
    },
    /// Spawn a new specialist subagent
    SpawnSpecialist {
        specialization: String,
        parent_task: String,
    },
    /// The orchestrator believes previous work needs to be rewritten
    RewriteArtifact {
        artifact: String,
        reason: String,
    },
    /// No clear next action (workflow may be stuck or complete)
    NoActionableWork,
}

/// Rich, advisor-style output from the WorkflowOrchestrator to the Supervisor.
///
/// This is the evolved form of `decide_next_action()` we want for an intelligent,
/// machine-native supervisor-orchestrator relationship.
#[derive(Debug, Clone)]
pub struct OrchestratorAdvice {
    pub recommended_action: OrchestratorDecision,
    pub deviation_analysis: Option<DeviationAnalysis>,
    pub suggested_override: Option<OverrideProposal>,
    pub should_enter_plan_mode: bool,
    pub reasoning: String,
}

/// Result of a drift evaluation performed by the Orchestrator.
#[derive(Debug, Clone)]
pub struct DriftReport {
    pub has_drift: bool,
    pub severity: DriftSeverity,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DriftSeverity {
    None,
    Low,
    Medium,
    High,
}

/// Directive from the Orchestrator to replan part (or all) of the workflow.
#[derive(Debug, Clone)]
pub struct ReplanDirective {
    pub affected_tasks: Vec<String>,
    pub reason: String,
    pub proposed_changes: String,
    pub severity: DriftSeverity,
}

// ============================================================================
// Machine-First Orchestration Decision Types
// These are designed for maximum agent / machine understanding and reasoning.
// ============================================================================

/// Structured, machine-readable reasons why the current declared plan may need to change.
/// These are designed to be easily comparable, filterable, and queryable by agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StructuredReason {
    /// Significant drift has been detected from the original intent.
    DriftDetected {
        severity: DriftSeverity,
        affected_tasks: Vec<String>,
        description: String,
    },
    /// A clearly better execution path is available.
    BetterPathAvailable {
        alternative_approach: String,
        estimated_leverage: f32, // 0.0 – 1.0
        description: String,
    },
    /// Quality signals (from evaluator, supervisor review, SCL gotchas, etc.) indicate problems.
    QualitySignal {
        source: String, // e.g. "supervisor_review", "evaluator_feedback", "scl_gotcha"
        score: f32,
        description: String,
    },
    /// The current path is inefficient in time, tokens, or resources.
    ResourceInefficiency {
        current_path_cost: f32,
        better_path_cost: f32,
        description: String,
    },
    /// The current trajectory is diverging from the original goal.
    StrategicMisalignment {
        original_goal: String,
        current_trajectory: String,
        gap_description: String,
    },
    /// The supervisor has significantly higher confidence in an alternative approach.
    SupervisorJudgmentOverride {
        supervisor_confidence: f32,
        description: String,
    },
}

/// Structured analysis comparing the current declared orchestrator plan
/// against the live judgment of an active supervisor.
///
/// This is the core machine-readable artifact for supervisor override decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviationAnalysis {
    /// How well the current declared plan still aligns with the original intent (0.0–1.0)
    pub current_plan_alignment: f32,
    /// How well the current path aligns with the original strategic goal
    pub strategic_alignment: f32,
    /// Risk that the workflow has drifted or will produce low-quality output
    pub drift_risk: f32,
    /// Risk that continuing the current path will lead to being stuck or inefficient
    pub execution_risk: f32,
    /// How much better an alternative path is estimated to be
    pub leverage_of_changing_course: f32,
    /// Detailed, machine-readable reasons for the analysis
    pub reasons: Vec<StructuredReason>,
    /// How confident the supervisor is in this analysis (0.0–1.0)
    pub confidence: f32,
    pub analyzed_at: chrono::DateTime<chrono::Utc>,
}

/// High-level actions a supervisor can propose when deviating from the current plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorAction {
    Replan,
    RewriteArtifact { artifact: String },
    ReorderSteps { new_order: Vec<String> },
    SpawnNewAgent { specialization: String },
    TerminateTask { task_id: String },
    Escalate,
    NoChange,
}

/// Status of an override proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideStatus {
    Pending,
    Approved,
    Rejected,
    Superseded,
}

/// A formal, auditable proposal from a supervisor to deviate from the current orchestrator plan.
/// This is the core machine-readable artifact for supervisor override decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverrideProposal {
    pub id: String,
    pub workflow_run_id: String,
    pub from_plan: String,           // What the orchestrator currently recommends
    pub to_plan: String,             // What the supervisor wants to do instead
    pub deviation_analysis: DeviationAnalysis,
    pub authority_used: Vec<SupervisorAuthority>,
    pub proposed_action: SupervisorAction,
    pub confidence: f32,
    pub created_by: String,          // Which supervisor / agent name
    pub status: OverrideStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub notes: Option<String>,
}

/// Structured context that a supervisor brings to any subagent task it oversees.
/// This is the central object for making fulfillment prompts context-rich
/// when a supervisor is actively driving the workflow.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SupervisorContext {
    /// Name / identity of the active supervisor (e.g. "grok-architect")
    pub supervisor_name: String,

    /// The supervisor’s current standing orders / directives.
    /// Subagents should treat these as high-priority constraints.
    pub standing_orders: Vec<String>,

    /// Recent structured deviation analyses (so subagents understand *why* the supervisor is intervening).
    pub recent_deviation_analyses: Vec<DeviationAnalysis>,

    /// Currently active or recently approved OverrideProposals.
    /// Subagents should know what direction has already been sanctioned.
    pub active_override_proposals: Vec<OverrideProposal>,

    /// High-level supervisor decisions / notes relevant to the current workflow.
    pub key_decisions: Vec<String>,

    /// The authority the current supervisor holds.
    pub authority: Vec<SupervisorAuthority>,
}
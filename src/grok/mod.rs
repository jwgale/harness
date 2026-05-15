//! Grok Build native coordination layer.
//!
//! This module implements the protocol that allows Harness to delegate
//! planner/builder/evaluator work to a running Grok Build session (this TUI)
//! instead of spawning an external CLI.
//!
//! ## Protocol Overview
//!
//! When `harness run --backend grok` (or auto-detected) is executed:
//!
//! 1. Harness writes a structured request to `.harness/grok/requests/<phase>-<id>.json`
//! 2. The Grok session (us) detects the request (via file watching or explicit handoff)
//! 3. Grok performs the work using its native capabilities:
//!    - Subagents for parallel work
//!    - `plan_mode` for complex architecture decisions
//!    - `todo_write` for long-running build tracking
//!    - Direct GitHub MCP tools (branches, PRs, file ops)
//!    - Rich SCL recording with `relates_to` chains
//! 4. Grok writes the result + rich trace to `.harness/grok/responses/<id>.json`
//! 5. Harness picks up the response, fires plugins, updates artifacts, continues the loop
//!
//! This gives us dramatically more powerful loops than Claude Code or Codex while
//! preserving the `.harness/` artifact contract for interoperability.

pub mod coordinator;
pub mod master_todo;
pub mod orchestrator;
pub mod protocol;
pub mod trace;

pub use coordinator::GrokCoordinator;
pub use master_todo::{MasterTodo, OrchestrationDecisionRecord, TodoStatus, TodoTask};
pub use orchestrator::{
    DeviationAnalysis, DriftReport, DriftSeverity, OrchestratorDecision, OverrideProposal,
    OverrideStatus, ReplanDirective, StructuredReason, SupervisorAction, SupervisorContext,
    WorkflowOrchestrator,
};
pub use crate::agents::SupervisorAuthority;
pub use protocol::{GrokRequest, GrokRequestMetadata, GrokResponse, Phase};
pub use trace::{GrokTrace, GrokTraceEvent};

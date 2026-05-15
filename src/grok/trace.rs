//! Rich execution trace for Grok-native runs.
//!
//! This is one of the major advantages of Grok over Claude/Codex:
//! We can record extremely high-fidelity traces of what actually happened
//! (subagent launches, tool calls, plan_mode decisions, SCL CIDs, GitHub operations, etc.).

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// A rich trace of a Grok-native execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokTrace {
    pub request_id: String,
    pub phase: String,
    pub started_at: String,
    pub events: Vec<GrokTraceEvent>,
    pub subagents_launched: Vec<String>,
    pub plan_mode_entries: Vec<String>,
    pub scl_records: Vec<String>,
    pub github_operations: Vec<String>,
}

/// Individual event in the trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokTraceEvent {
    pub timestamp_ms: u64,
    pub event_type: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl GrokTrace {
    pub fn new(request_id: String, phase: &str) -> Self {
        Self {
            request_id,
            phase: phase.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            events: Vec::new(),
            subagents_launched: Vec::new(),
            plan_mode_entries: Vec::new(),
            scl_records: Vec::new(),
            github_operations: Vec::new(),
        }
    }

    pub fn add_event(&mut self, event_type: &str, content: impl Into<String>) {
        self.events.push(GrokTraceEvent {
            timestamp_ms: 0, // TODO: calculate relative time
            event_type: event_type.to_string(),
            content: content.into(),
            metadata: None,
        });
    }

    pub fn record_subagent(&mut self, name: &str) {
        self.subagents_launched.push(name.to_string());
        self.add_event("subagent_launched", format!("Spawned subagent: {}", name));
    }

    pub fn record_plan_mode(&mut self, decision: &str) {
        self.plan_mode_entries.push(decision.to_string());
        self.add_event("plan_mode", decision);
    }

    pub fn record_scl(&mut self, cid: &str) {
        self.scl_records.push(cid.to_string());
        self.add_event("scl_record", format!("Recorded to SCL: {}", cid));
    }

    pub fn record_github(&mut self, operation: &str) {
        self.github_operations.push(operation.to_string());
        self.add_event("github_mcp", operation);
    }
}

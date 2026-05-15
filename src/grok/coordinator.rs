//! GrokCoordinator - the bridge between Harness orchestration and a live Grok Build session.

use crate::grok::protocol::{GrokRequest, GrokResponse};
use crate::agents;
use crate::grok::{
    DeviationAnalysis, OrchestrationDecisionRecord, OverrideProposal, OverrideStatus,
    SupervisorAuthority, SupervisorContext,
};
use std::path::PathBuf;

/// Coordinates communication between the Rust Harness process and a Grok Build session.
pub struct GrokCoordinator {
    grok_dir: PathBuf,
}

impl GrokCoordinator {
    pub fn new(harness_dir: &std::path::Path) -> Self {
        let grok_dir = harness_dir.join("grok");
        Self { grok_dir }
    }

    /// Ensure the `.harness/grok/{requests,responses,traces}` directories exist.
    pub fn ensure_directories(&self) -> Result<(), String> {
        std::fs::create_dir_all(self.grok_dir.join("requests"))
            .map_err(|e| format!("Failed to create grok/requests: {e}"))?;
        std::fs::create_dir_all(self.grok_dir.join("responses"))
            .map_err(|e| format!("Failed to create grok/responses: {e}"))?;
        std::fs::create_dir_all(self.grok_dir.join("traces"))
            .map_err(|e| format!("Failed to create grok/traces: {e}"))?;
        Ok(())
    }

    /// Write a request for the Grok session to pick up.
    pub fn write_request(&self, request: &GrokRequest) -> Result<PathBuf, String> {
        self.ensure_directories()?;
        let path = self.grok_dir.join("requests").join(format!(
            "{}-{}.json",
            request.phase.as_str(),
            request.metadata.request_id
        ));

        let content = serde_json::to_string_pretty(request)
            .map_err(|e| format!("Failed to serialize GrokRequest: {e}"))?;

        std::fs::write(&path, content).map_err(|e| format!("Failed to write request file: {e}"))?;

        Ok(path)
    }

    /// Try to read a response for a given request ID.
    pub fn read_response(&self, request_id: &str) -> Result<Option<GrokResponse>, String> {
        let responses_dir = self.grok_dir.join("responses");

        if !responses_dir.exists() {
            return Ok(None);
        }

        for entry in std::fs::read_dir(&responses_dir)
            .map_err(|e| format!("Failed to read responses dir: {e}"))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(resp) = serde_json::from_str::<GrokResponse>(&content) {
                        if resp.request_id == request_id {
                            return Ok(Some(resp));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Wait for a response to a specific request ID.
    /// Polls the responses directory until the response appears or timeout is reached.
    pub fn wait_for_response(
        &self,
        request_id: &str,
        timeout_secs: u64,
    ) -> Result<GrokResponse, String> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        loop {
            if let Some(response) = self.read_response(request_id)? {
                return Ok(response);
            }

            if start.elapsed() > timeout {
                return Err(format!(
                    "Timed out waiting for Grok response for request {} after {} seconds",
                    request_id, timeout_secs
                ));
            }

            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    /// Write a request and immediately wait for the corresponding response.
    /// This is the main entry point for synchronous Grok-native execution.
    pub fn execute_request(&self, request: &GrokRequest) -> Result<GrokResponse, String> {
        self.write_request(request)?;
        self.wait_for_response(&request.metadata.request_id, request.timeout_seconds)
    }

    /// List all pending (unfulfilled) requests.
    /// Useful for a Grok session to discover work to do.
    pub fn list_pending_requests(&self) -> Result<Vec<GrokRequest>, String> {
        let requests_dir = self.grok_dir.join("requests");
        let mut pending = Vec::new();

        if !requests_dir.exists() {
            return Ok(pending);
        }

        for entry in std::fs::read_dir(&requests_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(req) = serde_json::from_str::<GrokRequest>(&content) {
                        // Check if response already exists
                        if self.read_response(&req.metadata.request_id)?.is_none() {
                            pending.push(req);
                        }
                    }
                }
            }
        }

        Ok(pending)
    }

    /// Generate a ready-to-use prompt for the current Grok session to fulfill a request.
    /// This is the "direct fulfillment" helper for when you're running inside Grok.
    pub fn generate_fulfillment_prompt(&self, request: &GrokRequest) -> String {
        format!(
            "You are now acting as the Grok-native executor for the Harness orchestrator.\n\n\
             **Phase:** {:?}\n\
             **Project:** {}\n\
             **Goal:** {}\n\n\
             **Full Prompt / Task:**\n{}\n\n\
             **Instructions:**\n\
             - Use your full capabilities (subagents, plan_mode if needed, GitHub MCP tools, rich SCL recording with relates_to, todo tracking, etc.).\n\
             - When complete, write a GrokResponse JSON to:\n  .harness/grok/responses/{}.json\n\n\
             Use this schema for the response:\n\
             {{\n\
               \"request_id\": \"{}\",\n\
               \"phase\": \"...\",\n\
               \"success\": true/false,\n\
               \"output\": \"the main result (spec.md content, build status, evaluation, etc.)\",\n\
               \"error\": null or error message,\n\
               \"trace\": {{ ... rich execution details ... }},\n\
               \"duration_ms\": ...,\n\
               \"artifacts_written\": []\n\
             }}\n\n\
             After writing the response file, the `harness` process will automatically continue.",
            request.phase,
            request.metadata.project_name,
            request.metadata.goal,
            request.prompt,
            request.metadata.request_id,
            request.metadata.request_id
        )
    }

    /// Easy helper for the current Grok session to submit a completed response.
    /// This writes the GrokResponse JSON so the waiting `harness` process can pick it up.
    pub fn submit_response(&self, response: &GrokResponse) -> Result<PathBuf, String> {
        self.ensure_directories()?;

        let path = self.grok_dir.join("responses").join(format!(
            "{}.json",
            response.request_id
        ));

        let content = serde_json::to_string_pretty(response)
            .map_err(|e| format!("Failed to serialize GrokResponse: {e}"))?;

        std::fs::write(&path, content)
            .map_err(|e| format!("Failed to write response file: {e}"))?;

        Ok(path)
    }

    // === Master TODO Management ===

    pub fn grok_dir(&self) -> &std::path::Path {
        &self.grok_dir
    }

    fn master_todo_path(&self, workflow_run_id: &str) -> std::path::PathBuf {
        self.grok_dir.join("master_todos").join(format!("{}.json", workflow_run_id))
    }

    /// Get or create a Master TODO for a workflow run.
    pub fn get_or_create_master_todo(
        &self,
        workflow_name: &str,
        run_id: &str,
    ) -> Result<crate::grok::MasterTodo, String> {
        self.ensure_directories()?;
        std::fs::create_dir_all(self.grok_dir.join("master_todos"))
            .map_err(|e| format!("Failed to create master_todos directory: {e}"))?;

        let path = self.master_todo_path(run_id);

        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read Master TODO: {e}"))?;
            let todo: crate::grok::MasterTodo = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse Master TODO: {e}"))?;
            return Ok(todo);
        }

        let todo = crate::grok::MasterTodo::new(workflow_name.to_string(), run_id.to_string());
        self.save_master_todo(&todo)?;
        Ok(todo)
    }

    pub fn save_master_todo(&self, todo: &crate::grok::MasterTodo) -> Result<(), String> {
        let path = self.master_todo_path(&todo.workflow_run_id);
        let content = serde_json::to_string_pretty(todo)
            .map_err(|e| format!("Failed to serialize Master TODO: {e}"))?;
        std::fs::write(path, content).map_err(|e| format!("Failed to write Master TODO: {e}"))?;
        Ok(())
    }

    /// Load the most recently modified Master TODO (by file mtime).
    /// This is the main way the current Grok session interacts with an active workflow.
    pub fn load_latest_master_todo(&self) -> Result<Option<crate::grok::MasterTodo>, String> {
        let master_todos_dir = self.grok_dir.join("master_todos");
        if !master_todos_dir.exists() {
            return Ok(None);
        }

        let mut entries: Vec<_> = match std::fs::read_dir(&master_todos_dir) {
            Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
            Err(_) => return Ok(None),
        };

        if entries.is_empty() {
            return Ok(None);
        }

        // Sort by modified time, most recent first
        entries.sort_by_key(|entry| {
            std::fs::metadata(entry.path())
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });
        entries.reverse();

        for entry in entries {
            let path = entry.path();
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(todo) = serde_json::from_str::<crate::grok::MasterTodo>(&content) {
                    return Ok(Some(todo));
                }
            }
        }

        Ok(None)
    }

    /// Load a specific Master TODO by its run ID.
    pub fn load_master_todo(&self, run_id: &str) -> Result<Option<crate::grok::MasterTodo>, String> {
        let path = self.grok_dir.join("master_todos").join(format!("{}.json", run_id));
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read Master TODO {}: {}", run_id, e))?;

        let todo: crate::grok::MasterTodo = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse Master TODO {}: {}", run_id, e))?;

        Ok(Some(todo))
    }

    /// Update the status of a task in the latest Master TODO and persist it.
    pub fn update_master_todo_status(
        &self,
        task_id: &str,
        status: crate::grok::TodoStatus,
    ) -> Result<(), String> {
        let mut todo = self
            .load_latest_master_todo()?
            .ok_or_else(|| "No active Master TODO found. Run a Grok-native workflow first.".to_string())?;

        todo.update_status(task_id, status)?;
        self.save_master_todo(&todo)?;
        Ok(())
    }

    /// Add a note to a task (or "orchestrator") in the latest Master TODO.
    pub fn add_master_todo_note(&self, target: &str, note: &str) -> Result<(), String> {
        let mut todo = self
            .load_latest_master_todo()?
            .ok_or_else(|| "No active Master TODO found.".to_string())?;

        if target == "orchestrator" || target == "orchestrator-supervisor" {
            todo.add_orchestrator_note(note.to_string());
        } else {
            todo.add_note(target, note.to_string())?;
        }

        self.save_master_todo(&todo)?;
        Ok(())
    }

    /// Records a high-level orchestration decision (DeviationAnalysis + OverrideProposal)
    /// into the latest Master TODO. This is the key persistence step for making
    /// supervisor-orchestrator decisions machine-native and auditable.
    pub fn record_orchestration_decision(
        &self,
        record: OrchestrationDecisionRecord,
    ) -> Result<(), String> {
        let mut todo = self
            .load_latest_master_todo()?
            .ok_or_else(|| {
                "No active Master TODO found. Cannot record orchestration decision.".to_string()
            })?;

        todo.decisions.push(record);
        todo.updated_at = chrono::Utc::now();

        self.save_master_todo(&todo)?;
        Ok(())
    }

    /// Builds a `SupervisorContext` from the Master TODO.
    /// This is the main method used to gather rich supervisor context for
    /// context-aware subagent prompts and intelligent orchestration decisions.
    pub fn build_supervisor_context(
        &self,
        workflow_run_id: Option<&str>,
    ) -> Result<Option<SupervisorContext>, String> {
        let todo = match workflow_run_id {
            Some(id) => self.load_master_todo(id)?,
            None => self.load_latest_master_todo()?,
        };

        let todo = match todo {
            Some(t) => t,
            None => return Ok(None),
        };

        // Find the most relevant supervisor task (most recently added)
        let supervisor_task = todo
            .tasks
            .values()
            .filter(|t| {
                if let Some(name) = &t.assigned_to {
                    name.contains("architect")
                        || name.contains("supervisor")
                        || name.contains("orchestrator")
                } else {
                    false
                }
            })
            .last();   // Last in iteration order is usually the most recently inserted

        let supervisor_name = supervisor_task
            .and_then(|t| t.assigned_to.clone())
            .unwrap_or_else(|| "unknown-supervisor".to_string());

        // Try to load the actual AgentDef for richer metadata
        let agent_def = agents::load(&supervisor_name).ok();

        let authority = agent_def
            .as_ref()
            .map(|a| a.authority.clone())
            .unwrap_or_else(|| vec![
                SupervisorAuthority::Read,
                SupervisorAuthority::Rewrite,
                SupervisorAuthority::Replan,
                SupervisorAuthority::Spawn,
            ]);

        // Improved standing orders extraction
        let mut standing_orders: Vec<String> = Vec::new();
        let mut key_decisions: Vec<String> = Vec::new();

        for note in &todo.orchestrator_notes {
            let lower = note.to_lowercase();
            if lower.contains("standing order")
                || lower.contains("supervisor directive")
                || lower.contains("i will always")
                || lower.contains("as supervisor, i require")
            {
                standing_orders.push(note.clone());
            } else {
                key_decisions.push(note.clone());
            }
        }

        // If the AgentDef has standing orders declared, prefer those
        if let Some(def) = &agent_def {
            // Future: support a dedicated `standing_orders` field on AgentDef
            // For now we just use the description as additional context
            if let Some(desc) = &def.description {
                if !standing_orders.iter().any(|s| s.contains(desc)) {
                    standing_orders.push(format!("Supervisor description: {}", desc));
                }
            }
        }

        // Smarter filtering of recent/high-value decisions
        let mut recent_deviation_analyses = Vec::new();
        let mut active_override_proposals = Vec::new();

        for record in todo.decisions.iter().rev() {
            // Prioritize high-confidence analyses
            if let Some(analysis) = &record.deviation_analysis {
                if analysis.confidence >= 0.65 {
                    recent_deviation_analyses.push(analysis.clone());
                }
            }

            if let Some(proposal) = &record.override_proposal {
                if proposal.status == OverrideStatus::Pending
                    || proposal.status == OverrideStatus::Approved
                {
                    active_override_proposals.push(proposal.clone());
                }
            }

            if recent_deviation_analyses.len() >= 6 && active_override_proposals.len() >= 4 {
                break;
            }
        }

        // Limit key decisions for context size
        let key_decisions = key_decisions.into_iter().rev().take(8).collect();

        let context = SupervisorContext {
            supervisor_name,
            standing_orders,
            recent_deviation_analyses,
            active_override_proposals,
            key_decisions,
            authority,
        };

        Ok(Some(context))
    }

    /// Generates a fulfillment prompt that is automatically enriched with
    /// supervisor context (standing orders, recent deviation analyses, override proposals, etc.)
    /// when a supervisor is actively driving the workflow.
    ///
    /// If no supervisor is detected, it falls back to the basic prompt.
    pub fn generate_supervised_fulfillment_prompt(
        &self,
        request: &GrokRequest,
    ) -> Result<String, String> {
        let base_prompt = self.generate_fulfillment_prompt(request);

        let Some(supervisor_ctx) = self.build_supervisor_context(None)? else {
            return Ok(base_prompt);
        };

        let mut enriched = base_prompt;

        // Append supervisor context in a clear, structured way
        enriched.push_str("\n\n--- SUPERVISOR CONTEXT (Treat as high-priority constraints) ---\n");

        if !supervisor_ctx.standing_orders.is_empty() {
            enriched.push_str("\n**Standing Orders:**\n");
            for order in &supervisor_ctx.standing_orders {
                enriched.push_str(&format!("- {}\n", order));
            }
        }

        if !supervisor_ctx.key_decisions.is_empty() {
            enriched.push_str("\n**Recent Supervisor Decisions:**\n");
            for decision in supervisor_ctx.key_decisions.iter().rev().take(5) {
                enriched.push_str(&format!("- {}\n", decision));
            }
        }

        if !supervisor_ctx.recent_deviation_analyses.is_empty() {
            enriched.push_str("\n**Recent Deviation Analyses:**\n");
            for analysis in supervisor_ctx.recent_deviation_analyses.iter().rev().take(3) {
                enriched.push_str(&format!(
                    "- Leverage of change: {:.2}, Strategic alignment: {:.2} ({} reasons)\n",
                    analysis.leverage_of_changing_course,
                    analysis.strategic_alignment,
                    analysis.reasons.len()
                ));
            }
        }

        if !supervisor_ctx.active_override_proposals.is_empty() {
            enriched.push_str("\n**Active Override Proposals:**\n");
            for proposal in supervisor_ctx.active_override_proposals.iter().rev().take(3) {
                enriched.push_str(&format!(
                    "- Action: {:?} (confidence: {:.2}, status: {:?})\n",
                    proposal.proposed_action, proposal.confidence, proposal.status
                ));
            }
        }

        if !supervisor_ctx.authority.is_empty() {
            enriched.push_str("\n**Supervisor Authority:** ");
            let auths: Vec<_> = supervisor_ctx
                .authority
                .iter()
                .map(|a| format!("{:?}", a))
                .collect();
            enriched.push_str(&auths.join(", "));
            enriched.push('\n');
        }

        Ok(enriched)
    }
}

impl crate::grok::protocol::Phase {
    pub fn as_str(&self) -> &'static str {
        match self {
            crate::grok::protocol::Phase::Planner => "planner",
            crate::grok::protocol::Phase::Builder => "builder",
            crate::grok::protocol::Phase::Evaluator => "evaluator",
        }
    }
}

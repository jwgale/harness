//! Grok Build native backend.
//!
//! This is the real entry point for Grok-native execution when running inside
//! a Grok Build TUI session.

use chrono::Utc;
use super::AgentBackend;
use super::StreamingProcess;
use crate::artifacts;
use crate::grok::{GrokCoordinator, GrokRequest, MasterTodo, Phase};

pub struct GrokBackend {
    coordinator: GrokCoordinator,
}

impl GrokBackend {
    pub fn new() -> Self {
        let harness_dir = artifacts::harness_dir();
        Self {
            coordinator: GrokCoordinator::new(&harness_dir),
        }
    }

    fn is_running_in_grok(&self) -> bool {
        // Multiple signals that we're inside a Grok Build TUI session
        std::env::var("GROK_SESSION_ID").is_ok()
            || std::env::var("GROK_TUI").is_ok()
            || std::env::current_exe()
                .map(|p| p.to_string_lossy().to_lowercase().contains("grok"))
                .unwrap_or(false)
            || std::path::Path::new("/home/jason/.grok").exists()  // common dev location
            || std::path::Path::new(".grok").exists()
    }
}

impl AgentBackend for GrokBackend {
    fn name(&self) -> &'static str {
        "grok"
    }

    fn is_available(&self) -> Result<bool, String> {
        Ok(self.is_running_in_grok())
    }

    fn description(&self) -> String {
        "Grok Build (native — subagents • plan_mode • GitHub MCP • rich SCL)".to_string()
    }

    fn run_oneshot(&self, _model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
        if !self.is_running_in_grok() {
            return Err(self.not_in_grok_message());
        }

        // Smart phase detection for oneshot (used by both planner and evaluator)
        let phase = if crate::artifacts::artifact_exists("spec.md") {
            Phase::Evaluator
        } else {
            Phase::Planner
        };

        let mut request = GrokRequest::new(
            phase,
            "current-project".to_string(),
            "See .harness/goal.md".to_string(),
            prompt.to_string(),
            timeout_secs,
        );

        let _ = request.enrich_from_artifacts();
        self.coordinator.write_request(&request)?;

        // When running inside Grok, we don't block the binary forever.
        // Instead, we provide the current Grok session (this one) with everything needed to fulfill it directly.
        let fulfillment_prompt = self.coordinator.generate_fulfillment_prompt(&request);

        Err(format!(
            "Grok-native request prepared for direct fulfillment.\n\n\
             Request ID: {}\n\n\
             --- Fulfillment Prompt for Current Grok Session ---\n\n{}\n\n\
             After you generate the result, write a GrokResponse JSON to:\n.harness/grok/responses/{}.json\n\n\
             The harness process can then continue automatically.",
            request.metadata.request_id,
            fulfillment_prompt,
            request.metadata.request_id
        ))
    }

    fn run_builder(
        &self,
        _model: &str,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<String, String> {
        if !self.is_running_in_grok() {
            return Err(self.not_in_grok_message());
        }

        let mut request = GrokRequest::new(
            Phase::Builder,
            "current-project".to_string(),
            "See .harness/goal.md and .harness/spec.md".to_string(),
            prompt.to_string(),
            timeout_secs,
        );

        let _ = request.enrich_from_artifacts();

        self.coordinator.write_request(&request)?;

        let fulfillment_prompt = self.coordinator.generate_fulfillment_prompt(&request);

        Err(format!(
            "Grok-native builder request prepared for direct fulfillment.\n\n\
             Request ID: {}\n\n\
             --- Fulfillment Prompt for Current Grok Session ---\n\n{}\n\n\
             After you complete the build, write the response to .harness/grok/responses/{}.json",
            request.metadata.request_id,
            fulfillment_prompt,
            request.metadata.request_id
        ))
    }

    fn run_oneshot_streaming(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<StreamingProcess, String> {
        Err("Streaming for Grok backend is not yet implemented. Use non-streaming mode for now.".to_string())
    }

    fn run_builder_streaming(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<StreamingProcess, String> {
        self.run_oneshot_streaming(_model, _prompt, _timeout_secs)
    }
}

// === Grok-Native Multi-Agent Workflow Support (Prototyping) ===

impl GrokBackend {
    /// Execute a full multi-agent workflow using Grok subagents.
    /// This is the entry point for the future Grok-native execution model.
    ///
    /// For now, this writes enriched subagent requests for each step and relies on
    /// the current Grok session for direct fulfillment.
    pub fn execute_workflow(
        &self,
        workflow: &crate::workflows::WorkflowDef,
        _config: &crate::config::Config,
    ) -> Result<(), String> {
        if !self.is_running_in_grok() {
            return Err(self.not_in_grok_message());
        }

        self.coordinator.ensure_directories()?;

        // Generate a unique run ID for this workflow execution
        let run_id = format!("{}-{}", workflow.name, Utc::now().timestamp());

        // Initialize the Master TODO for this workflow run (core of the Orchestrator pattern)
        let mut master_todo = self.coordinator.get_or_create_master_todo(&workflow.name, &run_id)?;

        master_todo.add_orchestrator_note(format!(
            "Workflow execution started for '{}'. {} steps declared. Using Master TODO orchestration.",
            workflow.name,
            workflow.steps.len()
        ));

        // Create a top-level orchestrator task
        master_todo.add_task(
            "orchestrator".to_string(),
            format!("Orchestrate entire workflow: {}", workflow.name),
            Some("orchestrator-supervisor".to_string()),
            vec![],
        );

        // Create tasks for every step (subagent-per-step model)
        for (i, step) in workflow.steps.iter().enumerate() {
            let task_id = format!("step-{}", i + 1);
            let desc = format!("Subagent task: {} ({})", step.agent, step.prompt.as_deref().unwrap_or("default prompt"));

            let mut depends_on = vec!["orchestrator".to_string()];
            if i > 0 {
                depends_on.push(format!("step-{}", i));
            }

            master_todo.add_task(
                task_id,
                desc,
                Some(step.agent.clone()),
                depends_on,
            );
        }

        self.coordinator.save_master_todo(&master_todo)?;

        // Write the main workflow request + per-step subagent requests
        let workflow_request = GrokRequest::new(
            crate::grok::protocol::Phase::Builder,
            workflow.name.clone(),
            format!("Execute multi-agent workflow: {}", workflow.name),
            format!(
                "You are the Orchestrator for workflow '{}'. Maintain the Master TODO at .harness/grok/master_todos/{}.json.\n\n\
                 Use subagents for each declared step. Supervisor agents have rewrite/replan authority.\n\n\
                 Workflow definition:\n{:#?}",
                workflow.name, run_id, workflow
            ),
            3600,
        );

        self.coordinator.write_request(&workflow_request)?;

        for (i, step) in workflow.steps.iter().enumerate() {
            let mut step_request = GrokRequest::new(
                crate::grok::protocol::Phase::Builder,
                workflow.name.clone(),
                format!("Subagent step {}: {}", i + 1, step.agent),
                step.prompt.clone().unwrap_or_else(|| format!("Execute step for agent '{}'", step.agent)),
                1800,
            );

            let _ = step_request.enrich_from_artifacts();
            step_request.metadata.context.insert("workflow_step".to_string(), step.agent.clone());
            step_request.metadata.context.insert("master_todo_run_id".to_string(), run_id.clone());
            step_request.metadata.context.insert("task_id".to_string(), format!("step-{}", i + 1));

            self.coordinator.write_request(&step_request)?;
        }

        println!(
            "\nGrok-native workflow '{}' started (run_id: {}).",
            workflow.name, run_id
        );
        println!("Master TODO initialized at .harness/grok/master_todos/{}.json", run_id);
        println!("Subagent requests written to .harness/grok/requests/.");
        println!("\nThe current Grok session should now act as the Orchestrator / Supervisor using the Master TODO.");

        Ok(())
    }
}

impl GrokBackend {
    fn not_in_grok_message(&self) -> String {
        "Grok-native backend requires running inside a Grok Build TUI session.\n\n\
         Run `grok` in this directory and use `--backend grok` or auto-detection."
            .to_string()
    }
}

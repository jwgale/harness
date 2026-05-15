//! Grok-native commands (prototyping phase).
//!
//! These commands are designed to exercise the future Grok-native multi-agent
//! workflow system, especially supervisor agents, subagent-per-step execution,
//! and direct fulfillment from within a Grok Build session.

use crate::backend::GrokBackend;
use crate::config::Config;
use crate::grok::{GrokCoordinator, OrchestrationDecisionRecord};
use crate::workflows;
use chrono::Utc;

/// Execute a workflow using Grok-native subagent execution.
///
/// This is the main entry point for testing the new Grok-native multi-agent model.
/// When running from inside a Grok Build TUI session, it will write rich subagent
/// requests that the current session can fulfill using subagents, plan_mode, etc.
pub fn execute_workflow(name: &str) -> Result<(), String> {
    let wf = workflows::load(name)?;

    // Load minimal config (we may want to make this richer later)
    let config = if crate::artifacts::artifact_exists("config.json") {
        Config::load(&crate::artifacts::harness_dir())?
    } else {
        // Fallback config for testing
        Config {
            backend: "grok".to_string(),
            model: "best-available".to_string(),
            project_name: wf.name.clone(),
            max_eval_rounds: wf.max_rounds.unwrap_or(3),
            builder_timeout_seconds: 1800,
            evaluator_timeout_seconds: 600,
            evaluator_strategy: "default".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    };

    let backend = GrokBackend::new();
    backend.execute_workflow(&wf, &config)?;

    println!("\nGrok-native workflow '{}' has been prepared.", wf.name);
    println!("Rich subagent requests have been written to .harness/grok/requests/.");
    println!("A Master TODO has been initialized for this workflow run.\n");

    println!("Next steps from this Grok session:");
    println!("  harness grok pending                 # See all waiting requests");
    println!("  harness grok todo status             # View the Master TODO");
    println!("  harness grok fulfill                 # Get fulfillment prompt for the next subagent task");
    println!("  harness grok fulfill <request-id>    # Get prompt for a specific request");
    println!("\nAfter fulfilling work, update the Master TODO and write the response JSON.");

    Ok(())
}

/// List all pending Grok requests that have not yet been fulfilled.
pub fn list_pending() -> Result<(), String> {
    let coordinator = GrokCoordinator::new(&crate::artifacts::harness_dir());
    let pending = coordinator.list_pending_requests()?;

    if pending.is_empty() {
        println!("No pending Grok requests.");
        return Ok(());
    }

    println!("Pending Grok requests ({}):\n", pending.len());

    for req in pending {
        let phase = format!("{:?}", req.phase);
        let age = req.metadata.created_at.clone();

        println!("  [{}] {}", phase, req.metadata.request_id);
        println!("    Project: {}", req.metadata.project_name);
        println!("    Created: {}", age);
        if let Some(step) = req.metadata.context.get("workflow_step") {
            println!("    Workflow Step: {}", step);
        }
        println!();
    }

    println!("Use `harness grok fulfill` to get a ready-to-use prompt for the next request.");

    Ok(())
}

/// Get the next (or specific) pending request and print a high-quality fulfillment prompt
/// that the current Grok session can use to complete the work.
pub fn fulfill(request_id: Option<&str>) -> Result<(), String> {
    let coordinator = GrokCoordinator::new(&crate::artifacts::harness_dir());
    let mut pending = coordinator.list_pending_requests()?;

    if pending.is_empty() {
        println!("No pending requests to fulfill.");
        return Ok(());
    }

    let request = if let Some(id) = request_id {
        pending
            .into_iter()
            .find(|r| r.metadata.request_id == id)
            .ok_or_else(|| format!("No pending request found with ID: {}", id))?
    } else {
        // Just take the first one (could sort by created_at later)
        pending.remove(0)
    };

    println!("=== Fulfillment Request ===\n");
    println!("Request ID: {}", request.metadata.request_id);
    println!("Phase:      {:?}", request.phase);
    println!("Project:    {}", request.metadata.project_name);
    println!();

    let prompt = coordinator.generate_supervised_fulfillment_prompt(&request)?;
    println!("{}", prompt);

    println!("\n--- After you complete the work ---");
    println!("Write the GrokResponse JSON to:");
    println!("  .harness/grok/responses/{}.json", request.metadata.request_id);
    println!("\nYou can use the coordinator's submit_response() helper from within Grok.");

    Ok(())
}

// === Master TODO Commands (for use from within a Grok session) ===

fn get_coordinator() -> crate::grok::GrokCoordinator {
    crate::grok::GrokCoordinator::new(&crate::artifacts::harness_dir())
}

pub fn todo_status() -> Result<(), String> {
    let coordinator = get_coordinator();

    match coordinator.load_latest_master_todo()? {
        Some(todo) => {
            println!("=== Master TODO for workflow '{}' ===", todo.workflow_name);
            println!("Run ID: {}", todo.workflow_run_id);
            println!("Overall Status: {:?}\n", todo.overall_status);

            for (id, task) in &todo.tasks {
                let assigned = task.assigned_to.as_deref().unwrap_or("unassigned");
                println!(
                    "  [{}] {} — {:?} (assigned: {})",
                    id, task.description, task.status, assigned
                );
                if let Some(notes) = &task.notes {
                    println!("      Notes: {}", notes.replace('\n', "\n      "));
                }
            }

            if !todo.orchestrator_notes.is_empty() {
                println!("\nOrchestrator Notes:");
                for note in &todo.orchestrator_notes {
                    println!("  {}", note);
                }
            }
        }
        None => {
            println!("No active Master TODO found. Run `harness grok execute-workflow <name>` first.");
        }
    }

    Ok(())
}

pub fn todo_next() -> Result<(), String> {
    let coordinator = get_coordinator();

    match coordinator.load_latest_master_todo()? {
        Some(todo) => {
            if let Some(next_task) = todo.get_next_actionable_task() {
                println!("Next actionable task from the Master TODO:");
                println!("  ID:          {}", next_task.id);
                println!("  Description: {}", next_task.description);
                println!("  Assigned to: {:?}", next_task.assigned_to);
                println!("  Status:      {:?}", next_task.status);
                if let Some(notes) = &next_task.notes {
                    println!("  Notes:       {}", notes.replace('\n', "\n                 "));
                }
            } else {
                println!("No actionable tasks right now.");
                println!("(All pending tasks have unmet dependencies, or the workflow is done/blocked.)");
            }
        }
        None => {
            println!("No active Master TODO found.");
        }
    }

    Ok(())
}

pub fn todo_update(task_id: &str, status_str: &str) -> Result<(), String> {
    let status = match status_str.to_lowercase().as_str() {
        "pending" => crate::grok::TodoStatus::Pending,
        "in_progress" | "inprogress" | "progress" => crate::grok::TodoStatus::InProgress,
        "done" | "complete" => crate::grok::TodoStatus::Done,
        "blocked" => crate::grok::TodoStatus::Blocked,
        "failed" => crate::grok::TodoStatus::Failed,
        _ => return Err(format!(
            "Invalid status '{}'. Valid options: pending, in_progress, done, blocked, failed",
            status_str
        )),
    };

    let coordinator = get_coordinator();
    coordinator.update_master_todo_status(task_id, status)?;

    println!("Updated task '{}' → {:?}", task_id, status_str);
    Ok(())
}

pub fn todo_note(target: &str, note: &str) -> Result<(), String> {
    let coordinator = get_coordinator();
    coordinator.add_master_todo_note(target, note)?;

    println!("Added note to '{}': {}", target, note);
    Ok(())
}

/// Run one step of orchestration: load the latest Master TODO and ask the
/// WorkflowOrchestrator what the current Grok session should do next.
///
/// This is extremely useful when you are inside a Grok Build session and want
/// intelligent guidance on how to drive a complex multi-agent workflow.
pub fn orchestrate() -> Result<(), String> {
    let coordinator = get_coordinator();

    let master_todo = match coordinator.load_latest_master_todo()? {
        Some(todo) => todo,
        None => {
            println!("No active Master TODO found.");
            println!("Run `harness grok execute-workflow <name>` first to start a Grok-native workflow.");
            return Ok(());
        }
    };

    let orchestrator = crate::grok::WorkflowOrchestrator::new(
        master_todo.workflow_name.clone(),
        master_todo.workflow_run_id.clone(),
        master_todo.clone(),
    );

    println!("=== Grok Workflow Orchestrator ===\n");
    println!("Workflow: {}", master_todo.workflow_name);
    println!("Run ID:   {}", master_todo.workflow_run_id);
    println!("Overall Status: {:?}\n", master_todo.overall_status);

    // Drift evaluation
    let drift = orchestrator.evaluate_drift();
    if drift.has_drift {
        println!("⚠️  Drift Detected (Severity: {:?})", drift.severity);
        for reason in &drift.reasons {
            println!("   - {}", reason);
        }
        println!();
    } else {
        println!("✓ No significant drift detected at this time.\n");
    }

    // Use the new intelligent advisor
    let advice = orchestrator.advise();

    match &advice.recommended_action {
        crate::grok::OrchestratorDecision::AssignTask { task_id, description, suggested_agent } => {
            println!("Recommended Action: Assign Task");
            println!("  Task ID:      {}", task_id);
            println!("  Description:  {}", description);
            if let Some(agent) = suggested_agent {
                println!("  Suggested Agent: {}", agent);
            }
            println!("\nNext step: Use `harness grok fulfill` (or the corresponding request) to execute this task.");
        }
        crate::grok::OrchestratorDecision::EnterPlanMode { reason } => {
            println!("Recommended Action: Enter Plan Mode");
            println!("  Reason: {}", reason);

            let plan_prompt = orchestrator.generate_plan_mode_prompt(reason);
            println!("\n--- Ready-to-Use Plan Mode Prompt ---\n");
            println!("{}", plan_prompt);
            println!("\nYou can now enter plan_mode with the context above. After exiting plan mode,");
            println!("update the Master TODO with your decisions and continue orchestrating.");
        }
        crate::grok::OrchestratorDecision::SpawnSpecialist { specialization, parent_task } => {
            println!("Recommended Action: Spawn Specialist Subagent");
            println!("  Specialization: {}", specialization);
            println!("  Parent Task:    {}", parent_task);
        }
        crate::grok::OrchestratorDecision::RewriteArtifact { artifact, reason } => {
            println!("Recommended Action: Rewrite Artifact");
            println!("  Artifact: {}", artifact);
            println!("  Reason:   {}", reason);
        }
        crate::grok::OrchestratorDecision::NoActionableWork => {
            println!("Orchestrator Assessment: No clear actionable work right now.");
            println!("The workflow may be complete, stuck, or waiting on external input.");
        }
    }

    // Show rich advisor output
    println!("\n--- Orchestrator Reasoning ---");
    println!("{}", advice.reasoning);

    if let Some(analysis) = &advice.deviation_analysis {
        println!("\n--- Deviation Analysis ---");
        println!("Leverage of Change: {:.2} | Strategic Alignment: {:.2} | Execution Risk: {:.2}", 
                 analysis.leverage_of_changing_course, 
                 analysis.strategic_alignment, 
                 analysis.execution_risk);

        if let Some(proposal) = &advice.suggested_override {
            println!("\n--- Suggested Override Proposal ---");
            println!("Action: {:?} | Confidence: {:.2}", proposal.proposed_action, proposal.confidence);
            println!("Status: {:?}", proposal.status);
        }
    }

    println!("\nYou can now act on the above recommendation using your Grok capabilities");
    println!("(subagents, plan_mode, rich SCL context, GitHub tools, etc.).");

    Ok(())
}

/// Directly trigger a replanning session.
///
/// This is a first-class command for the current Grok session to initiate
/// a high-quality replan when the orchestrator (or the user) decides that
/// the current approach has significant drift or is stuck.
pub fn replan() -> Result<(), String> {
    let coordinator = get_coordinator();

    let master_todo = match coordinator.load_latest_master_todo()? {
        Some(todo) => todo,
        None => {
            println!("No active Master TODO found. Cannot initiate replan.");
            println!("Run `harness grok execute-workflow <name>` first.");
            return Ok(());
        }
    };

    let mut orchestrator = crate::grok::WorkflowOrchestrator::new(
        master_todo.workflow_name.clone(),
        master_todo.workflow_run_id.clone(),
        master_todo.clone(),
    );

    let drift = orchestrator.evaluate_drift();
    let decision = orchestrator.decide_next_action();

    println!("=== Grok Replan Session ===\n");
    println!("Workflow: {}", master_todo.workflow_name);
    println!("Run ID:   {}", master_todo.workflow_run_id);

    if drift.has_drift {
        println!("Drift Severity: {:?}\n", drift.severity);
        for reason in &drift.reasons {
            println!("  - {}", reason);
        }
    }

    // Generate the rich plan mode prompt
    let reason = if drift.has_drift {
        drift.reasons.join("; ")
    } else {
        "User or orchestrator requested replanning session.".to_string()
    };

    let plan_prompt = orchestrator.generate_plan_mode_prompt(&reason);

    println!("\n--- High-Quality Plan Mode Prompt (Ready to Use) ---\n");
    println!("{}", plan_prompt);

    // Record that a replan was initiated in the Master TODO
    let note = format!(
        "Replan session initiated via `harness grok replan`. Drift severity: {:?}. Reason: {}",
        drift.severity, reason
    );
    let _ = coordinator.add_master_todo_note("orchestrator", &note);

    println!("\n--- Post-Replan Instructions ---");
    println!("1. Enter `plan_mode` using the prompt above.");
    println!("2. After exiting plan mode, update the Master TODO with your revised plan.");
    println!("3. Use `harness grok todo note orchestrator \"...\"` to record key decisions.");
    println!("4. Resume orchestration with `harness grok orchestrate` or continue fulfilling tasks.");

    Ok(())
}
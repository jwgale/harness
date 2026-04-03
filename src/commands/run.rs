use crate::agents::{self, AgentDef};
use crate::artifacts;
use crate::cli_backend::{self, Backend};
use crate::commands::{build, evaluate, plan};
use crate::config::Config;
use crate::commands::evaluate::Verdict;
use crate::notifications;
use crate::plugins::{PluginManager, HookPoint};
use crate::prompts;
use crate::scl_lifecycle;
use crate::workflows;
use std::io::{self, Write};

pub fn run(
    backend_override: Option<&str>,
    max_rounds: Option<u32>,
    pause_after_plan: bool,
    pause_after_eval: bool,
    no_tui: bool,
) -> Result<(), String> {
    // If TUI is enabled (default), delegate to the TUI module
    if !no_tui && !pause_after_plan && !pause_after_eval {
        return crate::tui::run_with_tui(backend_override, max_rounds);
    }
    run_plain(backend_override, max_rounds, pause_after_plan, pause_after_eval)
}

fn run_plain(
    backend_override: Option<&str>,
    max_rounds: Option<u32>,
    pause_after_plan: bool,
    pause_after_eval: bool,
) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let max = max_rounds.unwrap_or(config.max_eval_rounds);
    let pm = PluginManager::load();

    // Phase 1: Plan
    pm.fire(HookPoint::BeforePlan);
    plan::run(backend_override)?;
    pm.fire(HookPoint::AfterPlan);

    if pause_after_plan {
        println!("\n--- Plan complete. Review .harness/spec.md ---");
        println!("Press Enter to continue to build, or Ctrl+C to abort.");
        wait_for_enter();
    }

    // Phase 2+3: Build → Evaluate loop
    for round in 1..=max {
        println!("\n=== Round {round}/{max} ===\n");

        // Save run metadata
        save_run_metadata(round, backend_override.unwrap_or(&config.backend))?;

        // Build
        pm.fire(HookPoint::BeforeBuild);
        build::run(backend_override)?;
        pm.fire(HookPoint::AfterBuild);

        // Evaluate
        pm.fire(HookPoint::BeforeEvaluate);
        let verdict = evaluate::run(backend_override)?;
        pm.fire(HookPoint::AfterEvaluate);

        // Update run metadata with outcome
        update_run_outcome(round, &verdict)?;

        match verdict {
            Verdict::Pass => {
                println!("\n=== BUILD PASSED on round {round} ===");
                return Ok(());
            }
            Verdict::Revise => {
                if round == max {
                    println!("\n=== Max rounds ({max}) exhausted. Last verdict: REVISE ===");
                    println!("Check .harness/evaluation.md for details.");
                    return Err("Max revision rounds exhausted".to_string());
                }
                println!("\nVerdict: REVISE — looping back to builder with feedback.");

                if pause_after_eval {
                    println!("Press Enter to continue to next round, or Ctrl+C to abort.");
                    wait_for_enter();
                }
            }
            Verdict::Fail => {
                println!("\n=== BUILD FAILED on round {round} ===");
                println!("Check .harness/evaluation.md for details.");
                return Err("Evaluator returned FAIL verdict".to_string());
            }
        }
    }

    Ok(())
}

fn wait_for_enter() {
    let mut input = String::new();
    let _ = io::stdout().flush();
    let _ = io::stdin().read_line(&mut input);
}

fn save_run_metadata(round: u32, backend: &str) -> Result<(), String> {
    let run_num = artifacts::next_run_number();
    let metadata = serde_json::json!({
        "id": run_num,
        "round": round,
        "phase": "build+evaluate",
        "backend": backend,
        "started_at": chrono::Utc::now().to_rfc3339(),
        "ended_at": null,
        "outcome": null,
    });
    let json = serde_json::to_string_pretty(&metadata)
        .map_err(|e| format!("Failed to serialize run metadata: {e}"))?;
    artifacts::write_artifact(&format!("runs/run-{run_num:03}.json"), &json)
}

fn update_run_outcome(round: u32, verdict: &Verdict) -> Result<(), String> {
    // Find the run file for this round and update it
    let run_num = if round == 1 { 1 } else { round };
    let path = format!("runs/run-{run_num:03}.json");
    if let Ok(content) = artifacts::read_artifact(&path)
        && let Ok(mut meta) = serde_json::from_str::<serde_json::Value>(&content)
    {
        meta["ended_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
        meta["outcome"] = serde_json::json!(format!("{verdict:?}"));
        let json = serde_json::to_string_pretty(&meta)
            .map_err(|e| format!("Failed to serialize: {e}"))?;
        artifacts::write_artifact(&path, &json)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Multi-agent orchestration
// ---------------------------------------------------------------------------

/// Run a multi-agent workflow, either from --agents list or --workflow name.
pub fn run_multi_agent(
    backend_override: Option<&str>,
    max_rounds: Option<u32>,
    agents_csv: Option<&str>,
    workflow_name: Option<&str>,
) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let pm = PluginManager::load();

    let (agent_defs, max) = if let Some(wf_name) = workflow_name {
        let wf = workflows::load(wf_name)?;
        let max = wf.max_rounds.unwrap_or(max_rounds.unwrap_or(config.max_eval_rounds));
        let defs = resolve_workflow_agents(&wf)?;
        println!("Running workflow '{}' ({} steps)", wf.name, defs.len());
        (defs, max)
    } else if let Some(csv) = agents_csv {
        let names: Vec<&str> = csv.split(',').map(|s| s.trim()).collect();
        let defs = resolve_agent_names(&names)?;
        let max = max_rounds.unwrap_or(config.max_eval_rounds);
        println!("Running {} agents: {}", defs.len(), csv);
        (defs, max)
    } else {
        return Err("Either --agents or --workflow is required".to_string());
    };

    // Record to SCL
    let agent_names: Vec<&str> = agent_defs.iter().map(|a| a.name.as_str()).collect();
    scl_lifecycle::record_agent_run_start(&config.project_name, &agent_names);

    let mut last_verdict = None;

    for (step_idx, agent) in agent_defs.iter().enumerate() {
        let step_num = step_idx + 1;
        let backend = Backend::from_str(
            backend_override.unwrap_or(&agent.backend),
        )?;
        let model = agent.model.as_deref().unwrap_or(&config.model);
        let timeout = agent.timeout_seconds.unwrap_or(match agent.role.as_str() {
            "builder" => config.builder_timeout_seconds,
            _ => config.evaluator_timeout_seconds,
        });

        println!("\n--- Step {step_num}/{}: agent '{}' [{}] ---", agent_defs.len(), agent.name, agent.role);

        match agent.role.as_str() {
            "planner" => {
                pm.fire(HookPoint::BeforePlan);
                let prompt = build_agent_prompt(agent, &config)?;
                let output = cli_backend::run_oneshot(&backend, model, &prompt, timeout)?;
                artifacts::write_artifact("spec.md", &output)?;
                pm.fire(HookPoint::AfterPlan);
                scl_lifecycle::record_agent_step(&config.project_name, &agent.name, "planner", "completed");
                println!("Plan written to .harness/spec.md");
            }
            "builder" => {
                pm.fire(HookPoint::BeforeBuild);
                let prompt = build_agent_prompt(agent, &config)?;
                let output = cli_backend::run_builder(&backend, model, &prompt, timeout)?;
                artifacts::write_artifact("status.md", &output)?;
                pm.fire(HookPoint::AfterBuild);
                scl_lifecycle::record_agent_step(&config.project_name, &agent.name, "builder", "completed");
                println!("Build complete.");
            }
            "evaluator" => {
                pm.fire(HookPoint::BeforeEvaluate);
                let prompt = build_agent_prompt(agent, &config)?;
                let output = cli_backend::run_oneshot(&backend, model, &prompt, timeout)?;
                artifacts::write_artifact("evaluation.md", &output)?;
                let fb_round = artifacts::next_feedback_number();
                artifacts::write_artifact(&format!("feedback/round-{fb_round:03}.md"), &output)?;

                let verdict = evaluate::parse_verdict(&output);
                pm.fire(HookPoint::AfterEvaluate);
                scl_lifecycle::record_agent_step(
                    &config.project_name, &agent.name, "evaluator",
                    &format!("{verdict:?}"),
                );
                notifications::fire_eval_event(&verdict, &config.project_name, fb_round);

                println!("Verdict: {verdict:?}");
                last_verdict = Some(verdict.clone());

                match verdict {
                    Verdict::Pass => {
                        println!("\n=== PASSED (agent '{}') ===", agent.name);
                    }
                    Verdict::Fail => {
                        scl_lifecycle::record_agent_run_end(&config.project_name, &agent_names, "FAIL");
                        return Err(format!("Agent '{}' returned FAIL verdict", agent.name));
                    }
                    Verdict::Revise => {
                        println!("Revise — continuing to next step.");
                    }
                }
            }
            _ => {
                // Custom role: run as oneshot
                let prompt = build_agent_prompt(agent, &config)?;
                let output = cli_backend::run_oneshot(&backend, model, &prompt, timeout)?;
                // Write output to a named artifact
                let artifact_name = format!("agent-{}.md", agent.name);
                artifacts::write_artifact(&artifact_name, &output)?;
                scl_lifecycle::record_agent_step(&config.project_name, &agent.name, "custom", "completed");
                println!("Output written to .harness/{artifact_name}");
            }
        }
    }

    let final_status = match &last_verdict {
        Some(Verdict::Pass) => "PASS",
        Some(Verdict::Fail) => "FAIL",
        _ => "completed",
    };
    scl_lifecycle::record_agent_run_end(&config.project_name, &agent_names, final_status);

    let _ = max; // max_rounds available for future iterative multi-agent loops
    println!("\n=== Multi-agent run complete ({final_status}) ===");
    Ok(())
}

/// Build the prompt for an agent step.
fn build_agent_prompt(agent: &AgentDef, config: &Config) -> Result<String, String> {
    // If the agent has a custom prompt template, use it
    if let Some(template) = &agent.prompt_template {
        // Check if it's a file path
        let path = std::path::Path::new(template);
        if path.exists() {
            return std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read prompt template '{}': {e}", template));
        }
        // Otherwise use as inline prompt
        return Ok(template.clone());
    }

    // Default: use the built-in prompts for known roles
    match agent.role.as_str() {
        "planner" => {
            let goal = artifacts::read_artifact("goal.md")?;
            Ok(prompts::planner_prompt(&goal))
        }
        "builder" => prompts::builder_prompt(),
        "evaluator" => prompts::evaluator_prompt(),
        _ => {
            // Custom role with no template — provide minimal context
            let goal = artifacts::read_artifact("goal.md").unwrap_or_default();
            let spec = artifacts::read_artifact("spec.md").unwrap_or_default();
            Ok(format!(
                "You are a '{}' agent for project '{}'.\n\n## Goal\n{goal}\n\n## Spec\n{spec}\n",
                agent.role, config.project_name
            ))
        }
    }
}

/// Resolve a workflow's steps into agent definitions.
fn resolve_workflow_agents(wf: &workflows::WorkflowDef) -> Result<Vec<AgentDef>, String> {
    let mut defs = Vec::new();
    for step in &wf.steps {
        let mut agent = agents::load(&step.agent)?;
        // Apply step-level prompt override if present
        if step.prompt.is_some() {
            agent.prompt_template = step.prompt.clone();
        }
        defs.push(agent);
    }
    Ok(defs)
}

/// Resolve a comma-separated list of agent names into definitions.
fn resolve_agent_names(names: &[&str]) -> Result<Vec<AgentDef>, String> {
    let mut defs = Vec::new();
    for name in names {
        let agent = agents::load(name)?;
        defs.push(agent);
    }
    Ok(defs)
}

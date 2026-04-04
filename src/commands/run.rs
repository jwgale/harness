use crate::agents::{self, AgentDef};
use crate::artifacts;
use crate::cli_backend::{self, Backend};
use crate::commands::{build, evaluate, plan};
use crate::config::Config;
use crate::commands::evaluate::Verdict;
use crate::notifications;
use crate::plugins::{PluginManager, HookPoint};
use crate::progress::ProgressSender;
use crate::prompts;
use crate::scl_lifecycle;
use crate::workflows;
use std::io::{self, Write};
use std::sync::Arc;

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
    _max_rounds: Option<u32>,
    agents_csv: Option<&str>,
    workflow_name: Option<&str>,
    parallel: bool,
    no_tui: bool,
) -> Result<(), String> {
    // If TUI enabled, delegate to the TUI wrapper
    if !no_tui {
        return crate::tui::run_multi_agent_tui(
            backend_override, agents_csv, workflow_name, parallel,
        );
    }

    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let pm = PluginManager::load();

    // Connect to progress socket if bridge set the env var
    let progress = ProgressSender::connect_from_env().map(Arc::new);

    if let Some(wf_name) = workflow_name {
        let wf = workflows::load(wf_name)?;

        // Validate workflow before running
        let errors = workflows::validate(&wf);
        if !errors.is_empty() {
            println!("Workflow '{}' validation failed:\n", wf.name);
            for err in &errors {
                println!("  - {err}");
            }
            return Err(format!("Workflow has {} validation error(s)", errors.len()));
        }

        let groups = workflows::plan_execution(&wf);
        let agent_names: Vec<String> = wf.steps.iter().map(|s| s.agent.clone()).collect();
        let name_refs: Vec<&str> = agent_names.iter().map(|s| s.as_str()).collect();
        scl_lifecycle::record_agent_run_start(&config.project_name, &name_refs);

        println!("Running workflow '{}' ({} groups from {} steps)", wf.name, groups.len(), wf.steps.len());
        artifacts::clear_progress_log();
        let start_msg = format!("Workflow '{}' started ({} steps)", wf.name, wf.steps.len());
        artifacts::append_progress(&start_msg);
        if let Some(ref ps) = progress { ps.event(&start_msg); }

        let result = run_step_groups(&groups, backend_override, &config, &pm, progress.as_ref());

        let status = if result.is_ok() { "completed" } else { "FAIL" };
        scl_lifecycle::record_agent_run_end(&config.project_name, &name_refs, status);
        let done_msg = format!("Workflow '{}' {status}", wf.name);
        artifacts::append_progress(&done_msg);
        if let Some(ref ps) = progress { ps.done(&done_msg); }
        println!("\n=== Workflow '{}' {status} ===", wf.name);
        return result;
    }

    if let Some(csv) = agents_csv {
        let names: Vec<&str> = csv.split(',').map(|s| s.trim()).collect();
        // Validate all agents exist before starting
        let defs = resolve_agent_names(&names)?;
        scl_lifecycle::record_agent_run_start(&config.project_name, &names);

        if parallel {
            println!("Running {} agents in parallel: {csv}", defs.len());
            let steps: Vec<workflows::WorkflowStep> = defs.iter().map(|a| {
                workflows::WorkflowStep {
                    agent: a.name.clone(),
                    prompt: None,
                    output_artifact: None,
                    parallel: true,
                    loop_until: None,
                    max_rounds: None,
                }
            }).collect();
            let group = workflows::StepGroup::Parallel(steps);
            let result = run_step_groups(&[group], backend_override, &config, &pm, progress.as_ref());
            let status = if result.is_ok() { "completed" } else { "FAIL" };
            scl_lifecycle::record_agent_run_end(&config.project_name, &names, status);
            println!("\n=== Multi-agent run {status} ===");
            return result;
        }

        println!("Running {} agents: {csv}", defs.len());
        let steps: Vec<workflows::WorkflowStep> = defs.iter().map(|a| {
            workflows::WorkflowStep {
                agent: a.name.clone(),
                prompt: None,
                output_artifact: None,
                parallel: false,
                loop_until: None,
                max_rounds: None,
            }
        }).collect();
        let groups: Vec<workflows::StepGroup> = steps.into_iter()
            .map(workflows::StepGroup::Single)
            .collect();
        let result = run_step_groups(&groups, backend_override, &config, &pm, progress.as_ref());
        let status = if result.is_ok() { "completed" } else { "FAIL" };
        scl_lifecycle::record_agent_run_end(&config.project_name, &names, status);
        println!("\n=== Multi-agent run {status} ===");
        return result;
    }

    Err("Either --agents or --workflow is required".to_string())
}

/// Execute a sequence of step groups.
/// When `tui_tx` is provided, parallel steps use streaming and send agent-tagged output.
pub fn run_step_groups(
    groups: &[workflows::StepGroup],
    backend_override: Option<&str>,
    config: &Config,
    pm: &PluginManager,
    progress: Option<&Arc<ProgressSender>>,
) -> Result<(), String> {
    run_step_groups_with_tui(groups, backend_override, config, pm, None, progress)
}

/// Execute step groups with optional TUI streaming channel.
pub fn run_step_groups_with_tui(
    groups: &[workflows::StepGroup],
    backend_override: Option<&str>,
    config: &Config,
    pm: &PluginManager,
    tui_tx: Option<&std::sync::mpsc::Sender<crate::tui::TuiEvent>>,
    progress: Option<&Arc<ProgressSender>>,
) -> Result<(), String> {
    for (gi, group) in groups.iter().enumerate() {
        match group {
            workflows::StepGroup::Single(step) => {
                if let Some(tx) = tui_tx
                    && let Ok(agent) = agents::load(&step.agent)
                {
                    let _ = tx.send(crate::tui::TuiEvent::PhaseChange(
                        crate::tui::TuiPhase::AgentStep(agent.name.clone(), agent.role.clone()),
                        (gi + 1) as u32,
                    ));
                }
                println!("\n--- Group {}/{}: agent '{}' ---", gi + 1, groups.len(), step.agent);
                let msg = format!("Step {}/{}: agent '{}' started", gi + 1, groups.len(), step.agent);
                artifacts::append_progress(&msg);
                if let Some(ps) = progress { ps.event(&msg); }
                run_single_step_streaming(step, backend_override, config, pm, tui_tx, progress)?;
                let msg = format!("Step {}/{}: agent '{}' done", gi + 1, groups.len(), step.agent);
                artifacts::append_progress(&msg);
                if let Some(ps) = progress { ps.event(&msg); }
            }
            workflows::StepGroup::Parallel(steps) => {
                let names: Vec<&str> = steps.iter().map(|s| s.agent.as_str()).collect();
                if let Some(tx) = tui_tx {
                    let _ = tx.send(crate::tui::TuiEvent::PhaseChange(
                        crate::tui::TuiPhase::Parallel(names.iter().map(|s| s.to_string()).collect()),
                        (gi + 1) as u32,
                    ));
                }
                println!("\n--- Group {}/{}: parallel [{}] ---", gi + 1, groups.len(), names.join(", "));
                let msg = format!("Parallel batch: [{}]", names.join(", "));
                artifacts::append_progress(&msg);
                if let Some(ps) = progress { ps.event(&msg); }
                scl_lifecycle::record_parallel_start(&config.project_name, &names);
                run_parallel_steps(steps, backend_override, config, pm, tui_tx, progress)?;
                scl_lifecycle::record_parallel_end(&config.project_name, &names);
                let msg = format!("Parallel batch done: [{}]", names.join(", "));
                artifacts::append_progress(&msg);
                if let Some(ps) = progress { ps.event(&msg); }
            }
            workflows::StepGroup::Loop { body, evaluator, max_rounds } => {
                let body_names: Vec<&str> = body.iter().map(|s| s.agent.as_str()).collect();
                if let Some(tx) = tui_tx {
                    let _ = tx.send(crate::tui::TuiEvent::PhaseChange(
                        crate::tui::TuiPhase::Loop { round: 1, max: *max_rounds },
                        (gi + 1) as u32,
                    ));
                }
                println!(
                    "\n--- Group {}/{}: loop [{}] -> evaluator '{}' (max {max_rounds} rounds) ---",
                    gi + 1, groups.len(), body_names.join(", "), evaluator.agent
                );
                run_iterative_loop(body, evaluator, *max_rounds, backend_override, config, pm, tui_tx, progress)?;
            }
        }
    }
    Ok(())
}

/// Run a single step, streaming output to TUI when channel is provided.
fn run_single_step_streaming(
    step: &workflows::WorkflowStep,
    backend_override: Option<&str>,
    config: &Config,
    pm: &PluginManager,
    tui_tx: Option<&std::sync::mpsc::Sender<crate::tui::TuiEvent>>,
    progress: Option<&Arc<ProgressSender>>,
) -> Result<Option<Verdict>, String> {
    let mut agent = agents::load(&step.agent)?;
    if step.prompt.is_some() {
        agent.prompt_template = step.prompt.clone();
    }

    let backend = Backend::from_str(backend_override.unwrap_or(&agent.backend))?;
    let model = agent.model.as_deref().unwrap_or(&config.model);
    let timeout = agent.timeout_seconds.unwrap_or(match agent.role.as_str() {
        "builder" => config.builder_timeout_seconds,
        _ => config.evaluator_timeout_seconds,
    });

    let prompt = build_agent_prompt(&agent, config)?;

    // Run with streaming when TUI channel or progress sender is available
    let output = run_agent_invocation(&agent, &backend, model, &prompt, timeout, tui_tx, progress)?;

    match agent.role.as_str() {
        "planner" => {
            pm.fire(HookPoint::BeforePlan);
            artifacts::write_artifact("spec.md", &output)?;
            pm.fire(HookPoint::AfterPlan);
            scl_lifecycle::record_agent_step(&config.project_name, &agent.name, "planner", "completed");
            artifacts::append_progress(&format!("Planner '{}' done -- spec.md written", agent.name));
            println!("  Plan written to .harness/spec.md");
            Ok(None)
        }
        "builder" => {
            pm.fire(HookPoint::BeforeBuild);
            artifacts::write_artifact("status.md", &output)?;
            pm.fire(HookPoint::AfterBuild);
            scl_lifecycle::record_agent_step(&config.project_name, &agent.name, "builder", "completed");
            artifacts::append_progress(&format!("Builder '{}' done", agent.name));
            println!("  Build complete.");
            Ok(None)
        }
        "evaluator" => {
            pm.fire(HookPoint::BeforeEvaluate);
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
            artifacts::append_progress(&format!("Evaluator '{}' verdict: {verdict:?}", agent.name));
            println!("  Verdict: {verdict:?}");

            match &verdict {
                Verdict::Pass => println!("  PASSED"),
                Verdict::Fail => {
                    return Err(format!("Agent '{}' returned FAIL verdict", agent.name));
                }
                Verdict::Revise => println!("  Revise requested."),
            }
            Ok(Some(verdict))
        }
        _ => {
            let default_artifact = format!("agent-{}.md", agent.name);
            let artifact_name = step.output_artifact.as_deref()
                .unwrap_or(&default_artifact);
            artifacts::write_artifact(artifact_name, &output)?;
            scl_lifecycle::record_agent_step(&config.project_name, &agent.name, "custom", "completed");
            artifacts::append_progress(&format!("Agent '{}' done -- {artifact_name}", agent.name));
            println!("  Output written to .harness/{artifact_name}");
            Ok(None)
        }
    }
}

/// Run an agent invocation, using streaming when a TUI channel or progress sender is available.
fn run_agent_invocation(
    agent: &AgentDef,
    backend: &Backend,
    model: &str,
    prompt: &str,
    timeout: u64,
    tui_tx: Option<&std::sync::mpsc::Sender<crate::tui::TuiEvent>>,
    progress: Option<&Arc<ProgressSender>>,
) -> Result<String, String> {
    // Use streaming when TUI or progress socket is available
    if tui_tx.is_some() || progress.is_some() {
        let proc = if agent.role == "builder" {
            cli_backend::run_builder_streaming(backend, model, prompt, timeout)?
        } else {
            cli_backend::run_oneshot_streaming(backend, model, prompt, timeout)?
        };

        let name = agent.name.clone();
        loop {
            match proc.lines.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(line) => {
                    // Forward to TUI if available
                    if let Some(tx) = tui_tx {
                        let _ = tx.send(crate::tui::TuiEvent::AgentOutputLine(
                            name.clone(), line.clone(),
                        ));
                    }
                    // Forward to progress socket if available
                    if let Some(ps) = progress {
                        ps.stdout(&name, &line);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        proc.wait()
    } else if agent.role == "builder" {
        cli_backend::run_builder(backend, model, prompt, timeout)
    } else {
        cli_backend::run_oneshot(backend, model, prompt, timeout)
    }
}

/// Run steps in parallel using std::thread with artifact isolation and optional streaming.
fn run_parallel_steps(
    steps: &[workflows::WorkflowStep],
    backend_override: Option<&str>,
    config: &Config,
    pm: &PluginManager,
    tui_tx: Option<&std::sync::mpsc::Sender<crate::tui::TuiEvent>>,
    progress: Option<&Arc<ProgressSender>>,
) -> Result<(), String> {
    let mut handles = Vec::new();

    let hook_roles: Vec<String> = steps.iter()
        .filter_map(|s| agents::load(&s.agent).ok().map(|a| a.role))
        .collect();

    for role in &hook_roles {
        match role.as_str() {
            "planner" => pm.fire(HookPoint::BeforePlan),
            "builder" => pm.fire(HookPoint::BeforeBuild),
            "evaluator" => pm.fire(HookPoint::BeforeEvaluate),
            _ => {}
        }
    }

    // Create shared channels for all agent streaming output
    let tui_tx_clone = tui_tx.cloned();
    let progress_clone = progress.cloned();

    for step in steps {
        let step = step.clone();
        let agent_name = step.agent.clone();
        let backend_str = backend_override.map(|s| s.to_string());
        let config = config.clone();
        let tui_sender = tui_tx_clone.clone();
        let progress_sender = progress_clone.clone();

        let handle = std::thread::spawn(move || -> Result<(), String> {
            let mut agent = agents::load(&step.agent)?;
            if step.prompt.is_some() {
                agent.prompt_template = step.prompt.clone();
            }

            let backend = Backend::from_str(
                backend_str.as_deref().unwrap_or(&agent.backend),
            )?;
            let model_owned = agent.model.clone().unwrap_or_else(|| config.model.clone());
            let timeout = agent.timeout_seconds.unwrap_or(match agent.role.as_str() {
                "builder" => config.builder_timeout_seconds,
                _ => config.evaluator_timeout_seconds,
            });

            let prompt = build_agent_prompt(&agent, &config)?;

            // Use streaming when TUI channel or progress sender is available
            let output = if tui_sender.is_some() || progress_sender.is_some() {
                let proc = if agent.role == "builder" {
                    cli_backend::run_builder_streaming(&backend, &model_owned, &prompt, timeout)?
                } else {
                    cli_backend::run_oneshot_streaming(&backend, &model_owned, &prompt, timeout)?
                };

                let name = agent.name.clone();
                loop {
                    match proc.lines.recv_timeout(std::time::Duration::from_millis(50)) {
                        Ok(line) => {
                            if let Some(ref tx) = tui_sender {
                                let _ = tx.send(crate::tui::TuiEvent::AgentOutputLine(
                                    name.clone(), line.clone(),
                                ));
                            }
                            if let Some(ref ps) = progress_sender {
                                ps.stdout(&name, &line);
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                proc.wait()?
            } else {
                // Non-streaming fallback
                if agent.role == "builder" {
                    cli_backend::run_builder(&backend, &model_owned, &prompt, timeout)?
                } else {
                    cli_backend::run_oneshot(&backend, &model_owned, &prompt, timeout)?
                }
            };

            // Write to isolated agent namespace
            let default_artifact = match agent.role.as_str() {
                "planner" => "spec.md",
                "builder" => "status.md",
                "evaluator" => "evaluation.md",
                _ => "output.md",
            };
            artifacts::write_agent_artifact(&agent.name, default_artifact, &output)?;

            // Respect step-level output_artifact override
            if let Some(ref custom) = step.output_artifact {
                artifacts::write_artifact(custom, &output)?;
            }

            scl_lifecycle::record_agent_step(&config.project_name, &agent.name, &agent.role, "completed");
            eprintln!("  [parallel] agent '{}' [{}] -> .harness/agents/{}/{default_artifact}", agent.name, agent.role, agent.name);

            Ok(())
        });
        handles.push((agent_name, handle));
    }

    // Collect results
    let mut errors = Vec::new();
    let completed_names: Vec<String> = handles.iter().map(|(n, _)| n.clone()).collect();
    for (name, handle) in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(format!("agent '{name}': {e}")),
            Err(_) => errors.push(format!("agent '{name}': thread panicked")),
        }
    }

    for role in &hook_roles {
        match role.as_str() {
            "planner" => pm.fire(HookPoint::AfterPlan),
            "builder" => pm.fire(HookPoint::AfterBuild),
            "evaluator" => pm.fire(HookPoint::AfterEvaluate),
            _ => {}
        }
    }

    if !errors.is_empty() {
        return Err(format!("Parallel execution failed:\n  {}", errors.join("\n  ")));
    }

    // Merge isolated artifacts into shared location
    let name_refs: Vec<&str> = completed_names.iter().map(|s| s.as_str()).collect();
    let _ = artifacts::merge_agent_artifacts(&name_refs, "status.md");
    let _ = artifacts::merge_agent_artifacts(&name_refs, "output.md");

    println!("  All parallel agents completed (isolated artifacts in .harness/agents/).");
    Ok(())
}

/// Run an iterative build-evaluate loop.
#[allow(clippy::too_many_arguments)]
fn run_iterative_loop(
    body: &[workflows::WorkflowStep],
    evaluator_step: &workflows::WorkflowStep,
    max_rounds: u32,
    backend_override: Option<&str>,
    config: &Config,
    pm: &PluginManager,
    tui_tx: Option<&std::sync::mpsc::Sender<crate::tui::TuiEvent>>,
    progress: Option<&Arc<ProgressSender>>,
) -> Result<(), String> {
    for round in 1..=max_rounds {
        if let Some(tx) = tui_tx {
            let _ = tx.send(crate::tui::TuiEvent::PhaseChange(
                crate::tui::TuiPhase::Loop { round, max: max_rounds },
                round,
            ));
        }
        println!("\n  === Iteration {round}/{max_rounds} ===");
        let msg = format!("Loop iteration {round}/{max_rounds}");
        artifacts::append_progress(&msg);
        if let Some(ps) = progress { ps.event(&msg); }

        for step in body {
            run_single_step_streaming(step, backend_override, config, pm, tui_tx, progress)?;
        }

        let verdict = run_single_step_streaming(evaluator_step, backend_override, config, pm, tui_tx, progress)?;

        scl_lifecycle::record_loop_iteration(&config.project_name, round, max_rounds);

        match verdict {
            Some(Verdict::Pass) => {
                let msg = format!("Loop completed: PASS (round {round})");
                artifacts::append_progress(&msg);
                if let Some(ps) = progress { ps.event(&msg); }
                println!("  Loop completed: PASS on round {round}");
                return Ok(());
            }
            Some(Verdict::Fail) => {
                let msg = format!("Loop failed: FAIL (round {round})");
                artifacts::append_progress(&msg);
                if let Some(ps) = progress { ps.event(&msg); }
                return Err(format!("Loop failed: FAIL on round {round}"));
            }
            Some(Verdict::Revise) => {
                if round == max_rounds {
                    return Err(format!(
                        "Loop exhausted: {max_rounds} rounds without PASS. Last verdict: REVISE"
                    ));
                }
                println!("  Revise — looping back (round {}/{max_rounds})", round + 1);
            }
            None => {
                // Evaluator didn't return a verdict somehow
                println!("  Warning: evaluator did not return a verdict.");
            }
        }
    }
    Ok(())
}

/// Build the prompt for an agent step.
pub fn build_agent_prompt(agent: &AgentDef, config: &Config) -> Result<String, String> {
    if let Some(template) = &agent.prompt_template {
        let path = std::path::Path::new(template);
        if path.exists() {
            return std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read prompt template '{}': {e}", template));
        }
        return Ok(template.clone());
    }

    match agent.role.as_str() {
        "planner" => {
            let goal = artifacts::read_artifact("goal.md")?;
            Ok(prompts::planner_prompt(&goal))
        }
        "builder" => prompts::builder_prompt(),
        "evaluator" => prompts::evaluator_prompt(),
        _ => {
            let goal = artifacts::read_artifact("goal.md").unwrap_or_default();
            let spec = artifacts::read_artifact("spec.md").unwrap_or_default();
            Ok(format!(
                "You are a '{}' agent for project '{}'.\n\n## Goal\n{goal}\n\n## Spec\n{spec}\n",
                agent.role, config.project_name
            ))
        }
    }
}

/// Resolve a comma-separated list of agent names into definitions.
pub fn resolve_agent_names(names: &[&str]) -> Result<Vec<AgentDef>, String> {
    let mut defs = Vec::new();
    for name in names {
        let agent = agents::load(name)?;
        defs.push(agent);
    }
    Ok(defs)
}

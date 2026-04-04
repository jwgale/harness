pub mod output_panel;
pub mod spec_parser;
pub mod status_panel;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::CrosstermBackend;
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;
use ratatui::Terminal;

use crate::artifacts;
use crate::cli_backend::{self, Backend, StreamingProcess};
use crate::commands::evaluate;
use crate::config::Config;
use crate::evaluator;
use crate::notifications;
use crate::plugins::{PluginManager, HookPoint};
use crate::scl_lifecycle;
use crate::prompts;

use self::output_panel::OutputPanel;
use self::spec_parser::Feature;
use self::status_panel::EvalScores;

#[derive(Clone)]
pub enum TuiPhase {
    Plan,
    Build,
    Evaluate,
    Done,
    /// Multi-agent: parallel batch running
    Parallel(Vec<String>),
    /// Multi-agent: iterative loop
    Loop { round: u32, max: u32 },
    /// Multi-agent: named agent step
    AgentStep(String, String), // (agent_name, role)
}

impl TuiPhase {
    pub fn label(&self) -> String {
        match self {
            TuiPhase::Plan => "PLAN".to_string(),
            TuiPhase::Build => "BUILD".to_string(),
            TuiPhase::Evaluate => "EVALUATE".to_string(),
            TuiPhase::Done => "DONE".to_string(),
            TuiPhase::Parallel(names) => format!("PARALLEL [{}]", names.join(", ")),
            TuiPhase::Loop { round, max } => format!("LOOP {round}/{max}"),
            TuiPhase::AgentStep(name, role) => format!("AGENT '{name}' [{role}]"),
        }
    }

    pub fn color(&self) -> Color {
        match self {
            TuiPhase::Plan => Color::Cyan,
            TuiPhase::Build => Color::Yellow,
            TuiPhase::Evaluate => Color::Magenta,
            TuiPhase::Done => Color::Green,
            TuiPhase::Parallel(_) => Color::Blue,
            TuiPhase::Loop { .. } => Color::LightYellow,
            TuiPhase::AgentStep(_, _) => Color::LightCyan,
        }
    }
}

/// View mode for the TUI layout.
enum ViewMode {
    Split,       // status + output (default)
    OutputOnly,  // full-width output
    StatusOnly,  // full-width status
}

/// Messages sent from the run loop thread to the TUI.
pub enum TuiEvent {
    /// A new output line from the current process.
    OutputLine(String),
    /// A new output line tagged with an agent name (for multi-agent mode).
    AgentOutputLine(String, String), // (agent_name, line)
    /// Phase changed.
    PhaseChange(TuiPhase, u32), // phase, round
    /// Process finished for the current phase, with full output.
    PhaseComplete,
    /// Evaluation scores parsed.
    EvalResult(EvalScores, evaluate::Verdict),
    /// The entire run is finished.
    RunFinished(Result<String, String>),
}

/// Run the full harness loop with TUI display.
pub fn run_with_tui(
    backend_override: Option<&str>,
    max_rounds: Option<u32>,
) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let max = max_rounds.unwrap_or(config.max_eval_rounds);
    let backend = Backend::from_str(backend_override.unwrap_or(&config.backend))?;
    let backend_name = backend_override.unwrap_or(&config.backend).to_string();
    let project_name = config.project_name.clone();
    let model = config.model.clone();

    // Parse features from spec if it exists
    let mut features = spec_parser::parse_features();

    // Channel for TUI events from the run loop
    let (tx, rx) = mpsc::channel::<TuiEvent>();

    // Spawn the run loop in a background thread
    let tx_clone = tx.clone();
    let config_clone_builder_timeout = config.builder_timeout_seconds;
    let config_clone_eval_timeout = config.evaluator_timeout_seconds;
    let model_clone = model.clone();
    let project_clone = project_name.clone();
    let eval_strategy = config.evaluator_strategy.clone();
    std::thread::spawn(move || {
        let result = run_loop(
            &backend,
            &model_clone,
            max,
            config_clone_builder_timeout,
            config_clone_eval_timeout,
            &tx_clone,
            &project_clone,
            &eval_strategy,
        );
        let _ = tx_clone.send(TuiEvent::RunFinished(result));
    });

    // Setup terminal
    terminal::enable_raw_mode()
        .map_err(|e| format!("Failed to enable raw mode: {e}"))?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)
        .map_err(|e| format!("Failed to enter alternate screen: {e}"))?;
    let backend_term = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend_term)
        .map_err(|e| format!("Failed to create terminal: {e}"))?;

    let result = tui_event_loop(
        &mut terminal,
        &rx,
        &project_name,
        &backend_name,
        max,
        &mut features,
    );

    // Restore terminal
    terminal::disable_raw_mode().ok();
    terminal.backend_mut().execute(LeaveAlternateScreen).ok();

    result
}

#[allow(clippy::too_many_arguments)]
fn run_loop(
    backend: &Backend,
    model: &str,
    max_rounds: u32,
    builder_timeout: u64,
    eval_timeout: u64,
    tx: &mpsc::Sender<TuiEvent>,
    project_name: &str,
    eval_strategy: &str,
) -> Result<String, String> {
    // Create a channel that routes plugin hook output into the TUI output panel
    let hook_tx = tx.clone();
    let (plugin_tx, plugin_rx) = mpsc::channel::<String>();
    let pm = PluginManager::load_with_channel(plugin_tx);

    // Spawn a thread to forward plugin output into TUI events
    std::thread::spawn(move || {
        for line in plugin_rx {
            let _ = hook_tx.send(TuiEvent::OutputLine(line));
        }
    });

    // Phase 1: Plan
    let _ = tx.send(TuiEvent::PhaseChange(TuiPhase::Plan, 0));
    pm.fire(HookPoint::BeforePlan);
    let prompt = prompts::planner_prompt(
        &artifacts::read_artifact("goal.md")?,
    );
    let proc = cli_backend::run_oneshot_streaming(backend, model, &prompt, eval_timeout)?;
    let output = drain_streaming(proc, tx);
    let plan_output = output?;
    artifacts::write_artifact("spec.md", &plan_output)?;
    pm.fire(HookPoint::AfterPlan);
    scl_lifecycle::record_plan_complete(project_name);
    let _ = tx.send(TuiEvent::PhaseComplete);

    // Phase 2+3: Build → Evaluate loop
    for round in 1..=max_rounds {
        // Build
        let _ = tx.send(TuiEvent::PhaseChange(TuiPhase::Build, round));
        save_run_metadata(round, backend)?;
        pm.fire(HookPoint::BeforeBuild);
        let prompt = prompts::builder_prompt()?;
        let proc = cli_backend::run_builder_streaming(backend, model, &prompt, builder_timeout)?;
        let _output = drain_streaming(proc, tx)?;
        pm.fire(HookPoint::AfterBuild);
        scl_lifecycle::record_build_complete(project_name, round);
        let _ = tx.send(TuiEvent::PhaseComplete);

        // Evaluate
        let _ = tx.send(TuiEvent::PhaseChange(TuiPhase::Evaluate, round));
        pm.fire(HookPoint::BeforeEvaluate);

        // Build evaluator prompt with optional strategy prefix
        let base_prompt = prompts::evaluator_prompt()?;
        let prefix = evaluator::streaming_prefix_for(eval_strategy)?;
        let prompt = match prefix {
            Some(pfx) => format!("{pfx}{base_prompt}"),
            None => base_prompt,
        };

        let proc = cli_backend::run_oneshot_streaming(backend, model, &prompt, eval_timeout)?;
        let eval_output = drain_streaming(proc, tx)?;

        artifacts::write_artifact("evaluation.md", &eval_output)?;
        let fb_round = artifacts::next_feedback_number();
        artifacts::write_artifact(&format!("feedback/round-{fb_round:03}.md"), &eval_output)?;

        let verdict = evaluate::parse_verdict(&eval_output);
        let scores = EvalScores::parse(&eval_output);
        pm.fire(HookPoint::AfterEvaluate);
        scl_lifecycle::record_eval_complete(project_name, round, &format!("{verdict:?}"), eval_strategy);
        notifications::fire_eval_event(&verdict, project_name, round);
        let _ = tx.send(TuiEvent::EvalResult(scores, verdict.clone()));
        let _ = tx.send(TuiEvent::PhaseComplete);

        update_run_outcome(round, &verdict)?;

        match verdict {
            evaluate::Verdict::Pass => {
                let _ = tx.send(TuiEvent::PhaseChange(TuiPhase::Done, round));
                return Ok(format!("BUILD PASSED on round {round}"));
            }
            evaluate::Verdict::Fail => {
                let _ = tx.send(TuiEvent::PhaseChange(TuiPhase::Done, round));
                return Err(format!("BUILD FAILED on round {round}"));
            }
            evaluate::Verdict::Revise => {
                if round == max_rounds {
                    return Err(format!("Max rounds ({max_rounds}) exhausted. Last verdict: REVISE"));
                }
            }
        }
    }
    Ok("Run complete".to_string())
}

/// Drain a streaming process, sending lines to the TUI channel.
fn drain_streaming(
    proc: StreamingProcess,
    tx: &mpsc::Sender<TuiEvent>,
) -> Result<String, String> {
    // Read lines from the process and forward to TUI
    loop {
        match proc.lines.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                let _ = tx.send(TuiEvent::OutputLine(line));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    proc.wait()
}

fn tui_event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    rx: &mpsc::Receiver<TuiEvent>,
    project_name: &str,
    backend_name: &str,
    max_rounds: u32,
    features: &mut Vec<Feature>,
) -> Result<(), String> {
    let mut output_panel = OutputPanel::new();
    let mut phase = TuiPhase::Plan;
    let mut round = 0u32;
    let mut scores = EvalScores::empty();
    let mut view_mode = ViewMode::Split;
    let start_time = Instant::now();
    let mut run_result: Option<Result<String, String>> = None;

    loop {
        // Process TUI events (non-blocking)
        loop {
            match rx.try_recv() {
                Ok(TuiEvent::OutputLine(line)) => {
                    spec_parser::update_feature_status(features, &line);
                    output_panel.push_line(line);
                }
                Ok(TuiEvent::AgentOutputLine(agent, line)) => {
                    spec_parser::update_feature_status(features, &line);
                    output_panel.push_agent_line(&agent, line);
                }
                Ok(TuiEvent::PhaseChange(new_phase, new_round)) => {
                    phase = new_phase;
                    round = new_round;
                    // Re-parse features after plan phase completes
                    if matches!(phase, TuiPhase::Build) && round == 1 {
                        *features = spec_parser::parse_features();
                    }
                }
                Ok(TuiEvent::PhaseComplete) => {}
                Ok(TuiEvent::EvalResult(new_scores, _verdict)) => {
                    scores = new_scores;
                    // Mark all features as completed if verdict is Pass
                    if scores.verdict.as_deref() == Some("PASS") {
                        for f in features.iter_mut() {
                            f.status = spec_parser::FeatureStatus::Completed;
                        }
                    }
                }
                Ok(TuiEvent::RunFinished(result)) => {
                    run_result = Some(result);
                }
                Err(_) => break,
            }
        }

        // Render
        let elapsed = start_time.elapsed().as_secs();
        terminal.draw(|frame| {
            let area = frame.area();
            match view_mode {
                ViewMode::Split => {
                    let chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(35),
                            Constraint::Percentage(65),
                        ])
                        .split(area);
                    status_panel::render(
                        frame, chunks[0], project_name, &phase, round,
                        max_rounds, backend_name, elapsed, features, &scores,
                        &output_panel.legend(),
                    );
                    output_panel.render(frame, chunks[1]);
                }
                ViewMode::OutputOnly => {
                    output_panel.render(frame, area);
                }
                ViewMode::StatusOnly => {
                    status_panel::render(
                        frame, area, project_name, &phase, round,
                        max_rounds, backend_name, elapsed, features, &scores,
                        &output_panel.legend(),
                    );
                }
            }

            // Show "finished" bar at bottom if done
            if let Some(ref result) = run_result {
                let msg = match result {
                    Ok(s) => format!(" {s} — press q to exit "),
                    Err(s) => format!(" {s} — press q to exit "),
                };
                let color = if result.is_ok() { Color::Green } else { Color::Red };
                let bar_area = Rect::new(area.x, area.y + area.height.saturating_sub(1), area.width, 1);
                let bar = Paragraph::new(msg)
                    .style(Style::default().fg(Color::White).bg(color));
                frame.render_widget(bar, bar_area);
            }
        }).map_err(|e| format!("Draw error: {e}"))?;

        // Handle keyboard input
        if event::poll(Duration::from_millis(50))
            .map_err(|e| format!("Event poll error: {e}"))?
            && let Event::Key(key) = event::read()
                .map_err(|e| format!("Event read error: {e}"))?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => {
                    if let Some(result) = run_result {
                        return result.map(|_| ());
                    }
                    return Err("Aborted by user".to_string());
                }
                KeyCode::Char('f') | KeyCode::End => {
                    output_panel.toggle_follow();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    output_panel.scroll_up(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let h = terminal.size().map(|s| s.height as usize).unwrap_or(24);
                    output_panel.scroll_down(1, h);
                }
                KeyCode::PageUp => {
                    let h = terminal.size().map(|s| s.height as usize).unwrap_or(24);
                    output_panel.page_up(h);
                }
                KeyCode::PageDown => {
                    let h = terminal.size().map(|s| s.height as usize).unwrap_or(24);
                    output_panel.page_down(h);
                }
                KeyCode::Tab => {
                    view_mode = match view_mode {
                        ViewMode::Split => ViewMode::OutputOnly,
                        ViewMode::OutputOnly => ViewMode::Split,
                        ViewMode::StatusOnly => ViewMode::Split,
                    };
                }
                KeyCode::Char('1') => view_mode = ViewMode::Split,
                KeyCode::Char('2') => view_mode = ViewMode::OutputOnly,
                KeyCode::Char('3') => view_mode = ViewMode::StatusOnly,
                KeyCode::Char('`') => { output_panel.cycle_filter(); }
                KeyCode::Char('4') => output_panel.set_filter(0), // All
                KeyCode::Char('5') => output_panel.set_filter(1), // Agent 1
                KeyCode::Char('6') => output_panel.set_filter(2), // Agent 2
                KeyCode::Char('7') => output_panel.set_filter(3), // Agent 3
                _ => {}
            }
        }
    }
}

fn save_run_metadata(round: u32, backend: &Backend) -> Result<(), String> {
    let backend_str = match backend {
        Backend::Claude => "claude",
        Backend::Codex => "codex",
        Backend::Mock => "mock",
    };
    let run_num = artifacts::next_run_number();
    let metadata = serde_json::json!({
        "id": run_num,
        "round": round,
        "phase": "build+evaluate",
        "backend": backend_str,
        "started_at": chrono::Utc::now().to_rfc3339(),
        "ended_at": null,
        "outcome": null,
    });
    let json = serde_json::to_string_pretty(&metadata)
        .map_err(|e| format!("Failed to serialize run metadata: {e}"))?;
    artifacts::write_artifact(&format!("runs/run-{run_num:03}.json"), &json)
}

fn update_run_outcome(round: u32, verdict: &evaluate::Verdict) -> Result<(), String> {
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

/// Run multi-agent workflow with TUI display.
/// Runs the multi-agent logic in a background thread while showing progress.
pub fn run_multi_agent_tui(
    backend_override: Option<&str>,
    agents_csv: Option<&str>,
    workflow_name: Option<&str>,
    parallel: bool,
) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let project_name = config.project_name.clone();
    let backend_name = backend_override.unwrap_or(&config.backend).to_string();

    let features = spec_parser::parse_features();

    let (tx, rx) = mpsc::channel::<TuiEvent>();

    // Capture params for the background thread
    let tx_clone = tx.clone();
    let backend_str = backend_override.map(|s| s.to_string());
    let agents_str = agents_csv.map(|s| s.to_string());
    let workflow_str = workflow_name.map(|s| s.to_string());

    std::thread::spawn(move || {
        // Redirect eprintln output to TUI by running the plain multi-agent logic
        // and sending phase updates through the channel
        let result = run_multi_agent_with_events(
            backend_str.as_deref(),
            agents_str.as_deref(),
            workflow_str.as_deref(),
            parallel,
            &tx_clone,
        );
        let _ = tx_clone.send(TuiEvent::RunFinished(
            result.map(|_| "Multi-agent run complete".to_string())
        ));
    });

    // Setup terminal
    terminal::enable_raw_mode()
        .map_err(|e| format!("Failed to enable raw mode: {e}"))?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)
        .map_err(|e| format!("Failed to enter alternate screen: {e}"))?;
    let backend_term = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend_term)
        .map_err(|e| format!("Failed to create terminal: {e}"))?;

    let mut features = features;
    let result = tui_event_loop(
        &mut terminal, &rx, &project_name, &backend_name,
        0, &mut features,
    );

    terminal::disable_raw_mode().ok();
    terminal.backend_mut().execute(LeaveAlternateScreen).ok();

    result
}

/// Run multi-agent logic while sending TUI events for phase tracking.
fn run_multi_agent_with_events(
    backend_override: Option<&str>,
    agents_csv: Option<&str>,
    workflow_name: Option<&str>,
    parallel: bool,
    tx: &mpsc::Sender<TuiEvent>,
) -> Result<(), String> {
    use crate::commands::run::run_step_groups_with_tui;
    use crate::scl_lifecycle;
    use crate::workflows;

    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;

    // Route plugin output into TUI
    let hook_tx = tx.clone();
    let (plugin_tx, plugin_rx) = mpsc::channel::<String>();
    let pm = PluginManager::load_with_channel(plugin_tx);
    std::thread::spawn(move || {
        for line in plugin_rx {
            let _ = hook_tx.send(TuiEvent::OutputLine(line));
        }
    });

    if let Some(wf_name) = workflow_name {
        let wf = workflows::load(wf_name)?;
        let errors = workflows::validate(&wf);
        if !errors.is_empty() {
            return Err(format!("Workflow has {} validation error(s)", errors.len()));
        }

        let groups = workflows::plan_execution(&wf);
        let agent_names: Vec<String> = wf.steps.iter().map(|s| s.agent.clone()).collect();
        let name_refs: Vec<&str> = agent_names.iter().map(|s| s.as_str()).collect();
        scl_lifecycle::record_agent_run_start(&config.project_name, &name_refs);

        let _ = tx.send(TuiEvent::OutputLine(
            format!("Running workflow '{}' ({} groups)", wf.name, groups.len())
        ));

        let result = run_step_groups_with_tui(&groups, backend_override, &config, &pm, Some(tx), None);

        let status = if result.is_ok() { "completed" } else { "FAIL" };
        scl_lifecycle::record_agent_run_end(&config.project_name, &name_refs, status);
        let _ = tx.send(TuiEvent::PhaseChange(TuiPhase::Done, 0));
        return result;
    }

    if let Some(csv) = agents_csv {
        let names: Vec<&str> = csv.split(',').map(|s| s.trim()).collect();
        let defs = crate::commands::run::resolve_agent_names(&names)?;
        scl_lifecycle::record_agent_run_start(&config.project_name, &names);

        if parallel {
            let names_owned: Vec<String> = defs.iter().map(|a| a.name.clone()).collect();
            let _ = tx.send(TuiEvent::PhaseChange(TuiPhase::Parallel(names_owned), 1));
        }

        let steps: Vec<workflows::WorkflowStep> = defs.iter().map(|a| {
            workflows::WorkflowStep {
                agent: a.name.clone(),
                prompt: None,
                output_artifact: None,
                parallel,
                loop_until: None,
                max_rounds: None,
            }
        }).collect();

        let groups: Vec<workflows::StepGroup> = if parallel {
            vec![workflows::StepGroup::Parallel(steps)]
        } else {
            steps.into_iter().map(workflows::StepGroup::Single).collect()
        };

        let result = run_step_groups_with_tui(&groups, backend_override, &config, &pm, Some(tx), None);
        let status = if result.is_ok() { "completed" } else { "FAIL" };
        scl_lifecycle::record_agent_run_end(&config.project_name, &names, status);
        let _ = tx.send(TuiEvent::PhaseChange(TuiPhase::Done, 0));
        return result;
    }

    Err("Either --agents or --workflow is required".to_string())
}

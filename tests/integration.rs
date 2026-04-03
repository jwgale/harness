use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn harness_bin() -> String {
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // remove test binary name
    path.pop(); // remove deps/
    path.push("harness");
    path.to_string_lossy().to_string()
}

fn run_harness(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(harness_bin())
        .args(args)
        .output()
        .expect("Failed to run harness");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

fn run_harness_in(dir: &PathBuf, args: &[&str]) -> (String, String, bool) {
    let output = Command::new(harness_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("Failed to run harness");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

fn tempdir(label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("harness-test-{label}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_version() {
    let (stdout, _, ok) = run_harness(&["--version"]);
    assert!(ok);
    assert!(stdout.contains("harness"));
}

#[test]
fn test_init_and_status() {
    let tmp = tempdir("init");

    let (stdout, _, ok) = run_harness_in(&tmp, &["init", "Test project for integration"]);
    assert!(ok, "init failed: {stdout}");
    assert!(stdout.contains("Initialized .harness/"));

    // Check artifacts exist
    assert!(tmp.join(".harness/config.json").exists());
    assert!(tmp.join(".harness/goal.md").exists());
    assert!(tmp.join(".harness/status.md").exists());
    assert!(tmp.join(".harness/prompts").exists());
    assert!(tmp.join(".harness/feedback").exists());
    assert!(tmp.join(".harness/runs").exists());

    // Check goal content
    let goal = fs::read_to_string(tmp.join(".harness/goal.md")).unwrap();
    assert_eq!(goal, "Test project for integration");

    // Status should work
    let (stdout, _, ok) = run_harness_in(&tmp, &["status"]);
    assert!(ok);
    assert!(stdout.contains("Test project for integration"));

    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_workspace_roundtrip() {
    let tmp = tempdir("ws");

    // Init a project first
    run_harness_in(&tmp, &["init", "Workspace test"]);

    // Register
    let (stdout, _, ok) = run_harness(&["workspace", "register", tmp.to_str().unwrap()]);
    assert!(ok, "register failed: {stdout}");
    assert!(stdout.contains("Registered workspace"));

    // List
    let (stdout, _, ok) = run_harness(&["workspace", "list"]);
    assert!(ok);
    let name = tmp.file_name().unwrap().to_str().unwrap();
    assert!(stdout.contains(name));
    assert!(stdout.contains("[active]"));

    // Remove
    let (stdout, _, ok) = run_harness(&["workspace", "remove", name]);
    assert!(ok, "remove failed: {stdout}");
    assert!(stdout.contains("Removed workspace"));

    // List again
    let (stdout, _, ok) = run_harness(&["workspace", "list"]);
    assert!(ok);
    assert!(!stdout.contains(name));

    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_schedule_roundtrip() {
    // Add
    let (stdout, _, ok) = run_harness(&["schedule", "add", "inttest", "0 8 * * *", "echo hello"]);
    assert!(ok, "schedule add failed: {stdout}");
    assert!(stdout.contains("Scheduled task added"));

    // List
    let (stdout, _, ok) = run_harness(&["schedule", "list"]);
    assert!(ok);
    assert!(stdout.contains("inttest"));
    assert!(stdout.contains("0 8 * * *"));

    // Manual run
    let (stdout, _, ok) = run_harness(&["schedule", "run", "inttest"]);
    assert!(ok, "schedule run failed: {stdout}");
    assert!(stdout.contains("Completed"));

    // History should now have an entry
    let (stdout, _, ok) = run_harness(&["schedule", "history"]);
    assert!(ok);
    assert!(stdout.contains("inttest"));

    // Remove
    let (stdout, _, ok) = run_harness(&["schedule", "remove", "inttest"]);
    assert!(ok, "schedule remove failed: {stdout}");
    assert!(stdout.contains("Removed scheduled task"));

    // List again
    let (stdout, _, ok) = run_harness(&["schedule", "list"]);
    assert!(ok);
    assert!(!stdout.contains("inttest"));
}

#[test]
fn test_hook_execution() {
    let tmp = tempdir("hook");
    let plugins_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let marker = tmp.join("hook-fired.txt");
    let marker_path = marker.to_string_lossy().replace('\"', "\\\"");

    // Create a test plugin that writes a marker file
    let plugin_content = format!(
        "name = \"integration-test-hook\"\ndescription = \"test\"\nversion = \"0.1.0\"\n\n[hooks]\nbefore_plan = \"echo fired > '{marker_path}'\"\n"
    );
    let plugin_file = plugins_dir.join("integration-test-hook.toml");
    fs::write(&plugin_file, &plugin_content).unwrap();

    // Init a project
    run_harness_in(&tmp, &["init", "Hook test"]);

    // Run plan with mock backend (fast, no real CLI needed)
    let _ = run_harness_in(&tmp, &["plan", "--backend", "mock"]);

    // Check the marker file was created
    assert!(marker.exists(), "Hook did not fire — marker file not created at {}", marker.display());

    // Cleanup
    fs::remove_file(&plugin_file).ok();
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_plugin_list() {
    let (stdout, _, ok) = run_harness(&["plugin", "list"]);
    assert!(ok);
    // Should either show plugins or say none installed
    assert!(stdout.contains("plugin") || stdout.contains("No plugins"));
}

#[test]
fn test_feedback_no_harness() {
    let tmp = tempdir("fb");
    let (_, stderr, ok) = run_harness_in(&tmp, &["feedback"]);
    assert!(!ok);
    assert!(stderr.contains("No .harness/"));
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_schedule_history() {
    let (stdout, _, ok) = run_harness(&["schedule", "history"]);
    assert!(ok);
    // Should say no history or show entries
    assert!(stdout.contains("history") || stdout.contains("History"));
}

#[test]
fn test_mock_backend_plan() {
    let tmp = tempdir("mock");
    run_harness_in(&tmp, &["init", "Mock test"]);

    let (stdout, _, ok) = run_harness_in(&tmp, &["plan", "--backend", "mock"]);
    assert!(ok, "mock plan failed: {stdout}");
    assert!(stdout.contains("spec.md"));
    assert!(tmp.join(".harness/spec.md").exists());

    let spec = fs::read_to_string(tmp.join(".harness/spec.md")).unwrap();
    assert!(spec.contains("Mock"));

    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_reset_generates_handoff() {
    let tmp = tempdir("reset");
    run_harness_in(&tmp, &["init", "Reset test"]);

    let (stdout, _, ok) = run_harness_in(&tmp, &["reset"]);
    assert!(ok, "reset failed: {stdout}");
    assert!(stdout.contains("handoff.md"));
    assert!(tmp.join(".harness/handoff.md").exists());

    let handoff = fs::read_to_string(tmp.join(".harness/handoff.md")).unwrap();
    assert!(handoff.contains("Reset test"));

    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_evaluator_list() {
    let (stdout, _, ok) = run_harness(&["evaluator", "list"]);
    assert!(ok);
    assert!(stdout.contains("default"));
    assert!(stdout.contains("playwright-mcp"));
    assert!(stdout.contains("curl"));
}

#[test]
fn test_evaluator_use() {
    let tmp = tempdir("evaluse");
    run_harness_in(&tmp, &["init", "Evaluator test"]);

    // Set to curl
    let (stdout, _, ok) = run_harness_in(&tmp, &["evaluator", "use", "curl"]);
    assert!(ok, "evaluator use failed: {stdout}");
    assert!(stdout.contains("curl"));

    // Verify config was updated
    let config_str = fs::read_to_string(tmp.join(".harness/config.json")).unwrap();
    assert!(config_str.contains("curl"));

    // Set to invalid strategy
    let (_, stderr, ok) = run_harness_in(&tmp, &["evaluator", "use", "bogus"]);
    assert!(!ok);
    assert!(stderr.contains("Unknown strategy"));

    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_evaluator_strategy_in_config() {
    let tmp = tempdir("evalcfg");
    run_harness_in(&tmp, &["init", "Strategy config test"]);

    // Default config should have evaluator_strategy = "default"
    let config_str = fs::read_to_string(tmp.join(".harness/config.json")).unwrap();
    assert!(config_str.contains("evaluator_strategy"));
    assert!(config_str.contains("default"));

    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_mock_evaluate_with_strategy() {
    let tmp = tempdir("evalstrat");
    run_harness_in(&tmp, &["init", "Mock eval strategy test"]);

    // Plan first (needed before evaluate)
    run_harness_in(&tmp, &["plan", "--backend", "mock"]);

    // Evaluate with default strategy
    let (stdout, _, ok) = run_harness_in(&tmp, &["evaluate", "--backend", "mock"]);
    assert!(ok, "evaluate failed: {stdout}");
    assert!(stdout.contains("Verdict:"));
    assert!(stdout.contains("strategy: default"));

    // Switch to curl and evaluate again
    run_harness_in(&tmp, &["evaluator", "use", "curl"]);
    let (stdout, _, ok) = run_harness_in(&tmp, &["evaluate", "--backend", "mock"]);
    assert!(ok, "curl evaluate failed: {stdout}");
    assert!(stdout.contains("strategy: curl"));

    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_notification_plugin_discovery() {
    let plugins_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    // Create a notification plugin
    let plugin_content = r#"
name = "test-notify"
description = "Test notification plugin"

[notifications]
strategy = "webhook"
url = "http://localhost:9999/test"
events = ["on_eval_pass"]
"#;
    let plugin_file = plugins_dir.join("test-notify.toml");
    fs::write(&plugin_file, plugin_content).unwrap();

    // Plugin list should still work (notifications are separate from hooks)
    let (stdout, _, ok) = run_harness(&["plugin", "list"]);
    assert!(ok, "plugin list failed after adding notification plugin: {stdout}");

    // Cleanup
    fs::remove_file(&plugin_file).ok();
}

#[test]
fn test_agent_list_empty() {
    let (stdout, _, ok) = run_harness(&["agent", "list"]);
    assert!(ok);
    assert!(stdout.contains("agent") || stdout.contains("No agents"));
}

#[test]
fn test_agent_add_remove() {
    let agents_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/agents");
    fs::create_dir_all(&agents_dir).unwrap();

    // Add
    let (stdout, _, ok) = run_harness(&[
        "agent", "add", "test-planner",
        "--role", "planner",
        "--backend", "mock",
        "--description", "Integration test planner",
    ]);
    assert!(ok, "agent add failed: {stdout}");
    assert!(stdout.contains("test-planner"));
    assert!(agents_dir.join("test-planner.toml").exists());

    // List should show it
    let (stdout, _, ok) = run_harness(&["agent", "list"]);
    assert!(ok);
    assert!(stdout.contains("test-planner"));
    assert!(stdout.contains("planner"));

    // Remove
    let (stdout, _, ok) = run_harness(&["agent", "remove", "test-planner"]);
    assert!(ok, "agent remove failed: {stdout}");
    assert!(stdout.contains("removed"));
    assert!(!agents_dir.join("test-planner.toml").exists());
}

#[test]
fn test_agent_add_invalid_role() {
    let (_, stderr, ok) = run_harness(&[
        "agent", "add", "bad-agent",
        "--role", "wizard",
        "--backend", "claude",
    ]);
    assert!(!ok);
    assert!(stderr.contains("Invalid role"));
}

#[test]
fn test_multi_agent_run_with_mock() {
    let tmp = tempdir("multiagent");
    run_harness_in(&tmp, &["init", "Multi-agent test"]);

    let agents_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/agents");
    fs::create_dir_all(&agents_dir).unwrap();

    // Create test agents
    fs::write(agents_dir.join("ma-planner.toml"), r#"
name = "ma-planner"
role = "planner"
backend = "mock"
"#).unwrap();

    fs::write(agents_dir.join("ma-builder.toml"), r#"
name = "ma-builder"
role = "builder"
backend = "mock"
"#).unwrap();

    fs::write(agents_dir.join("ma-evaluator.toml"), r#"
name = "ma-evaluator"
role = "evaluator"
backend = "mock"
"#).unwrap();

    // Run with --agents
    let (stdout, _, ok) = run_harness_in(&tmp, &[
        "run", "--agents", "ma-planner,ma-builder,ma-evaluator", "--no-tui",
    ]);
    assert!(ok, "multi-agent run failed: {stdout}");
    assert!(stdout.contains("Multi-agent run"));
    assert!(stdout.contains("ma-planner"));

    // Artifacts should exist
    assert!(tmp.join(".harness/spec.md").exists());
    assert!(tmp.join(".harness/evaluation.md").exists());

    // Cleanup
    fs::remove_file(agents_dir.join("ma-planner.toml")).ok();
    fs::remove_file(agents_dir.join("ma-builder.toml")).ok();
    fs::remove_file(agents_dir.join("ma-evaluator.toml")).ok();
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_workflow_run_with_mock() {
    let tmp = tempdir("workflow");
    run_harness_in(&tmp, &["init", "Workflow test"]);

    let agents_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/agents");
    let workflows_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/workflows");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::create_dir_all(&workflows_dir).unwrap();

    // Create agents
    fs::write(agents_dir.join("wf-planner.toml"), r#"
name = "wf-planner"
role = "planner"
backend = "mock"
"#).unwrap();

    fs::write(agents_dir.join("wf-builder.toml"), r#"
name = "wf-builder"
role = "builder"
backend = "mock"
"#).unwrap();

    // Create workflow
    fs::write(workflows_dir.join("test-flow.toml"), r#"
name = "test-flow"
description = "Test workflow"
max_rounds = 1

[[steps]]
agent = "wf-planner"

[[steps]]
agent = "wf-builder"
"#).unwrap();

    // Run with --workflow
    let (stdout, stderr, ok) = run_harness_in(&tmp, &[
        "run", "--workflow", "test-flow", "--no-tui",
    ]);
    assert!(ok, "workflow run failed: stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("Running workflow 'test-flow'"));
    assert!(stdout.contains("Workflow 'test-flow' completed"));

    // Cleanup
    fs::remove_file(agents_dir.join("wf-planner.toml")).ok();
    fs::remove_file(agents_dir.join("wf-builder.toml")).ok();
    fs::remove_file(workflows_dir.join("test-flow.toml")).ok();
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_parallel_agents_with_mock() {
    let tmp = tempdir("parallel");
    run_harness_in(&tmp, &["init", "Parallel test"]);

    let agents_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/agents");
    fs::create_dir_all(&agents_dir).unwrap();

    fs::write(agents_dir.join("par-a.toml"), "name = \"par-a\"\nrole = \"custom\"\nbackend = \"mock\"\n").unwrap();
    fs::write(agents_dir.join("par-b.toml"), "name = \"par-b\"\nrole = \"custom\"\nbackend = \"mock\"\n").unwrap();

    // Run with --parallel
    let (stdout, _stderr, ok) = run_harness_in(&tmp, &[
        "run", "--agents", "par-a,par-b", "--parallel", "--no-tui",
    ]);
    assert!(ok, "parallel run failed: {stdout}");
    assert!(stdout.contains("parallel"));

    // Both agent-namespaced outputs should exist
    assert!(tmp.join(".harness/agents/par-a/output.md").exists());
    assert!(tmp.join(".harness/agents/par-b/output.md").exists());

    fs::remove_file(agents_dir.join("par-a.toml")).ok();
    fs::remove_file(agents_dir.join("par-b.toml")).ok();
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_workflow_validate() {
    let agents_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/agents");
    let workflows_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/workflows");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::create_dir_all(&workflows_dir).unwrap();

    // Create agents
    fs::write(agents_dir.join("val-planner.toml"), "name = \"val-planner\"\nrole = \"planner\"\nbackend = \"mock\"\n").unwrap();
    fs::write(agents_dir.join("val-builder.toml"), "name = \"val-builder\"\nrole = \"builder\"\nbackend = \"mock\"\n").unwrap();

    // Create valid workflow
    fs::write(workflows_dir.join("valid-wf.toml"), r#"
name = "valid-wf"
[[steps]]
agent = "val-planner"
[[steps]]
agent = "val-builder"
"#).unwrap();

    let (stdout, _, ok) = run_harness(&["workflow", "validate", "valid-wf"]);
    assert!(ok, "validate failed: {stdout}");
    assert!(stdout.contains("valid"));

    // Create invalid workflow (references non-existent agent)
    fs::write(workflows_dir.join("invalid-wf.toml"), r#"
name = "invalid-wf"
[[steps]]
agent = "nonexistent-agent"
"#).unwrap();

    let (_, stderr, ok) = run_harness(&["workflow", "validate", "invalid-wf"]);
    assert!(!ok, "invalid workflow should fail validation");
    assert!(stderr.contains("validation failed") || stderr.contains("error"));

    // Cleanup
    fs::remove_file(agents_dir.join("val-planner.toml")).ok();
    fs::remove_file(agents_dir.join("val-builder.toml")).ok();
    fs::remove_file(workflows_dir.join("valid-wf.toml")).ok();
    fs::remove_file(workflows_dir.join("invalid-wf.toml")).ok();
}

#[test]
fn test_workflow_list() {
    let (stdout, _, ok) = run_harness(&["workflow", "list"]);
    assert!(ok);
    assert!(stdout.contains("workflow") || stdout.contains("No workflows"));
}

#[test]
fn test_iterative_loop_workflow() {
    let tmp = tempdir("iterloop");
    run_harness_in(&tmp, &["init", "Iterative loop test"]);

    let agents_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/agents");
    let workflows_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/workflows");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::create_dir_all(&workflows_dir).unwrap();

    fs::write(agents_dir.join("loop-planner.toml"), "name = \"loop-planner\"\nrole = \"planner\"\nbackend = \"mock\"\n").unwrap();
    fs::write(agents_dir.join("loop-builder.toml"), "name = \"loop-builder\"\nrole = \"builder\"\nbackend = \"mock\"\n").unwrap();
    fs::write(agents_dir.join("loop-evaluator.toml"), "name = \"loop-evaluator\"\nrole = \"evaluator\"\nbackend = \"mock\"\n").unwrap();

    // Workflow with loop_until
    fs::write(workflows_dir.join("iter-flow.toml"), r#"
name = "iter-flow"
max_rounds = 2

[[steps]]
agent = "loop-planner"

[[steps]]
agent = "loop-builder"
loop_until = "pass"

[[steps]]
agent = "loop-evaluator"
"#).unwrap();

    let (stdout, _stderr, ok) = run_harness_in(&tmp, &[
        "run", "--workflow", "iter-flow", "--no-tui",
    ]);
    assert!(ok, "iterative workflow failed: {stdout}");
    // Mock backend returns PASS, so loop should complete on first iteration
    assert!(stdout.contains("Loop completed: PASS"));

    fs::remove_file(agents_dir.join("loop-planner.toml")).ok();
    fs::remove_file(agents_dir.join("loop-builder.toml")).ok();
    fs::remove_file(agents_dir.join("loop-evaluator.toml")).ok();
    fs::remove_file(workflows_dir.join("iter-flow.toml")).ok();
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_parallel_artifact_isolation() {
    let tmp = tempdir("artisolation");
    run_harness_in(&tmp, &["init", "Artifact isolation test"]);

    let agents_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/agents");
    fs::create_dir_all(&agents_dir).unwrap();

    // Two builders running in parallel — should get isolated artifacts
    fs::write(agents_dir.join("iso-a.toml"), "name = \"iso-a\"\nrole = \"builder\"\nbackend = \"mock\"\n").unwrap();
    fs::write(agents_dir.join("iso-b.toml"), "name = \"iso-b\"\nrole = \"builder\"\nbackend = \"mock\"\n").unwrap();

    // Create a spec.md so builders have something to work with
    fs::write(tmp.join(".harness/spec.md"), "# Test Spec\nBuild something.").unwrap();

    let (stdout, stderr, ok) = run_harness_in(&tmp, &[
        "run", "--agents", "iso-a,iso-b", "--parallel", "--no-tui",
    ]);
    assert!(ok, "parallel isolation run failed: {stdout} {stderr}");

    // Each builder should have its own isolated status.md
    assert!(tmp.join(".harness/agents/iso-a/status.md").exists());
    assert!(tmp.join(".harness/agents/iso-b/status.md").exists());

    // Merged status.md should also exist in the shared location
    assert!(tmp.join(".harness/status.md").exists());
    let merged = fs::read_to_string(tmp.join(".harness/status.md")).unwrap();
    assert!(merged.contains("Agent: iso-a") || merged.contains("Agent: iso-b"));

    fs::remove_file(agents_dir.join("iso-a.toml")).ok();
    fs::remove_file(agents_dir.join("iso-b.toml")).ok();
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_parallel_output_artifact_override() {
    let tmp = tempdir("artoverride");
    run_harness_in(&tmp, &["init", "Artifact override test"]);
    fs::write(tmp.join(".harness/spec.md"), "# Test Spec").unwrap();

    let agents_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/agents");
    let workflows_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/.config"))
        .join("harness/workflows");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::create_dir_all(&workflows_dir).unwrap();

    fs::write(agents_dir.join("ov-a.toml"), "name = \"ov-a\"\nrole = \"custom\"\nbackend = \"mock\"\n").unwrap();
    fs::write(agents_dir.join("ov-b.toml"), "name = \"ov-b\"\nrole = \"custom\"\nbackend = \"mock\"\n").unwrap();

    // Workflow with output_artifact overrides in parallel steps
    fs::write(workflows_dir.join("override-wf.toml"), r#"
name = "override-wf"

[[steps]]
agent = "ov-a"
parallel = true
output_artifact = "custom-a.md"

[[steps]]
agent = "ov-b"
parallel = true
output_artifact = "custom-b.md"
"#).unwrap();

    let (stdout, stderr, ok) = run_harness_in(&tmp, &[
        "run", "--workflow", "override-wf", "--no-tui",
    ]);
    assert!(ok, "override workflow failed: {stdout} {stderr}");

    // Custom artifacts should exist
    assert!(tmp.join(".harness/custom-a.md").exists(), "custom-a.md not found");
    assert!(tmp.join(".harness/custom-b.md").exists(), "custom-b.md not found");

    fs::remove_file(agents_dir.join("ov-a.toml")).ok();
    fs::remove_file(agents_dir.join("ov-b.toml")).ok();
    fs::remove_file(workflows_dir.join("override-wf.toml")).ok();
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_vault_init() {
    let (stdout, _, ok) = run_harness(&["vault", "init"]);
    assert!(ok, "vault init failed: {stdout}");
    assert!(stdout.contains("public key") || stdout.contains("Vault initialized"));
}

#[test]
fn test_vault_status_disabled() {
    // Without vault enabled, should show disabled status
    let (stdout, _, ok) = run_harness(&["vault", "status"]);
    assert!(ok, "vault status failed: {stdout}");
    assert!(stdout.contains("disabled") || stdout.contains("enabled"));
}

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

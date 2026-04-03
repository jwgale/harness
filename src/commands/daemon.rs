use std::fs;
use std::process::Command;

use crate::xdg;

const SERVICE_NAME: &str = "harness";

pub fn run(action: &str) -> Result<(), String> {
    xdg::ensure_dirs()?;

    match action {
        "start" => start(),
        "stop" => stop(),
        "status" => status(),
        "logs" => logs(),
        _ => Err(format!("Unknown daemon action: {action}. Use start, stop, status, or logs.")),
    }
}

fn harness_binary_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("Failed to find harness binary path: {e}"))
}

fn service_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".config/systemd/user")
}

fn service_file_path() -> std::path::PathBuf {
    service_dir().join(format!("{SERVICE_NAME}.service"))
}

fn write_service_file(binary: &str) -> Result<(), String> {
    let dir = service_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create systemd user dir: {e}"))?;

    let data_dir = xdg::data_dir();
    let content = format!(
        r#"[Unit]
Description=Harness Agent Daemon
Documentation=https://github.com/jwgale/harness

[Service]
Type=simple
ExecStart={binary} daemon _run
Restart=on-failure
RestartSec=5
Environment=HARNESS_DAEMON=1
WorkingDirectory={data_dir}

[Install]
WantedBy=default.target
"#,
        data_dir = data_dir.display()
    );

    fs::write(service_file_path(), content)
        .map_err(|e| format!("Failed to write service file: {e}"))
}

fn systemctl(args: &[&str]) -> Result<String, String> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run systemctl: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(format!("systemctl failed: {stderr}"));
    }
    Ok(format!("{stdout}{stderr}"))
}

fn start() -> Result<(), String> {
    // Check if already running
    if let Ok(output) = systemctl(&["is-active", SERVICE_NAME])
        && output.trim() == "active"
    {
        println!("Daemon is already running.");
        return Ok(());
    }

    let binary = harness_binary_path()?;
    write_service_file(&binary)?;

    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", SERVICE_NAME])?;
    systemctl(&["start", SERVICE_NAME])?;

    println!("Daemon started.");
    println!("  Service: {}", service_file_path().display());
    println!("  Binary:  {binary}");
    println!();
    println!("Check status:  harness daemon status");
    println!("View logs:     harness daemon logs");
    println!("Stop:          harness daemon stop");
    Ok(())
}

fn stop() -> Result<(), String> {
    // Check if active first
    let active = systemctl(&["is-active", SERVICE_NAME])
        .map(|s| s.trim() == "active")
        .unwrap_or(false);

    if !active {
        println!("Daemon is not running.");
        return Ok(());
    }

    systemctl(&["stop", SERVICE_NAME])?;
    systemctl(&["disable", SERVICE_NAME])?;

    println!("Daemon stopped and disabled.");
    Ok(())
}

fn status() -> Result<(), String> {
    let active = systemctl(&["is-active", SERVICE_NAME])
        .unwrap_or_else(|_| "inactive".to_string());
    let state = active.trim();

    match state {
        "active" => println!("Daemon is running."),
        "activating" => println!("Daemon is starting..."),
        "failed" => println!("Daemon has failed. Check logs: harness daemon logs"),
        _ => println!("Daemon is not running."),
    }

    // Show service info if the file exists
    if service_file_path().exists() {
        println!("  Service file: {}", service_file_path().display());
    }

    println!();
    println!("XDG directories:");
    println!("  config:  {}", xdg::config_dir().display());
    println!("  data:    {}", xdg::data_dir().display());
    println!("  cache:   {}", xdg::cache_dir().display());
    println!("  plugins: {}", xdg::plugins_dir().display());
    Ok(())
}

fn logs() -> Result<(), String> {
    let output = Command::new("journalctl")
        .args(["--user", "-u", SERVICE_NAME, "-n", "50", "--no-pager"])
        .output()
        .map_err(|e| format!("Failed to run journalctl: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        println!("No daemon logs yet.");
    } else {
        print!("{stdout}");
    }
    Ok(())
}

/// Internal entry point for the daemon process itself.
/// Called via `harness daemon _run` by the systemd service.
pub fn run_daemon_loop() -> Result<(), String> {
    let data_dir = xdg::data_dir();
    let pid = std::process::id();
    let pid_file = data_dir.join("daemon.pid");
    fs::write(&pid_file, pid.to_string())
        .map_err(|e| format!("Failed to write PID file: {e}"))?;

    eprintln!("Harness daemon started (PID {pid})");
    eprintln!("Watching for agent triggers...");

    // Main daemon loop — for now, just stay alive and log heartbeats.
    // Future: watch .harness/ directories, fire plugin hooks, manage agent sessions.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        eprintln!("Harness daemon heartbeat (PID {pid})");
    }
}

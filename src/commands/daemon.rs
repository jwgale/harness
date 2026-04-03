use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use notify::{Event, EventKind, RecursiveMode, Watcher};

use crate::commands::schedule;
use crate::plugins::{HookPoint, PluginManager};
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

fn service_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".config/systemd/user")
}

fn service_file_path() -> PathBuf {
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
ExecStart={binary} daemon internal-run
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

    if service_file_path().exists() {
        println!("  Service file: {}", service_file_path().display());
    }

    // Show watched workspaces
    let ws_dir = xdg::data_dir().join("workspaces");
    if ws_dir.exists()
        && let Ok(entries) = fs::read_dir(&ws_dir)
    {
        let workspaces: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "path"))
            .collect();
        if !workspaces.is_empty() {
            println!();
            println!("Watched workspaces ({}):", workspaces.len());
            for ws in &workspaces {
                let ws_name = ws.path().file_stem()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                if let Ok(path) = fs::read_to_string(ws.path()) {
                    let p = path.trim();
                    let has_harness = Path::new(p).join(".harness").exists();
                    let tag = if has_harness { "active" } else { "no .harness/" };
                    println!("  {ws_name}: {p} [{tag}]");
                }
            }
        }
    }

    // Show schedules
    let schedules = schedule::load_schedules();
    if !schedules.is_empty() {
        println!();
        println!("Scheduled tasks ({}):", schedules.len());
        for (name, cron, cmd) in &schedules {
            println!("  {name}: [{cron}] `{cmd}`");
        }
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
pub fn run_daemon_loop() -> Result<(), String> {
    let data_dir = xdg::data_dir();
    let pid = std::process::id();
    let pid_file = data_dir.join("daemon.pid");
    fs::write(&pid_file, pid.to_string())
        .map_err(|e| format!("Failed to write PID file: {e}"))?;

    let ws_dir = data_dir.join("workspaces");
    fs::create_dir_all(&ws_dir)
        .map_err(|e| format!("Failed to create workspaces dir: {e}"))?;

    let plugins_dir = xdg::plugins_dir();

    eprintln!("Harness daemon started (PID {pid})");
    eprintln!("Watching workspaces in: {}", ws_dir.display());
    eprintln!("Watching plugins in: {}", plugins_dir.display());

    let mut pm = PluginManager::load();

    // Set up notify watcher
    let (notify_tx, notify_rx) = std_mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            let _ = notify_tx.send(event);
        }
    }).map_err(|e| format!("Failed to create file watcher: {e}"))?;

    // Watch the workspaces directory for new registrations
    watcher.watch(ws_dir.as_ref(), RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch workspaces dir: {e}"))?;

    // Watch the plugins directory for hot-reload
    watcher.watch(plugins_dir.as_ref(), RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch plugins dir: {e}"))?;

    // Watch all currently registered workspace .harness/ dirs
    let mut watched_dirs: HashSet<PathBuf> = HashSet::new();
    refresh_watches(&ws_dir, &mut watcher, &mut watched_dirs);

    eprintln!("Watching {} workspace(s)", watched_dirs.len());

    // Track last minute for schedule checking
    let mut last_schedule_minute: i64 = -1;

    loop {
        match notify_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(event) => {
                handle_event(&event, &mut pm, &ws_dir, &plugins_dir, &mut watcher, &mut watched_dirs);
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                // Periodic: refresh watches and check schedules
                refresh_watches(&ws_dir, &mut watcher, &mut watched_dirs);
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("File watcher disconnected, exiting.");
                break;
            }
        }

        // Check scheduled tasks once per minute (local time)
        let now = chrono::Local::now();
        let current_minute = now.timestamp() / 60;
        if current_minute != last_schedule_minute {
            last_schedule_minute = current_minute;
            run_due_schedules(&now, current_minute);
        }
    }

    Ok(())
}

fn handle_event(
    event: &Event,
    pm: &mut PluginManager,
    ws_dir: &Path,
    plugins_dir: &Path,
    watcher: &mut impl Watcher,
    watched_dirs: &mut HashSet<PathBuf>,
) {
    if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)) {
        return;
    }

    for path in &event.paths {
        // Plugin hot-reload: if a .toml in plugins dir changed
        if path.starts_with(plugins_dir) && path.extension().is_some_and(|e| e == "toml") {
            eprintln!("[daemon] Plugin change detected, hot-reloading plugins");
            *pm = PluginManager::load();
            continue;
        }

        // Workspace registration changes
        if path.starts_with(ws_dir) && path.extension().is_some_and(|e| e == "path") {
            if matches!(event.kind, EventKind::Remove(_)) {
                // Workspace was removed — unwatch its .harness/ dir
                if let Ok(content) = fs::read_to_string(path) {
                    let harness_dir = PathBuf::from(content.trim()).join(".harness");
                    if watched_dirs.remove(&harness_dir) {
                        let _ = watcher.unwatch(harness_dir.as_ref());
                        eprintln!("[daemon] Unwatched: {}", harness_dir.display());
                    }
                }
            } else {
                eprintln!("[daemon] Workspace registration changed, refreshing watches");
                refresh_watches(ws_dir, watcher, watched_dirs);
            }
            continue;
        }

        // Harness artifact changes
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            let hook = match filename {
                "spec.md" => Some(HookPoint::AfterPlan),
                "status.md" => Some(HookPoint::AfterBuild),
                "evaluation.md" => Some(HookPoint::AfterEvaluate),
                _ => None,
            };
            if let Some(hook) = hook {
                let ws_name = path.parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.file_name())
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                eprintln!("[daemon] {ws_name}: {filename} changed, firing {}", hook.label());
                pm.fire(hook);
            }
        }
    }
}

fn refresh_watches(
    ws_dir: &Path,
    watcher: &mut impl Watcher,
    watched_dirs: &mut HashSet<PathBuf>,
) {
    let Ok(entries) = fs::read_dir(ws_dir) else { return };

    // Collect currently registered workspace paths
    let mut current_harness_dirs: HashSet<PathBuf> = HashSet::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|e| e != "path") {
            continue;
        }
        let Ok(workspace_path) = fs::read_to_string(&path) else { continue };
        let harness_dir = PathBuf::from(workspace_path.trim()).join(".harness");

        if harness_dir.exists() {
            current_harness_dirs.insert(harness_dir.clone());
            if !watched_dirs.contains(&harness_dir)
                && watcher.watch(harness_dir.as_ref(), RecursiveMode::NonRecursive).is_ok()
            {
                eprintln!("[daemon] Now watching: {}", harness_dir.display());
                watched_dirs.insert(harness_dir);
            }
        }
    }

    // Unwatch dirs that are no longer registered
    let stale: Vec<PathBuf> = watched_dirs.difference(&current_harness_dirs).cloned().collect();
    for dir in stale {
        let _ = watcher.unwatch(dir.as_ref());
        eprintln!("[daemon] Unwatched (removed): {}", dir.display());
        watched_dirs.remove(&dir);
    }
}

fn run_due_schedules(now: &chrono::DateTime<chrono::Local>, now_minute: i64) {
    let schedules = schedule::load_schedules();
    for (name, cron, cmd) in &schedules {
        if !schedule::cron_matches_local(cron, now) {
            continue;
        }
        // Deduplication: skip if already run this minute
        if !schedule::should_run(name, now_minute) {
            continue;
        }

        eprintln!("[daemon] Schedule '{name}' firing: `{cmd}`");
        let start = std::time::Instant::now();
        let result = Command::new("sh")
            .args(["-c", cmd.as_str()])
            .env("HARNESS_SCHEDULE", name.as_str())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                for line in stdout.lines().chain(stderr.lines()) {
                    if !line.trim().is_empty() {
                        eprintln!("[schedule:{name}] {line}");
                    }
                }
                let exit_code = output.status.code().unwrap_or(-1);
                if !output.status.success() {
                    eprintln!("[schedule:{name}] exited with {}", output.status);
                }
                schedule::record_history(name, cmd, exit_code, duration_ms);
            }
            Err(e) => {
                eprintln!("[schedule:{name}] failed to execute: {e}");
                schedule::record_history(name, cmd, -1, duration_ms);
            }
        }
    }
}

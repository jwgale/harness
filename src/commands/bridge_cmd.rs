//! `harness bridge telegram` subcommands.
//!
//! Manages the Telegram bot bridge as a systemd user service.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::bridge::telegram;
use crate::xdg;

const SERVICE_NAME: &str = "harness-telegram";

fn service_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".config/systemd/user")
}

fn service_file_path() -> PathBuf {
    service_dir().join(format!("{SERVICE_NAME}.service"))
}

fn harness_binary_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("Failed to find harness binary path: {e}"))
}

fn write_service_file(binary: &str) -> Result<(), String> {
    let dir = service_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create systemd user dir: {e}"))?;

    let data_dir = xdg::data_dir();
    let content = format!(
        r#"[Unit]
Description=Harness Telegram Bridge
Documentation=https://github.com/jwgale/harness
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={binary} bridge telegram internal-listen
Restart=on-failure
RestartSec=10
Environment=HARNESS_BRIDGE=telegram
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

/// `harness bridge telegram start`
pub fn start() -> Result<(), String> {
    // Verify credentials first
    print!("Checking credentials... ");
    match telegram::check_credentials() {
        Ok(bot_name) => println!("OK ({bot_name})"),
        Err(e) => {
            println!("FAILED");
            return Err(e);
        }
    }

    if telegram::is_running() {
        println!("Telegram bridge is already running.");
        return Ok(());
    }

    let binary = harness_binary_path()?;
    write_service_file(&binary)?;

    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", SERVICE_NAME])?;
    systemctl(&["start", SERVICE_NAME])?;

    println!("Telegram bridge started.");
    println!("  Service: {}", service_file_path().display());
    println!("  Binary:  {binary}");
    println!();
    println!("Check status:  harness bridge telegram status");
    println!("View logs:     journalctl --user -u {SERVICE_NAME} -f");
    println!("Stop:          harness bridge telegram stop");
    Ok(())
}

/// `harness bridge telegram stop`
pub fn stop() -> Result<(), String> {
    if !telegram::is_running() {
        println!("Telegram bridge is not running.");
        return Ok(());
    }

    systemctl(&["stop", SERVICE_NAME])?;
    systemctl(&["disable", SERVICE_NAME])?;

    println!("Telegram bridge stopped and disabled.");
    Ok(())
}

/// `harness bridge telegram status`
pub fn status() -> Result<(), String> {
    let active = systemctl(&["is-active", SERVICE_NAME])
        .unwrap_or_else(|_| "inactive".to_string());
    let state = active.trim();

    match state {
        "active" => println!("Telegram bridge is running."),
        "activating" => println!("Telegram bridge is starting..."),
        "failed" => println!("Telegram bridge has failed. Check logs: journalctl --user -u {SERVICE_NAME} -n 50"),
        _ => println!("Telegram bridge is not running."),
    }

    if service_file_path().exists() {
        println!("  Service file: {}", service_file_path().display());
    }

    // Show credential status
    match telegram::check_credentials() {
        Ok(bot_name) => println!("  Bot: {bot_name}"),
        Err(e) => println!("  Credentials: {e}"),
    }

    Ok(())
}

/// Internal: run the listener loop (used by systemd).
pub fn internal_listen() -> Result<(), String> {
    telegram::run_listener()
}

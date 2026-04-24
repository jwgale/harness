use std::path::Path;
use std::process::Command;

use crate::artifacts;
use crate::cli_backend::Backend;
use crate::config::Config;
use crate::global_config::GlobalConfig;
use crate::{scl, vault, xdg};

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

struct Check {
    name: String,
    status: Status,
    detail: String,
    required: bool,
}

impl Check {
    fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Ok,
            detail: detail.into(),
            required: false,
        }
    }

    fn warn(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Warn,
            detail: detail.into(),
            required: false,
        }
    }

    fn fail(name: impl Into<String>, detail: impl Into<String>, required: bool) -> Self {
        Self {
            name: name.into(),
            status: Status::Fail,
            detail: detail.into(),
            required,
        }
    }
}

pub fn run(deep: bool) -> Result<(), String> {
    xdg::ensure_dirs()?;

    let mut checks = Vec::new();
    check_workspace(&mut checks);
    check_global_config(&mut checks);
    check_tool(&mut checks, "Rust compiler", "rustc", &["--version"], true);
    check_tool(&mut checks, "Cargo", "cargo", &["--version"], true);
    check_tool(&mut checks, "Git", "git", &["--version"], true);
    check_tool(&mut checks, "curl", "curl", &["--version"], true);
    check_tool(&mut checks, "systemd", "systemctl", &["--version"], false);
    check_tool(&mut checks, "Claude CLI", "claude", &["--version"], false);
    check_codex(&mut checks);
    check_optional_cargo_tools(&mut checks);

    if deep {
        check_quality_gate(&mut checks, "cargo fmt --check", &["fmt", "--check"]);
        check_quality_gate(
            &mut checks,
            "cargo clippy -- -D warnings",
            &["clippy", "--", "-D", "warnings"],
        );
        check_quality_gate(&mut checks, "cargo test", &["test"]);
    }

    print_checks(&checks, deep);

    let required_failures = checks
        .iter()
        .filter(|check| check.required && check.status == Status::Fail)
        .count();

    if required_failures > 0 {
        Err(format!(
            "doctor found {required_failures} required failure(s)"
        ))
    } else {
        Ok(())
    }
}

fn check_workspace(checks: &mut Vec<Check>) {
    if Path::new(".git").exists() {
        checks.push(Check::ok("Git workspace", "repository detected"));
    } else {
        checks.push(Check::warn(
            "Git workspace",
            "no .git directory in current path",
        ));
    }

    match artifacts::ensure_harness_exists() {
        Ok(()) => {
            checks.push(Check::ok(".harness", "workspace harness state present"));
            match Config::load(&artifacts::harness_dir()) {
                Ok(config) => {
                    let backend_status = Backend::from_str(&config.backend)
                        .map(|_| "valid")
                        .unwrap_or("invalid");
                    checks.push(Check::ok(
                        ".harness/config.json",
                        format!(
                            "backend={}, model={}, evaluator={}, backend_status={backend_status}",
                            config.backend, config.model, config.evaluator_strategy
                        ),
                    ));
                }
                Err(e) => checks.push(Check::fail(".harness/config.json", e, true)),
            }
        }
        Err(e) => checks.push(Check::warn(".harness", e)),
    }
}

fn check_global_config(checks: &mut Vec<Check>) {
    let config_path = xdg::config_dir().join("config.toml");
    if config_path.exists() {
        checks.push(Check::ok(
            "Global config",
            format!("{}", config_path.display()),
        ));
    } else {
        checks.push(Check::warn(
            "Global config",
            "not present; defaults will be used",
        ));
    }

    let gc = GlobalConfig::load();
    if let Some(scl_cfg) = gc.scl() {
        if scl::is_healthy(scl_cfg.url()) {
            checks.push(Check::ok(
                "Shared Context Layer",
                format!("connected at {}", scl_cfg.url()),
            ));
        } else {
            checks.push(Check::warn(
                "Shared Context Layer",
                format!("configured but unreachable at {}", scl_cfg.url()),
            ));
        }
    } else {
        checks.push(Check::warn("Shared Context Layer", "disabled"));
    }

    let vault_cfg = vault::load_config();
    if vault_cfg.enabled {
        if vault::is_healthy(&vault_cfg) {
            checks.push(Check::ok(
                "Vault",
                format!("connected at {}", vault_cfg.addr),
            ));
        } else {
            checks.push(Check::warn(
                "Vault",
                format!("enabled but unreachable at {}", vault_cfg.addr),
            ));
        }
    } else {
        checks.push(Check::warn("Vault", "disabled"));
    }
}

fn check_codex(checks: &mut Vec<Check>) {
    match command_output("codex", &["--version"]) {
        Ok(version) => checks.push(Check::ok("Codex CLI", first_line(&version))),
        Err(e) => {
            checks.push(Check::fail("Codex CLI", e, true));
            return;
        }
    }

    match command_output("codex", &["exec", "--help"]) {
        Ok(help) => {
            let required_flags = ["--json", "--output-last-message", "--cd", "--full-auto"];
            let missing: Vec<&str> = required_flags
                .iter()
                .copied()
                .filter(|flag| !help.contains(flag))
                .collect();
            if missing.is_empty() {
                checks.push(Check::ok(
                    "Codex exec capabilities",
                    "json events, final-message capture, cwd, and full-auto supported",
                ));
            } else {
                checks.push(Check::fail(
                    "Codex exec capabilities",
                    format!("missing flags: {}", missing.join(", ")),
                    true,
                ));
            }
        }
        Err(e) => checks.push(Check::fail("Codex exec capabilities", e, true)),
    }
}

fn check_optional_cargo_tools(checks: &mut Vec<Check>) {
    for (label, args) in [
        ("cargo-audit", ["audit", "--version"]),
        ("cargo-deny", ["deny", "--version"]),
        ("cargo-nextest", ["nextest", "--version"]),
        ("cargo-llvm-cov", ["llvm-cov", "--version"]),
        ("cargo-machete", ["machete", "--version"]),
    ] {
        match command_output("cargo", &args) {
            Ok(output) => checks.push(Check::ok(label, first_line(&output))),
            Err(_) => checks.push(Check::warn(label, "not installed")),
        }
    }
}

fn check_tool(checks: &mut Vec<Check>, label: &str, command: &str, args: &[&str], required: bool) {
    match command_output(command, args) {
        Ok(output) => checks.push(Check::ok(label, first_line(&output))),
        Err(e) => checks.push(Check::fail(label, e, required)),
    }
}

fn check_quality_gate(checks: &mut Vec<Check>, label: &str, cargo_args: &[&str]) {
    match command_output("cargo", cargo_args) {
        Ok(output) => checks.push(Check::ok(label, last_non_empty_line(&output))),
        Err(e) => checks.push(Check::fail(label, e, true)),
    }
}

fn command_output(command: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run `{command}`: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    if output.status.success() {
        Ok(combined)
    } else {
        Err(last_non_empty_line(&combined))
    }
}

fn first_line(s: &str) -> String {
    s.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("(no output)")
        .trim()
        .to_string()
}

fn last_non_empty_line(s: &str) -> String {
    s.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("(no output)")
        .trim()
        .to_string()
}

fn print_checks(checks: &[Check], deep: bool) {
    println!("Harness doctor");
    println!("  Mode: {}", if deep { "deep" } else { "standard" });
    println!();

    for check in checks {
        let status = match check.status {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => {
                if check.required {
                    "fail"
                } else {
                    "warn"
                }
            }
        };
        println!("  [{status}] {:<26} {}", check.name, check.detail);
    }

    println!();
    if !deep {
        println!("Run `harness doctor --deep` to include fmt, clippy, and tests.");
    }
}

//! Pluggable evaluator strategies.
//!
//! Strategies:
//! - `default` — prompt-based evaluation via the CLI backend (existing behavior)
//! - `playwright-mcp` — launches Playwright MCP to interact with the running app
//! - `curl` — simple HTTP health-check style evaluation for APIs

use crate::artifacts;
use crate::cli_backend::{self, Backend};
use crate::config::Config;
use crate::prompts;

use std::process::Command;

/// All known evaluator strategy names.
pub const STRATEGIES: &[&str] = &["default", "playwright-mcp", "curl"];

/// Validate that a strategy name is known.
pub fn is_valid_strategy(name: &str) -> bool {
    STRATEGIES.contains(&name)
}

/// Describe a strategy for `harness evaluator list`.
pub fn describe(name: &str) -> &'static str {
    match name {
        "default" => "Prompt-based evaluation via CLI backend (Claude/Codex/mock)",
        "playwright-mcp" => "Uses Playwright MCP to interact with the running app in a browser",
        "curl" => "Simple HTTP health-check evaluation for APIs (checks endpoints return 2xx)",
        _ => "(unknown strategy)",
    }
}

/// Run an evaluation using the configured strategy. Returns the evaluation output text.
pub fn run_strategy(config: &Config, backend: &Backend) -> Result<String, String> {
    match config.evaluator_strategy.as_str() {
        "default" => run_default(config, backend),
        "playwright-mcp" => run_playwright_mcp(config, backend),
        "curl" => run_curl(config, backend),
        other => Err(format!(
            "Unknown evaluator strategy: '{other}'. Use `harness evaluator list` to see available strategies."
        )),
    }
}

/// Default strategy: prompt-based evaluation through the CLI backend.
fn run_default(config: &Config, backend: &Backend) -> Result<String, String> {
    let prompt = prompts::evaluator_prompt()?;
    cli_backend::run_oneshot(backend, &config.model, &prompt, config.evaluator_timeout_seconds)
}

/// Playwright MCP strategy: runs the default evaluator but prepends instructions to use
/// Playwright MCP for browser-based interaction testing.
fn run_playwright_mcp(config: &Config, backend: &Backend) -> Result<String, String> {
    let base_prompt = prompts::evaluator_prompt()?;
    let playwright_prefix = r#"## Evaluation Mode: Playwright MCP (Browser Interaction)

You have access to the Playwright MCP tool. Use it to:
1. Launch the application (check spec.md for how to run it)
2. Navigate to the app URL (typically http://localhost:3000 or similar)
3. Interact with the UI — click buttons, fill forms, navigate pages
4. Verify that core user flows work end-to-end
5. Take screenshots of any failures

If the app is not a web application or cannot be launched, fall back to code inspection
and note that browser testing was not applicable.

After browser testing, continue with the standard evaluation below.

---

"#;
    let prompt = format!("{playwright_prefix}{base_prompt}");
    cli_backend::run_oneshot(backend, &config.model, &prompt, config.evaluator_timeout_seconds)
}

/// Curl strategy: checks HTTP endpoints defined in .harness/endpoints.json or spec.md,
/// then runs the default evaluator with the results prepended.
fn run_curl(config: &Config, backend: &Backend) -> Result<String, String> {
    let endpoints = load_endpoints();
    let mut health_report = String::from("## Health Check Results (curl evaluator)\n\n");

    if endpoints.is_empty() {
        health_report.push_str("No endpoints configured. Create `.harness/endpoints.json` with an array of URLs, e.g.:\n");
        health_report.push_str("```json\n[\"http://localhost:3000\", \"http://localhost:3000/api/health\"]\n```\n\n");
        health_report.push_str("Falling back to prompt-based evaluation.\n\n---\n\n");
    } else {
        for endpoint in &endpoints {
            let result = curl_check(endpoint);
            health_report.push_str(&format!("- `{endpoint}`: {result}\n"));
        }
        health_report.push_str("\nUse these results when scoring functionality and robustness.\n\n---\n\n");
    }

    let base_prompt = prompts::evaluator_prompt()?;
    let prompt = format!("{health_report}{base_prompt}");
    cli_backend::run_oneshot(backend, &config.model, &prompt, config.evaluator_timeout_seconds)
}

/// Load endpoints from .harness/endpoints.json (array of URL strings).
fn load_endpoints() -> Vec<String> {
    let path = artifacts::harness_dir().join("endpoints.json");
    if !path.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<Vec<String>>(&content).unwrap_or_default()
}

/// Run a curl health check against a single endpoint. Returns a status string.
fn curl_check(url: &str) -> String {
    let output = Command::new("curl")
        .args([
            "-s", "-o", "/dev/null",
            "-w", "%{http_code}",
            "--max-time", "5",
            "--connect-timeout", "3",
            url,
        ])
        .output();

    match output {
        Ok(out) => {
            let code = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if code.starts_with('2') {
                format!("OK ({code})")
            } else if code == "000" {
                "UNREACHABLE (connection refused or timeout)".to_string()
            } else {
                format!("FAILED ({code})")
            }
        }
        Err(e) => format!("ERROR (curl failed: {e})"),
    }
}

/// Run the pre-evaluation steps for a strategy name and return a prefix
/// to prepend to the evaluator prompt for streaming.
pub fn streaming_prefix_for(strategy: &str) -> Result<Option<String>, String> {
    match strategy {
        "default" => Ok(None),
        "playwright-mcp" => {
            let prefix = r#"## Evaluation Mode: Playwright MCP (Browser Interaction)

You have access to the Playwright MCP tool. Use it to:
1. Launch the application (check spec.md for how to run it)
2. Navigate to the app URL (typically http://localhost:3000 or similar)
3. Interact with the UI — click buttons, fill forms, navigate pages
4. Verify that core user flows work end-to-end
5. Take screenshots of any failures

If the app is not a web application or cannot be launched, fall back to code inspection
and note that browser testing was not applicable.

After browser testing, continue with the standard evaluation below.

---

"#;
            Ok(Some(prefix.to_string()))
        }
        "curl" => {
            let endpoints = load_endpoints();
            let mut report = String::from("## Health Check Results (curl evaluator)\n\n");
            if endpoints.is_empty() {
                report.push_str("No endpoints configured. Create `.harness/endpoints.json`.\n");
                report.push_str("Falling back to prompt-based evaluation.\n\n---\n\n");
            } else {
                for endpoint in &endpoints {
                    let result = curl_check(endpoint);
                    report.push_str(&format!("- `{endpoint}`: {result}\n"));
                }
                report.push_str("\nUse these results when scoring functionality and robustness.\n\n---\n\n");
            }
            Ok(Some(report))
        }
        _ => Ok(None),
    }
}

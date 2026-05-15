//! Claude Code backend implementation.
//!
//! Wraps the `claude` CLI using `--print` + `--permission-mode bypassPermissions`
//! (equivalent to the old `--dangerously-skip-permissions` behavior).

use std::io::Write;
use std::process::{Command, Stdio};

use super::StreamingProcess;
use crate::global_config::GlobalConfig;
use crate::scl;

pub struct ClaudeBackend;

impl ClaudeBackend {
    pub fn new() -> Self {
        Self
    }

    fn should_pass_model(model: &str) -> bool {
        let model = model.trim();
        !model.is_empty() && model != "default"
    }

    fn build_command(model: &str) -> Command {
        let mut cmd = Command::new("claude");
        cmd.args(["--print", "--permission-mode", "bypassPermissions"]);

        if Self::should_pass_model(model) {
            cmd.args(["--model", model]);
        }

        // Inject SCL MCP config if enabled and reachable (Claude-only feature today)
        let gc = GlobalConfig::load();
        if let Some(scl_cfg) = gc.scl()
            && scl::is_healthy(scl_cfg.url())
            && let Ok(mcp_path) = scl::generate_mcp_config(scl_cfg.url())
        {
            cmd.arg("--mcp-config");
            cmd.arg(mcp_path);
            eprintln!("[scl] Connected: {}", scl_cfg.url());
        }

        cmd
    }

    fn run_oneshot_impl(model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
        let mut cmd = Self::build_command(model);
        cmd.args(["-p", prompt]);

        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn claude: {e}"))?;

        let output = super::wait_with_timeout(&mut child, timeout_secs)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("claude exited with error: {stderr}"));
        }

        String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8 in claude output: {e}"))
    }

    fn run_builder_impl(model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
        // For now, builder uses the same one-shot style as the original.
        // In a future improvement we can support long-running interactive Claude sessions.
        Self::run_oneshot_impl(model, prompt, timeout_secs)
    }
}

impl super::AgentBackend for ClaudeBackend {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn is_available(&self) -> Result<bool, String> {
        // Simple check: is `claude` in PATH?
        match Command::new("claude").arg("--version").output() {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    fn description(&self) -> String {
        "Claude Code (Claude Max / Team subscription)".to_string()
    }

    fn run_oneshot(&self, model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
        Self::run_oneshot_impl(model, prompt, timeout_secs)
    }

    fn run_builder(&self, model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
        Self::run_builder_impl(model, prompt, timeout_secs)
    }

    fn run_oneshot_streaming(
        &self,
        model: &str,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<StreamingProcess, String> {
        let mut cmd = Self::build_command(model);
        cmd.args(["-p", prompt]);

        super::spawn_streaming_no_stdin(cmd, timeout_secs)
    }

    fn run_builder_streaming(
        &self,
        model: &str,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<StreamingProcess, String> {
        self.run_oneshot_streaming(model, prompt, timeout_secs)
    }
}

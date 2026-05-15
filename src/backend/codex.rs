//! Codex backend implementation (stubbed for PR 1).
//!
//! Full logic will be moved from the old cli_backend.rs in the next step of this PR.

use super::AgentBackend;
use super::StreamingProcess;

pub struct CodexBackend;

impl CodexBackend {
    pub fn new() -> Self {
        Self
    }
}

impl AgentBackend for CodexBackend {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn is_available(&self) -> Result<bool, String> {
        Ok(std::process::Command::new("codex")
            .arg("--version")
            .output()
            .is_ok())
    }

    fn description(&self) -> String {
        "OpenAI Codex CLI (ChatGPT Pro subscription)".to_string()
    }

    fn run_oneshot(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<String, String> {
        Err("Codex backend not yet fully extracted in PR 1. Using legacy path for now.".to_string())
    }

    fn run_builder(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<String, String> {
        self.run_oneshot(_model, _prompt, _timeout_secs)
    }

    fn run_oneshot_streaming(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<StreamingProcess, String> {
        Err("Codex streaming not yet extracted".to_string())
    }

    fn run_builder_streaming(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<StreamingProcess, String> {
        self.run_oneshot_streaming(_model, _prompt, _timeout_secs)
    }
}

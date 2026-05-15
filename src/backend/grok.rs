//! Grok Build native backend (stub for PR 1).
//!
//! When running inside a Grok Build TUI session, this backend will eventually
//! drive planner/builder/evaluator using subagents, todo_write, plan_mode,
//! direct SCL + GitHub MCP tools, and rich tracing — without spawning an external CLI.

use super::StreamingProcess;
use super::AgentBackend;

pub struct GrokBackend;

impl GrokBackend {
    pub fn new() -> Self {
        Self
    }
}

impl AgentBackend for GrokBackend {
    fn name(&self) -> &'static str {
        "grok"
    }

    fn is_available(&self) -> Result<bool, String> {
        // In a real Grok Build session we are always "available" when the env is detected.
        // For now we just report whether we look like we're inside the Grok TUI.
        let in_grok = std::env::var("GROK_SESSION_ID").is_ok()
            || std::path::Path::new("~/.grok").exists()
            || std::env::current_exe()
                .map(|p| p.to_string_lossy().contains("grok"))
                .unwrap_or(false);

        Ok(in_grok)
    }

    fn description(&self) -> String {
        "Grok Build (native — subagents, plan mode, SCL, GitHub MCP)".to_string()
    }

    fn run_oneshot(&self, _model: &str, _prompt: &str, _timeout_secs: u64) -> Result<String, String> {
        Err(self.not_implemented_message())
    }

    fn run_builder(&self, _model: &str, _prompt: &str, _timeout_secs: u64) -> Result<String, String> {
        Err(self.not_implemented_message())
    }

    fn run_oneshot_streaming(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<StreamingProcess, String> {
        Err(self.not_implemented_message())
    }

    fn run_builder_streaming(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<StreamingProcess, String> {
        Err(self.not_implemented_message())
    }
}

impl GrokBackend {
    fn not_implemented_message(&self) -> String {
        "Grok-native backend is not yet fully implemented (PR 1 only added the trait).\n\n\
         You are seeing this because you ran `harness ... --backend grok`.\n\n\
         Recommended right now:\n\
         1. Run your harness commands directly from inside this Grok Build TUI session.\n\
         2. Use `/always-approve on` (or Ctrl+O) for YOLO mode.\n\
         3. The full Grok-native experience (subagents for parallel planner/builder/evaluator, \
         plan_mode for complex refactors, rich SCL recording with relates_to, native GitHub MCP \
         for branches/PRs, todo tracking, etc.) is being built in the subsequent PRs.\n\n\
         See the Grok-Native Harness design (L0 architecture decision CID 42346e23c1ff9c69 in SCL).\n\
         Full design document is in the session history.".to_string()
    }
}

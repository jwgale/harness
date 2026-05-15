//! Mock backend for testing (instant responses, no real CLI).

use super::AgentBackend;
use super::StreamingProcess;

pub struct MockBackend;

impl MockBackend {
    pub fn new() -> Self {
        Self
    }

    fn mock_response(phase: &str) -> String {
        format!(
            "# Mock {phase} response\n\nThis is a mock response for testing.\n\nVERDICT: PASS\n\nSCORES:\n  functionality: 8/10\n  completeness: 8/10\n  code_quality: 8/10\n  design_quality: 7/10\n  robustness: 7/10\n"
        )
    }
}

impl AgentBackend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn is_available(&self) -> Result<bool, String> {
        Ok(true)
    }

    fn description(&self) -> String {
        "Mock backend (instant responses for testing)".to_string()
    }

    fn run_oneshot(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<String, String> {
        Ok(Self::mock_response("oneshot"))
    }

    fn run_builder(
        &self,
        _model: &str,
        _prompt: &str,
        _timeout_secs: u64,
    ) -> Result<String, String> {
        Ok(Self::mock_response("builder"))
    }

    fn run_oneshot_streaming(
        &self,
        model: &str,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<StreamingProcess, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let response = Self::mock_response("oneshot");
        let _ = tx.send(response.clone());

        Ok(StreamingProcess::new(
            rx,
            Box::new(()),
            Box::new(|| Ok(())),
            Box::new(move || Ok(response)),
        ))
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

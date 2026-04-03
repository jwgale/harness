use crate::artifacts;
use crate::cli_backend::{self, Backend};
use crate::config::Config;
use crate::plugins::{PluginManager, HookPoint};
use crate::prompts;
use crate::scl_lifecycle;

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Pass,
    Revise,
    Fail,
}

pub fn run(backend_override: Option<&str>) -> Result<Verdict, String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let backend = Backend::from_str(backend_override.unwrap_or(&config.backend))?;
    let pm = PluginManager::load();

    if !artifacts::artifact_exists("spec.md") {
        return Err("No spec.md found. Run `harness plan` first.".to_string());
    }

    pm.fire(HookPoint::BeforeEvaluate);
    println!("Running evaluator...");
    let prompt = prompts::evaluator_prompt()?;
    let output = cli_backend::run_oneshot(&backend, &config.model, &prompt, config.evaluator_timeout_seconds)?;

    // Save evaluation
    artifacts::write_artifact("evaluation.md", &output)?;

    // Save as feedback round
    let round = artifacts::next_feedback_number();
    artifacts::write_artifact(&format!("feedback/round-{round:03}.md"), &output)?;

    // Parse verdict
    let verdict = parse_verdict(&output);
    pm.fire(HookPoint::AfterEvaluate);
    scl_lifecycle::record_eval_complete(&config.project_name, round, &format!("{verdict:?}"), "");
    println!("Evaluation written to .harness/evaluation.md");
    println!("Verdict: {verdict:?}");

    Ok(verdict)
}

pub fn parse_verdict(output: &str) -> Verdict {
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("VERDICT:") {
            let v = rest.trim().to_uppercase();
            return match v.as_str() {
                "PASS" => Verdict::Pass,
                "FAIL" => Verdict::Fail,
                _ => Verdict::Revise,
            };
        }
    }
    // If we can't parse a verdict, treat as REVISE (conservative)
    Verdict::Revise
}

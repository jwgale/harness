use crate::artifacts;
use crate::cli_backend::Backend;
use crate::config::Config;
use crate::evaluator;
use crate::notifications;
use crate::plugins::{HookPoint, PluginManager};
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
    println!(
        "Running evaluator (strategy: {})...",
        config.evaluator_strategy
    );
    let output = evaluator::run_strategy(&config, &backend)?;

    // Save evaluation
    artifacts::write_artifact("evaluation.md", &output)?;

    // Save as feedback round
    let round = artifacts::next_feedback_number();
    artifacts::write_artifact(&format!("feedback/round-{round:03}.md"), &output)?;

    // Parse verdict
    let verdict = parse_verdict(&output);
    pm.fire(HookPoint::AfterEvaluate);
    scl_lifecycle::record_eval_complete(
        &config.project_name,
        round,
        &format!("{verdict:?}"),
        &config.evaluator_strategy,
    );

    // Fire notification events
    notifications::fire_eval_event(&verdict, &config.project_name, round);

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

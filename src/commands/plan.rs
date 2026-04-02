use crate::artifacts;
use crate::cli_backend::{self, Backend};
use crate::config::Config;
use crate::prompts;

pub fn run(backend_override: Option<&str>) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let goal = artifacts::read_artifact("goal.md")?;
    let backend = Backend::from_str(backend_override.unwrap_or(&config.backend))?;

    println!("Running planner...");
    let prompt = prompts::planner_prompt(&goal);
    let output = cli_backend::run_oneshot(&backend, &config.model, &prompt, config.evaluator_timeout_seconds)?;

    artifacts::write_artifact("spec.md", &output)?;

    println!("Spec written to .harness/spec.md");
    println!("Review and edit the spec, then run `harness build` to start building.");

    Ok(())
}

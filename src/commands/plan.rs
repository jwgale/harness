use crate::artifacts;
use crate::cli_backend::{self, Backend};
use crate::config::Config;
use crate::plugins::{PluginManager, HookPoint};
use crate::prompts;
use crate::scl_lifecycle;

pub fn run(backend_override: Option<&str>) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let goal = artifacts::read_artifact("goal.md")?;
    let backend = Backend::from_str(backend_override.unwrap_or(&config.backend))?;
    let pm = PluginManager::load();

    pm.fire(HookPoint::BeforePlan);
    println!("Running planner...");
    let prompt = prompts::planner_prompt(&goal);
    let output = cli_backend::run_oneshot(&backend, &config.model, &prompt, config.evaluator_timeout_seconds)?;

    artifacts::write_artifact("spec.md", &output)?;
    pm.fire(HookPoint::AfterPlan);
    scl_lifecycle::record_plan_complete(&config.project_name);

    println!("Spec written to .harness/spec.md");
    println!("Review and edit the spec, then run `harness build` to start building.");

    Ok(())
}

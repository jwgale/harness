use crate::artifacts;
use crate::backend::get_backend;
use crate::cli_backend::Backend; // still used for from_str during transition
use crate::config::Config;
use crate::plugins::{HookPoint, PluginManager};
use crate::prompts;
use crate::scl_lifecycle;

pub fn run(backend_override: Option<&str>) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let goal = artifacts::read_artifact("goal.md")?;
    let _ = Backend::from_str(backend_override.unwrap_or(&config.backend))?; // validation
    let agent = get_backend(backend_override.unwrap_or(&config.backend))?;
    let pm = PluginManager::load();

    pm.fire(HookPoint::BeforePlan);
    println!("Running planner...");
    let prompt = prompts::planner_prompt(&goal);
    let output = agent.run_oneshot(&config.model, &prompt, config.evaluator_timeout_seconds)?;

    artifacts::write_artifact("spec.md", &output)?;
    pm.fire(HookPoint::AfterPlan);
    scl_lifecycle::record_plan_complete(&config.project_name);

    println!("Spec written to .harness/spec.md");
    println!("Review and edit the spec, then run `harness build` to start building.");

    Ok(())
}

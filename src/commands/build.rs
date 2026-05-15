use crate::artifacts;
use crate::backend::get_backend;
use crate::cli_backend::Backend;
use crate::config::Config;
use crate::plugins::{HookPoint, PluginManager};
use crate::prompts;
use crate::scl_lifecycle;

pub fn run(backend_override: Option<&str>) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let _ = Backend::from_str(backend_override.unwrap_or(&config.backend))?;
    let agent = get_backend(backend_override.unwrap_or(&config.backend))?;
    let pm = PluginManager::load();

    if !artifacts::artifact_exists("spec.md") {
        return Err("No spec.md found. Run `harness plan` first.".to_string());
    }

    pm.fire(HookPoint::BeforeBuild);
    println!("Running builder...");
    let prompt = prompts::builder_prompt()?;
    let output = agent.run_builder(&config.model, &prompt, config.builder_timeout_seconds)?;
    pm.fire(HookPoint::AfterBuild);
    scl_lifecycle::record_build_complete(&config.project_name, 1);

    println!("Builder finished.");
    if !output.is_empty() {
        println!("Builder output:\n{output}");
    }

    Ok(())
}

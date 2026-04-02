use crate::artifacts;
use crate::cli_backend::{self, Backend};
use crate::config::Config;
use crate::prompts;

pub fn run(backend_override: Option<&str>) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let backend = Backend::from_str(backend_override.unwrap_or(&config.backend))?;

    if !artifacts::artifact_exists("spec.md") {
        return Err("No spec.md found. Run `harness plan` first.".to_string());
    }

    println!("Running builder...");
    let prompt = prompts::builder_prompt()?;
    let output = cli_backend::run_builder(&backend, &config.model, &prompt, config.builder_timeout_seconds)?;

    println!("Builder finished.");
    if !output.is_empty() {
        println!("Builder output:\n{output}");
    }

    Ok(())
}

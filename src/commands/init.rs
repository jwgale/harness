use crate::artifacts;
use crate::config::Config;

pub fn run(goal: &str) -> Result<(), String> {
    artifacts::init_harness_dir()?;

    // Derive project name from current directory
    let project_name = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unnamed-project".to_string());

    let config = Config::new(&project_name);
    config.save(&artifacts::harness_dir())?;

    artifacts::write_artifact("goal.md", goal)?;
    artifacts::write_artifact("status.md", "# Build Status\n\nNo build started yet.\n")?;

    println!("Initialized .harness/ for project: {project_name}");
    println!("Goal: {goal}");
    println!("\nNext: run `harness plan` to generate a spec from this goal.");

    Ok(())
}

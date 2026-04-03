use crate::artifacts;
use crate::config::Config;
use crate::evaluator;

/// List all available evaluator strategies.
pub fn list() -> Result<(), String> {
    println!("Available evaluator strategies:\n");
    for &name in evaluator::STRATEGIES {
        let desc = evaluator::describe(name);
        let marker = if is_current(name) { " (active)" } else { "" };
        println!("  {name}{marker}");
        println!("    {desc}");
        println!();
    }

    if let Ok(config) = load_config() {
        println!("Current strategy: {}", config.evaluator_strategy);
    } else {
        println!("No .harness/ workspace found. Run `harness init` to configure.");
    }

    Ok(())
}

/// Set the evaluator strategy for this workspace.
pub fn use_strategy(name: &str) -> Result<(), String> {
    if !evaluator::is_valid_strategy(name) {
        return Err(format!(
            "Unknown strategy: '{name}'. Available: {}",
            evaluator::STRATEGIES.join(", ")
        ));
    }

    artifacts::ensure_harness_exists()?;
    let mut config = Config::load(&artifacts::harness_dir())?;
    let old = config.evaluator_strategy.clone();
    config.evaluator_strategy = name.to_string();
    config.save(&artifacts::harness_dir())?;

    if old == name {
        println!("Evaluator strategy already set to '{name}'.");
    } else {
        println!("Evaluator strategy changed: {old} -> {name}");
    }

    if name == "curl" {
        let ep_path = artifacts::harness_dir().join("endpoints.json");
        if !ep_path.exists() {
            println!();
            println!("Tip: Create .harness/endpoints.json with URLs to check:");
            println!("  [\"http://localhost:3000\", \"http://localhost:3000/api/health\"]");
        }
    }

    Ok(())
}

fn load_config() -> Result<Config, String> {
    artifacts::ensure_harness_exists()?;
    Config::load(&artifacts::harness_dir())
}

fn is_current(name: &str) -> bool {
    load_config()
        .map(|c| c.evaluator_strategy == name)
        .unwrap_or(false)
}

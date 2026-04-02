use crate::artifacts;

pub fn run() -> Result<(), String> {
    artifacts::ensure_harness_exists()?;

    // Goal
    if let Ok(goal) = artifacts::read_artifact("goal.md") {
        println!("Goal: {goal}");
    }

    // Config
    if let Ok(config) = crate::config::Config::load(&artifacts::harness_dir()) {
        println!("Backend: {}", config.backend);
        println!("Max rounds: {}", config.max_eval_rounds);
    }

    // Spec
    if artifacts::artifact_exists("spec.md") {
        println!("\nSpec: .harness/spec.md (exists)");
    } else {
        println!("\nSpec: not yet generated (run `harness plan`)");
    }

    // Status
    if artifacts::artifact_exists("status.md") {
        let status = artifacts::read_artifact("status.md")?;
        println!("\n--- Build Status ---");
        println!("{status}");
    }

    // Latest evaluation
    if artifacts::artifact_exists("evaluation.md") {
        println!("Latest evaluation: .harness/evaluation.md (exists)");
    }

    // Feedback rounds
    let rounds = artifacts::next_feedback_number() - 1;
    if rounds > 0 {
        println!("Feedback rounds completed: {rounds}");
    }

    Ok(())
}

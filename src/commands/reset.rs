use crate::artifacts;

pub fn run() -> Result<(), String> {
    artifacts::ensure_harness_exists()?;

    let mut handoff = String::from("# Handoff Brief\n\n");
    handoff.push_str(&format!(
        "Generated: {}\n\n",
        chrono::Utc::now().to_rfc3339()
    ));

    // Goal
    if let Ok(goal) = artifacts::read_artifact("goal.md") {
        handoff.push_str(&format!("## Goal\n\n{goal}\n\n"));
    }

    // Spec summary
    if artifacts::artifact_exists("spec.md") {
        handoff.push_str("## Spec\n\nSee .harness/spec.md for full specification.\n\n");
    }

    // Current status
    if let Ok(status) = artifacts::read_artifact("status.md") {
        handoff.push_str(&format!("## Current Status\n\n{status}\n\n"));
    }

    // Latest evaluation
    if let Ok(eval) = artifacts::read_artifact("evaluation.md") {
        handoff.push_str(&format!("## Latest Evaluation\n\n{eval}\n\n"));
    }

    // File listing
    let files = artifacts::list_project_files();
    if !files.is_empty() {
        handoff.push_str(&format!("## Project Files\n\n```\n{files}\n```\n"));
    }

    artifacts::write_artifact("handoff.md", &handoff)?;

    println!("Handoff brief written to .harness/handoff.md");
    println!("Use this to brief a fresh session if context reset is needed.");

    Ok(())
}

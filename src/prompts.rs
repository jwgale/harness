use crate::artifacts;

const PLANNER_TEMPLATE: &str = include_str!("../prompts/planner.md");
const BUILDER_TEMPLATE: &str = include_str!("../prompts/builder.md");
const EVALUATOR_TEMPLATE: &str = include_str!("../prompts/evaluator.md");

/// Assemble the full planner prompt: template + goal
pub fn planner_prompt(goal: &str) -> String {
    format!("{PLANNER_TEMPLATE}\n\n---\n\n## Goal\n\n{goal}\n")
}

/// Assemble the full builder prompt: template + spec + feedback (if any)
pub fn builder_prompt() -> Result<String, String> {
    let spec = artifacts::read_artifact("spec.md")?;

    let mut prompt = format!("{BUILDER_TEMPLATE}\n\n---\n\n## Specification\n\n{spec}\n");

    // Include latest feedback if we're in a revision loop
    let feedback_num = artifacts::next_feedback_number();
    if feedback_num > 1 {
        let latest = format!("feedback/round-{:03}.md", feedback_num - 1);
        if let Ok(feedback) = artifacts::read_artifact(&latest) {
            prompt.push_str(&format!("\n---\n\n## Evaluator Feedback (Round {})\n\n{feedback}\n", feedback_num - 1));
        }
    }

    // Include project file listing for context
    let files = artifacts::list_project_files();
    if !files.is_empty() {
        prompt.push_str(&format!("\n---\n\n## Current Project Files\n\n```\n{files}\n```\n"));
    }

    Ok(prompt)
}

/// Assemble the full evaluator prompt: template + spec + status + file listing
pub fn evaluator_prompt() -> Result<String, String> {
    let spec = artifacts::read_artifact("spec.md")?;

    let mut prompt = format!("{EVALUATOR_TEMPLATE}\n\n---\n\n## Specification\n\n{spec}\n");

    if artifacts::artifact_exists("status.md") {
        let status = artifacts::read_artifact("status.md")?;
        prompt.push_str(&format!("\n---\n\n## Builder Status Report\n\n{status}\n"));
    }

    let files = artifacts::list_project_files();
    if !files.is_empty() {
        prompt.push_str(&format!("\n---\n\n## Current Project Files\n\n```\n{files}\n```\n"));
    }

    Ok(prompt)
}

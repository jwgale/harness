use crate::artifacts;

const PLANNER_TEMPLATE: &str = include_str!("../prompts/planner.md");
const BUILDER_TEMPLATE: &str = include_str!("../prompts/builder.md");
const EVALUATOR_TEMPLATE: &str = include_str!("../prompts/evaluator.md");

/// Load a prompt template, checking for user override in .harness/prompts/ first.
fn load_template(name: &str, default: &str) -> String {
    let override_path = artifacts::harness_dir().join("prompts").join(name);
    if override_path.exists()
        && let Ok(content) = std::fs::read_to_string(&override_path)
    {
        return content;
    }
    default.to_string()
}

/// Assemble the full planner prompt: template + goal
pub fn planner_prompt(goal: &str) -> String {
    let template = load_template("planner.md", PLANNER_TEMPLATE);
    format!("{template}\n\n---\n\n## Goal\n\n{goal}\n")
}

/// Assemble the full builder prompt: template + spec + feedback (if any)
pub fn builder_prompt() -> Result<String, String> {
    let template = load_template("builder.md", BUILDER_TEMPLATE);
    let spec = artifacts::read_artifact("spec.md")?;

    let mut prompt = format!("{template}\n\n---\n\n## Specification\n\n{spec}\n");

    // Include latest feedback if we're in a revision loop
    let feedback_num = artifacts::next_feedback_number();
    if feedback_num > 1 {
        let latest = format!("feedback/round-{:03}.md", feedback_num - 1);
        if let Ok(feedback) = artifacts::read_artifact(&latest) {
            prompt.push_str(&format!(
                "\n---\n\n## Evaluator Feedback (Round {})\n\n{feedback}\n",
                feedback_num - 1
            ));
        }
    }

    // Include project file listing for context
    let files = artifacts::list_project_files();
    if !files.is_empty() {
        prompt.push_str(&format!(
            "\n---\n\n## Current Project Files\n\n```\n{files}\n```\n"
        ));
    }

    Ok(prompt)
}

/// Assemble the full evaluator prompt: template + spec + status + file listing
pub fn evaluator_prompt() -> Result<String, String> {
    let template = load_template("evaluator.md", EVALUATOR_TEMPLATE);
    let spec = artifacts::read_artifact("spec.md")?;

    let mut prompt = format!("{template}\n\n---\n\n## Specification\n\n{spec}\n");

    if artifacts::artifact_exists("status.md") {
        let status = artifacts::read_artifact("status.md")?;
        prompt.push_str(&format!("\n---\n\n## Builder Status Report\n\n{status}\n"));
    }

    let files = artifacts::list_project_files();
    if !files.is_empty() {
        prompt.push_str(&format!(
            "\n---\n\n## Current Project Files\n\n```\n{files}\n```\n"
        ));
    }

    Ok(prompt)
}

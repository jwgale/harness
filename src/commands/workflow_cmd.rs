use crate::workflows;
use crate::xdg;

/// List all defined workflows.
pub fn list() -> Result<(), String> {
    xdg::ensure_dirs()?;
    let wfs = workflows::discover();

    if wfs.is_empty() {
        println!("No workflows defined.\n");
        println!("Define workflows in: {}\n", xdg::workflows_dir().display());
        println!("Example:");
        println!("  # ~/.config/harness/workflows/standard.toml");
        println!("  name = \"standard\"");
        println!("  description = \"Plan, build, evaluate\"");
        println!("  [[steps]]");
        println!("  agent = \"my-planner\"");
        println!("  [[steps]]");
        println!("  agent = \"my-builder\"");
        println!("  loop_until = \"pass\"");
        println!("  [[steps]]");
        println!("  agent = \"my-evaluator\"");
        return Ok(());
    }

    println!("Defined workflows ({}):\n", wfs.len());
    for wf in &wfs {
        let desc = wf.description.as_deref().unwrap_or("(no description)");
        let step_names: Vec<&str> = wf.steps.iter().map(|s| s.agent.as_str()).collect();
        let parallel_count = wf.steps.iter().filter(|s| s.parallel).count();
        let loop_count = wf.steps.iter().filter(|s| s.loop_until.is_some()).count();

        println!("  {}", wf.name);
        println!("    {desc}");
        println!("    steps: {}", step_names.join(" -> "));
        if parallel_count > 0 {
            println!("    parallel steps: {parallel_count}");
        }
        if loop_count > 0 {
            println!("    iterative loops: {loop_count}");
        }
        println!();
    }

    Ok(())
}

/// Validate a workflow definition.
pub fn validate(name: &str) -> Result<(), String> {
    let wf = workflows::load(name)?;
    let errors = workflows::validate(&wf);

    if errors.is_empty() {
        println!("Workflow '{}' is valid.", wf.name);
        let groups = workflows::plan_execution(&wf);
        println!("\nExecution plan ({} groups):", groups.len());
        for (i, group) in groups.iter().enumerate() {
            match group {
                workflows::StepGroup::Single(step) => {
                    println!("  {}. [sequential] agent '{}'", i + 1, step.agent);
                }
                workflows::StepGroup::Parallel(steps) => {
                    let names: Vec<&str> = steps.iter().map(|s| s.agent.as_str()).collect();
                    println!("  {}. [parallel] agents: {}", i + 1, names.join(", "));
                }
                workflows::StepGroup::Loop { body, evaluator, max_rounds } => {
                    let body_names: Vec<&str> = body.iter().map(|s| s.agent.as_str()).collect();
                    println!(
                        "  {}. [loop max={max_rounds}] body: {} -> evaluator: '{}'",
                        i + 1,
                        body_names.join(", "),
                        evaluator.agent
                    );
                }
            }
        }
        Ok(())
    } else {
        println!("Workflow '{}' has {} error(s):\n", wf.name, errors.len());
        for err in &errors {
            println!("  - {err}");
        }
        Err(format!("Workflow validation failed with {} error(s)", errors.len()))
    }
}

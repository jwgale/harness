use crate::agents;
use crate::xdg;

/// List all defined agents.
pub fn list() -> Result<(), String> {
    xdg::ensure_dirs()?;
    let agents = agents::discover();

    if agents.is_empty() {
        println!("No agents defined.\n");
        println!("Define agents in: {}\n", xdg::agents_dir().display());
        println!("Quick start:");
        println!("  harness agent add my-planner --role planner --backend claude");
        println!("  harness agent add my-builder --role builder --backend claude");
        println!("  harness agent add my-evaluator --role evaluator --backend claude");
        return Ok(());
    }

    println!("Defined agents ({}):\n", agents.len());
    for agent in &agents {
        let desc = agent
            .description
            .as_deref()
            .unwrap_or(agents::role_description(&agent.role));
        let model_info = agent.model.as_deref().unwrap_or("(project default)");
        println!("  {} [{}]", agent.name, agent.role);
        println!("    {desc}");
        println!("    backend: {}, model: {model_info}", agent.backend);
        if let Some(tools) = &agent.tools {
            println!("    tools: {}", tools.join(", "));
        }
        let specializations = agent.specialization_tags();
        if !specializations.is_empty() {
            println!("    specializations: {}", specializations.join(", "));
        }
        let defaults = agent.default_for_tags();
        if !defaults.is_empty() {
            println!("    default for: {}", defaults.join(", "));
        }
        let context_scopes = agent.context_scope_tags();
        if !context_scopes.is_empty() {
            println!("    context scopes: {}", context_scopes.join(", "));
        }
        if let Some(timeout) = agent.timeout_seconds {
            println!("    timeout: {timeout}s");
        }
        println!();
    }

    Ok(())
}

/// Add a new agent definition.
pub fn add(
    name: &str,
    role: &str,
    backend: &str,
    description: Option<&str>,
    specializations: Option<&str>,
    context_scopes: Option<&str>,
    default_for: Option<&str>,
) -> Result<(), String> {
    // Validate role
    let valid_roles = ["planner", "builder", "evaluator", "custom"];
    if !valid_roles.contains(&role) {
        return Err(format!(
            "Invalid role: '{role}'. Use one of: {}",
            valid_roles.join(", ")
        ));
    }

    // Validate backend
    let valid_backends = ["claude", "codex", "mock"];
    if !valid_backends.contains(&backend) {
        return Err(format!(
            "Invalid backend: '{backend}'. Use one of: {}",
            valid_backends.join(", ")
        ));
    }

    agents::add(
        name,
        role,
        backend,
        description,
        parse_csv(specializations),
        parse_csv(context_scopes),
        parse_csv(default_for),
    )?;
    println!("Agent '{name}' created ({role}, {backend}).");
    println!(
        "Edit: {}",
        xdg::agents_dir().join(format!("{name}.toml")).display()
    );

    Ok(())
}

/// Remove an agent definition.
pub fn remove(name: &str) -> Result<(), String> {
    agents::remove(name)?;
    println!("Agent '{name}' removed.");
    Ok(())
}

fn parse_csv(value: Option<&str>) -> Option<Vec<String>> {
    let tags: Vec<String> = value?
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    if tags.is_empty() { None } else { Some(tags) }
}

use serde::Deserialize;
use std::fs;
use std::process::Command;

use crate::xdg;

#[derive(Debug, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    #[allow(dead_code)]
    pub backend: Option<String>,
    pub hooks: Option<PluginHooks>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PluginHooks {
    pub before_plan: Option<String>,
    pub after_plan: Option<String>,
    pub before_build: Option<String>,
    pub after_build: Option<String>,
    pub before_evaluate: Option<String>,
    pub after_evaluate: Option<String>,
}

/// Hook points in the harness lifecycle.
#[derive(Debug, Clone, Copy)]
pub enum HookPoint {
    BeforePlan,
    AfterPlan,
    BeforeBuild,
    AfterBuild,
    BeforeEvaluate,
    AfterEvaluate,
}

impl HookPoint {
    pub fn label(self) -> &'static str {
        match self {
            HookPoint::BeforePlan => "before_plan",
            HookPoint::AfterPlan => "after_plan",
            HookPoint::BeforeBuild => "before_build",
            HookPoint::AfterBuild => "after_build",
            HookPoint::BeforeEvaluate => "before_evaluate",
            HookPoint::AfterEvaluate => "after_evaluate",
        }
    }
}

/// Manages loaded plugins and fires hooks at lifecycle points.
pub struct PluginManager {
    plugins: Vec<PluginManifest>,
}

impl PluginManager {
    /// Load all plugins from the plugins directory.
    pub fn load() -> Self {
        let plugins = discover();
        if !plugins.is_empty() {
            eprintln!("Loaded {} plugin(s)", plugins.len());
        }
        Self { plugins }
    }

    /// Fire a hook point — execute registered commands for each plugin.
    pub fn fire(&self, point: HookPoint) {
        let label = point.label();
        let project_name = detect_project_name();

        for plugin in &self.plugins {
            let cmd_str = plugin.hooks.as_ref().and_then(|h| match point {
                HookPoint::BeforePlan => h.before_plan.as_deref(),
                HookPoint::AfterPlan => h.after_plan.as_deref(),
                HookPoint::BeforeBuild => h.before_build.as_deref(),
                HookPoint::AfterBuild => h.after_build.as_deref(),
                HookPoint::BeforeEvaluate => h.before_evaluate.as_deref(),
                HookPoint::AfterEvaluate => h.after_evaluate.as_deref(),
            });
            if let Some(cmd_str) = cmd_str {
                eprintln!("[plugin:{}] {label} -> `{cmd_str}`", plugin.name);
                execute_hook(&plugin.name, label, cmd_str, &project_name);
            }
        }
    }

    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.plugins.len()
    }
}

/// Execute a hook command with environment variables.
fn execute_hook(plugin_name: &str, hook_label: &str, cmd_str: &str, project_name: &str) {
    let cwd = std::env::current_dir().unwrap_or_default();

    let result = Command::new("sh")
        .args(["-c", cmd_str])
        .env("HARNESS_HOOK", hook_label)
        .env("HARNESS_PLUGIN", plugin_name)
        .env("HARNESS_PROJECT", project_name)
        .env("HARNESS_DIR", cwd.join(".harness").to_string_lossy().as_ref())
        .env("HARNESS_PLUGINS_DIR", xdg::plugins_dir().to_string_lossy().as_ref())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stdout.trim().is_empty() {
                for line in stdout.lines() {
                    eprintln!("[plugin:{plugin_name}] {line}");
                }
            }
            if !stderr.trim().is_empty() {
                for line in stderr.lines() {
                    eprintln!("[plugin:{plugin_name}] stderr: {line}");
                }
            }
            if !output.status.success() {
                eprintln!("[plugin:{plugin_name}] hook {hook_label} exited with {}", output.status);
            }
        }
        Err(e) => {
            eprintln!("[plugin:{plugin_name}] hook {hook_label} failed to execute: {e}");
        }
    }
}

fn detect_project_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Discover all plugin manifests in ~/.config/harness/plugins/*.toml
pub fn discover() -> Vec<PluginManifest> {
    let dir = xdg::plugins_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut plugins = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml")
            && let Ok(content) = fs::read_to_string(&path)
        {
            match toml::from_str::<PluginManifest>(&content) {
                Ok(manifest) => plugins.push(manifest),
                Err(e) => {
                    eprintln!("Warning: failed to parse plugin {}: {e}", path.display());
                }
            }
        }
    }
    plugins
}

/// List discovered plugins (for CLI output).
pub fn list() -> Result<(), String> {
    xdg::ensure_dirs()?;
    let plugins = discover();

    if plugins.is_empty() {
        println!("No plugins installed.");
        println!();
        println!("Place plugin manifests in: {}", xdg::plugins_dir().display());
        println!();
        println!("Example plugin (example.toml):");
        println!("  name = \"my-plugin\"");
        println!("  description = \"A custom harness plugin\"");
        println!("  version = \"0.1.0\"");
        println!();
        println!("  [hooks]");
        println!("  after_build = \"cargo test\"");
        return Ok(());
    }

    println!("Installed plugins ({}):", plugins.len());
    for p in &plugins {
        let desc = p.description.as_deref().unwrap_or("(no description)");
        let ver = p.version.as_deref().unwrap_or("?");
        println!("  {} v{} — {}", p.name, ver, desc);
        print_hooks(&p.hooks);
    }
    Ok(())
}

fn print_hooks(hooks: &Option<PluginHooks>) {
    let Some(h) = hooks else { return };
    let entries: Vec<(&str, &Option<String>)> = vec![
        ("before_plan", &h.before_plan),
        ("after_plan", &h.after_plan),
        ("before_build", &h.before_build),
        ("after_build", &h.after_build),
        ("before_evaluate", &h.before_evaluate),
        ("after_evaluate", &h.after_evaluate),
    ];
    for (label, cmd) in entries {
        if let Some(cmd) = cmd {
            println!("    {label}: `{cmd}`");
        }
    }
}

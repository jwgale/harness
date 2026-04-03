use serde::Deserialize;
use std::fs;

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
    fn label(self) -> &'static str {
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

    /// Fire a hook point — logs which plugins have hooks registered.
    /// Actual execution will come in a later phase.
    pub fn fire(&self, point: HookPoint) {
        let label = point.label();
        for plugin in &self.plugins {
            let cmd = plugin.hooks.as_ref().and_then(|h| match point {
                HookPoint::BeforePlan => h.before_plan.as_deref(),
                HookPoint::AfterPlan => h.after_plan.as_deref(),
                HookPoint::BeforeBuild => h.before_build.as_deref(),
                HookPoint::AfterBuild => h.after_build.as_deref(),
                HookPoint::BeforeEvaluate => h.before_evaluate.as_deref(),
                HookPoint::AfterEvaluate => h.after_evaluate.as_deref(),
            });
            if let Some(cmd) = cmd {
                eprintln!("[plugin:{}] hook {label} -> `{cmd}` (not yet executed)", plugin.name);
            }
        }
    }

    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.plugins.len()
    }
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
        let hook_count = count_hooks(&p.hooks);
        println!("  {} v{} — {} ({} hooks)", p.name, ver, desc, hook_count);
    }
    Ok(())
}

fn count_hooks(hooks: &Option<PluginHooks>) -> usize {
    let Some(h) = hooks else { return 0 };
    [
        &h.before_plan, &h.after_plan,
        &h.before_build, &h.after_build,
        &h.before_evaluate, &h.after_evaluate,
    ].iter().filter(|x| x.is_some()).count()
}

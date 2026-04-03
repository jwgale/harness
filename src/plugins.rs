use serde::Deserialize;
use std::fs;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use crate::xdg;

const DEFAULT_HOOK_TIMEOUT: u64 = 30;

#[derive(Debug, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    #[allow(dead_code)]
    pub backend: Option<String>,
    /// Global timeout for all hooks in this plugin (seconds). Default: 30.
    pub timeout_seconds: Option<u64>,
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

/// Output sink for hook messages. In TUI mode, sends to the output panel.
/// In plain mode, prints to stderr.
pub enum HookOutput {
    Stderr,
    Channel(mpsc::Sender<String>),
}

impl HookOutput {
    fn send(&self, msg: &str) {
        match self {
            HookOutput::Stderr => eprintln!("{msg}"),
            HookOutput::Channel(tx) => { let _ = tx.send(msg.to_string()); }
        }
    }
}

/// Manages loaded plugins and fires hooks at lifecycle points.
pub struct PluginManager {
    plugins: Vec<PluginManifest>,
    output: HookOutput,
}

impl PluginManager {
    /// Load all plugins, output to stderr.
    pub fn load() -> Self {
        let plugins = discover();
        if !plugins.is_empty() {
            eprintln!("Loaded {} plugin(s)", plugins.len());
        }
        Self { plugins, output: HookOutput::Stderr }
    }

    /// Load all plugins, output to a TUI channel.
    pub fn load_with_channel(tx: mpsc::Sender<String>) -> Self {
        let plugins = discover();
        if !plugins.is_empty() {
            let _ = tx.send(format!("Loaded {} plugin(s)", plugins.len()));
        }
        Self { plugins, output: HookOutput::Channel(tx) }
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
                let timeout = plugin.timeout_seconds.unwrap_or(DEFAULT_HOOK_TIMEOUT);
                self.output.send(&format!("[plugin:{}] {label} -> `{cmd_str}`", plugin.name));
                self.execute_hook(&plugin.name, label, cmd_str, &project_name, timeout);
            }
        }
    }

    fn execute_hook(&self, plugin_name: &str, hook_label: &str, cmd_str: &str, project_name: &str, timeout_secs: u64) {
        let cwd = std::env::current_dir().unwrap_or_default();

        let mut child = match Command::new("sh")
            .args(["-c", cmd_str])
            .env("HARNESS_HOOK", hook_label)
            .env("HARNESS_PLUGIN", plugin_name)
            .env("HARNESS_PROJECT", project_name)
            .env("HARNESS_DIR", cwd.join(".harness").to_string_lossy().as_ref())
            .env("HARNESS_PLUGINS_DIR", xdg::plugins_dir().to_string_lossy().as_ref())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                self.output.send(&format!("[plugin:{plugin_name}] hook {hook_label} failed to spawn: {e}"));
                return;
            }
        };

        let timeout = Duration::from_secs(timeout_secs);
        let start = std::time::Instant::now();

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Process finished — collect output
                    let mut stdout_buf = Vec::new();
                    let mut stderr_buf = Vec::new();
                    if let Some(mut out) = child.stdout.take() {
                        let _ = std::io::Read::read_to_end(&mut out, &mut stdout_buf);
                    }
                    if let Some(mut err) = child.stderr.take() {
                        let _ = std::io::Read::read_to_end(&mut err, &mut stderr_buf);
                    }
                    let stdout = String::from_utf8_lossy(&stdout_buf);
                    let stderr = String::from_utf8_lossy(&stderr_buf);

                    for line in stdout.lines() {
                        if !line.trim().is_empty() {
                            self.output.send(&format!("[plugin:{plugin_name}] {line}"));
                        }
                    }
                    for line in stderr.lines() {
                        if !line.trim().is_empty() {
                            self.output.send(&format!("[plugin:{plugin_name}] stderr: {line}"));
                        }
                    }
                    if !status.success() {
                        self.output.send(&format!("[plugin:{plugin_name}] hook {hook_label} exited with {status}"));
                    }
                    return;
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        self.output.send(&format!(
                            "[plugin:{plugin_name}] hook {hook_label} killed after {timeout_secs}s timeout"
                        ));
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    self.output.send(&format!("[plugin:{plugin_name}] hook {hook_label} wait error: {e}"));
                    return;
                }
            }
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
        let timeout = p.timeout_seconds.unwrap_or(DEFAULT_HOOK_TIMEOUT);
        println!("  {} v{} — {} (timeout: {timeout}s)", p.name, ver, desc);
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

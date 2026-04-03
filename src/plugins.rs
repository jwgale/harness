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
    #[allow(dead_code)]
    pub hooks: Option<PluginHooks>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PluginHooks {
    pub before_plan: Option<String>,
    pub after_plan: Option<String>,
    pub before_build: Option<String>,
    pub after_build: Option<String>,
    pub before_evaluate: Option<String>,
    pub after_evaluate: Option<String>,
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
    }
    Ok(())
}

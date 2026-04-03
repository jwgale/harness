use std::fs;
use std::path::{Path, PathBuf};

use crate::xdg;

fn workspaces_dir() -> PathBuf {
    xdg::data_dir().join("workspaces")
}

pub fn register(path: Option<&str>) -> Result<(), String> {
    xdg::ensure_dirs()?;
    let ws_dir = workspaces_dir();
    fs::create_dir_all(&ws_dir)
        .map_err(|e| format!("Failed to create workspaces dir: {e}"))?;

    let abs_path = match path {
        Some(p) if p != "." => {
            if Path::new(p).is_absolute() {
                PathBuf::from(p)
            } else {
                std::env::current_dir()
                    .map_err(|e| format!("Failed to get current dir: {e}"))?
                    .join(p)
            }
        }
        _ => std::env::current_dir()
            .map_err(|e| format!("Failed to get current dir: {e}"))?,
    };

    let canon = abs_path.canonicalize()
        .map_err(|e| format!("Path does not exist: {}: {e}", abs_path.display()))?;

    // Derive a friendly name from the directory
    let name = canon.file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string());

    let ws_file = ws_dir.join(format!("{name}.path"));
    fs::write(&ws_file, canon.to_string_lossy().as_bytes())
        .map_err(|e| format!("Failed to write workspace file: {e}"))?;

    println!("Registered workspace: {name}");
    println!("  Path: {}", canon.display());
    println!("  File: {}", ws_file.display());

    if !canon.join(".harness").exists() {
        println!();
        println!("Note: No .harness/ directory found. Run `harness init` in that project first.");
    }

    Ok(())
}

pub fn list() -> Result<(), String> {
    xdg::ensure_dirs()?;
    let ws_dir = workspaces_dir();
    if !ws_dir.exists() {
        println!("No workspaces registered.");
        println!("Register one: harness workspace register <path>");
        return Ok(());
    }

    let entries: Vec<_> = fs::read_dir(&ws_dir)
        .map_err(|e| format!("Failed to read workspaces dir: {e}"))?
        .flatten()
        .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "path"))
        .collect();

    if entries.is_empty() {
        println!("No workspaces registered.");
        println!("Register one: harness workspace register <path>");
        return Ok(());
    }

    println!("Registered workspaces ({}):", entries.len());
    for entry in &entries {
        let name = entry.path()
            .file_stem()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let path = fs::read_to_string(entry.path()).unwrap_or_default();
        let path = path.trim();
        let has_harness = Path::new(path).join(".harness").exists();
        let status = if has_harness { "active" } else { "no .harness/" };
        println!("  {name}: {path} [{status}]");
    }
    Ok(())
}

pub fn remove(name: &str) -> Result<(), String> {
    xdg::ensure_dirs()?;
    let ws_file = workspaces_dir().join(format!("{name}.path"));

    if !ws_file.exists() {
        return Err(format!("Workspace '{name}' not found. Use `harness workspace list` to see registered workspaces."));
    }

    fs::remove_file(&ws_file)
        .map_err(|e| format!("Failed to remove workspace: {e}"))?;

    println!("Removed workspace: {name}");
    Ok(())
}

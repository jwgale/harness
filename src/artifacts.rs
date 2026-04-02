use std::fs;
use std::path::{Path, PathBuf};

pub fn harness_dir() -> PathBuf {
    PathBuf::from(".harness")
}

pub fn ensure_harness_exists() -> Result<(), String> {
    let dir = harness_dir();
    if !dir.exists() {
        return Err("No .harness/ directory found. Run `harness init` first.".to_string());
    }
    Ok(())
}

pub fn init_harness_dir() -> Result<(), String> {
    let dir = harness_dir();
    if dir.exists() {
        return Err(".harness/ already exists. Remove it first or use a different directory.".to_string());
    }
    for sub in &["feedback", "runs"] {
        fs::create_dir_all(dir.join(sub))
            .map_err(|e| format!("Failed to create .harness/{sub}: {e}"))?;
    }
    Ok(())
}

pub fn write_artifact(name: &str, content: &str) -> Result<(), String> {
    let path = harness_dir().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory for {name}: {e}"))?;
    }
    fs::write(&path, content)
        .map_err(|e| format!("Failed to write {name}: {e}"))
}

pub fn read_artifact(name: &str) -> Result<String, String> {
    let path = harness_dir().join(name);
    fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {name}: {e}"))
}

pub fn artifact_exists(name: &str) -> bool {
    harness_dir().join(name).exists()
}

pub fn next_run_number() -> u32 {
    let runs_dir = harness_dir().join("runs");
    if !runs_dir.exists() {
        return 1;
    }
    let mut max = 0u32;
    if let Ok(entries) = fs::read_dir(&runs_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // run-001.json -> 001 -> 1
            if let Some(num_str) = name.strip_prefix("run-").and_then(|s| s.strip_suffix(".json"))
                && let Ok(n) = num_str.parse::<u32>()
            {
                max = max.max(n);
            }
        }
    }
    max + 1
}

pub fn next_feedback_number() -> u32 {
    let dir = harness_dir().join("feedback");
    if !dir.exists() {
        return 1;
    }
    let mut max = 0u32;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(num_str) = name.strip_prefix("round-").and_then(|s| s.strip_suffix(".md"))
                && let Ok(n) = num_str.parse::<u32>()
            {
                max = max.max(n);
            }
        }
    }
    max + 1
}

pub fn list_project_files() -> String {
    let mut files = Vec::new();
    collect_files(Path::new("."), &mut files);
    files.join("\n")
}

fn collect_files(dir: &Path, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = path.to_string_lossy();
        // Skip hidden dirs, target, node_modules
        if let Some(fname) = path.file_name() {
            let fname = fname.to_string_lossy();
            if fname.starts_with('.') || fname == "target" || fname == "node_modules" {
                continue;
            }
        }
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(name.to_string());
        }
    }
}

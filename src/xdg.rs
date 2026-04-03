use std::fs;
use std::path::PathBuf;

/// Global config directory: ~/.config/harness/
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("harness")
}

/// Global data directory: ~/.local/share/harness/
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| home_dir().join(".local/share"))
        .join("harness")
}

/// Global cache directory: ~/.cache/harness/
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| home_dir().join(".cache"))
        .join("harness")
}

/// Plugins directory: ~/.config/harness/plugins/
pub fn plugins_dir() -> PathBuf {
    config_dir().join("plugins")
}

/// Ensure all XDG directories exist.
pub fn ensure_dirs() -> Result<(), String> {
    for dir in &[config_dir(), data_dir(), cache_dir(), plugins_dir()] {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    }
    Ok(())
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
}

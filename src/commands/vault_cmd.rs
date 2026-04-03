use crate::vault;
use crate::xdg;

/// Initialize vault configuration.
pub fn init() -> Result<(), String> {
    xdg::ensure_dirs()?;

    // Generate signing key if needed
    let pubkey = vault::public_key_hex()?;
    println!("Vault initialized for harness agent.");
    println!();
    println!("Agent public key: {pubkey}");
    println!();
    println!("Register this key with your SanctumAI vault:");
    println!("  sanctum agent register harness --pubkey {pubkey}");
    println!();

    // Ensure vault config exists in global config
    let config_path = xdg::config_dir().join("config.toml");
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path).unwrap_or_default();
        if !content.contains("[vault]") {
            let addition = r#"

[vault]
enabled = true
addr = "127.0.0.1:7600"
agent_name = "harness"
"#;
            std::fs::write(&config_path, format!("{content}{addition}"))
                .map_err(|e| format!("Failed to update config: {e}"))?;
            println!("Added [vault] section to {}", config_path.display());
        } else {
            println!("Vault config already exists in {}", config_path.display());
        }
    }

    Ok(())
}

/// Show vault status.
pub fn status() -> Result<(), String> {
    let config = vault::load_config();
    vault::print_status(&config)
}

/// Add a credential to the vault.
pub fn credential_add(name: &str) -> Result<(), String> {
    let config = vault::load_config();
    if !config.enabled {
        return Err("Vault is not enabled. Run `harness vault init` first.".to_string());
    }

    // Read value from stdin (don't echo)
    println!("Enter credential value for '{name}' (will not be echoed):");
    let value = read_password()?;

    if value.is_empty() {
        return Err("Empty credential value".to_string());
    }

    vault::store_credential(&config, name, &value, Some(&format!("Stored via harness for {name}")))?;
    println!("Credential '{name}' stored in vault.");

    Ok(())
}

/// List credentials in the vault.
pub fn credential_list() -> Result<(), String> {
    let config = vault::load_config();
    if !config.enabled {
        return Err("Vault is not enabled. Run `harness vault init` first.".to_string());
    }

    let creds = vault::list_credentials(&config)?;
    if creds.is_empty() {
        println!("No credentials stored.");
        return Ok(());
    }

    println!("Credentials ({}):\n", creds.len());
    for (path, desc) in &creds {
        let d = desc.as_deref().unwrap_or("");
        if d.is_empty() {
            println!("  {path}");
        } else {
            println!("  {path} — {d}");
        }
    }

    Ok(())
}

fn read_password() -> Result<String, String> {
    // Try to disable echo for password input
    let mut value = String::new();
    std::io::stdin().read_line(&mut value)
        .map_err(|e| format!("Failed to read input: {e}"))?;
    Ok(value.trim().to_string())
}

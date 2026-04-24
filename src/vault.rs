//! SanctumAI vault integration.
//!
//! Provides sync wrappers around the async sanctum-ai SDK using a minimal
//! tokio runtime. Credentials are retrieved via the "use, don't retrieve"
//! pattern where possible — the vault handles HTTP requests and signing
//! operations without exposing raw secrets.

use std::fs;
use std::path::PathBuf;

use ed25519_dalek::SigningKey;
use sanctum_ai::SanctumClient;
use serde_json::Value;

use crate::xdg;

const DEFAULT_VAULT_ADDR: &str = "127.0.0.1:7600";
const DEFAULT_AGENT_NAME: &str = "harness";
const CREDENTIAL_TTL: u64 = 300; // 5 minutes

/// Vault configuration stored per-workspace or globally.
#[derive(Debug, Clone)]
pub struct VaultConfig {
    pub enabled: bool,
    pub addr: String,
    pub agent_name: String,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            addr: DEFAULT_VAULT_ADDR.to_string(),
            agent_name: DEFAULT_AGENT_NAME.to_string(),
        }
    }
}

/// Load vault config from global config.toml.
pub fn load_config() -> VaultConfig {
    let path = xdg::config_dir().join("config.toml");
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return VaultConfig::default(),
    };
    let parsed: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(_) => return VaultConfig::default(),
    };

    let vault = match parsed.get("vault") {
        Some(v) => v,
        None => return VaultConfig::default(),
    };

    VaultConfig {
        enabled: vault
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        addr: vault
            .get("addr")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_VAULT_ADDR)
            .to_string(),
        agent_name: vault
            .get("agent_name")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_AGENT_NAME)
            .to_string(),
    }
}

/// Get or create the Ed25519 signing key for this harness agent.
fn load_or_create_signing_key() -> Result<SigningKey, String> {
    let key_path = key_file_path();
    if key_path.exists() {
        let bytes = fs::read(&key_path).map_err(|e| format!("Failed to read signing key: {e}"))?;
        if bytes.len() != 32 {
            return Err("Invalid signing key file (expected 32 bytes)".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(SigningKey::from_bytes(&arr))
    } else {
        // Generate a new key
        let mut rng = rand::thread_rng();
        let key = SigningKey::generate(&mut rng);
        if let Some(parent) = key_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create key dir: {e}"))?;
        }
        fs::write(&key_path, key.to_bytes())
            .map_err(|e| format!("Failed to write signing key: {e}"))?;
        // Set permissions to owner-only on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
        }
        Ok(key)
    }
}

fn key_file_path() -> PathBuf {
    xdg::data_dir().join("vault-key.bin")
}

/// Get the public key hex string (for registering with the vault).
pub fn public_key_hex() -> Result<String, String> {
    let key = load_or_create_signing_key()?;
    Ok(hex::encode(key.verifying_key().to_bytes()))
}

/// Run an async vault operation synchronously using a minimal tokio runtime.
fn block_on<F: std::future::Future<Output = T>, T>(f: F) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .expect("Failed to create tokio runtime")
        .block_on(f)
}

/// Check if the vault is reachable and healthy.
pub fn is_healthy(config: &VaultConfig) -> bool {
    if !config.enabled {
        return false;
    }
    block_on(async { SanctumClient::connect(&config.addr).await.is_ok() })
}

/// Connect and authenticate to the vault. Returns a connected client.
fn connect_and_auth(config: &VaultConfig) -> Result<SanctumClient, String> {
    let key = load_or_create_signing_key()?;
    block_on(async {
        let client = SanctumClient::connect(&config.addr)
            .await
            .map_err(|e| format!("Vault connection failed: {e}"))?;
        client
            .authenticate(&config.agent_name, &key)
            .await
            .map_err(|e| format!("Vault authentication failed: {e}"))?;
        Ok(client)
    })
}

/// Retrieve a credential value by path. Returns the JSON value.
pub fn get_credential(config: &VaultConfig, path: &str) -> Result<Value, String> {
    let client = connect_and_auth(config)?;
    block_on(async {
        let cred = client
            .retrieve(path, CREDENTIAL_TTL)
            .await
            .map_err(|e| format!("Failed to retrieve credential '{path}': {e}"))?;
        Ok(cred.value)
    })
}

/// Retrieve a credential as a string.
pub fn get_credential_string(config: &VaultConfig, path: &str) -> Result<String, String> {
    let value = get_credential(config, path)?;
    match value {
        Value::String(s) => Ok(s),
        other => Ok(other.to_string()),
    }
}

/// Use the "use, don't retrieve" pattern — vault executes the operation
/// without exposing the raw credential.
pub fn use_credential(
    config: &VaultConfig,
    path: &str,
    operation: &str,
    params: Value,
) -> Result<Value, String> {
    let client = connect_and_auth(config)?;
    block_on(async {
        let result = client
            .use_credential(path, operation, params)
            .await
            .map_err(|e| format!("Vault use_credential failed: {e}"))?;
        if !result.success {
            return Err("Vault operation returned failure".to_string());
        }
        Ok(result.output.unwrap_or(Value::Null))
    })
}

/// List all credentials available to this agent.
pub fn list_credentials(config: &VaultConfig) -> Result<Vec<(String, Option<String>)>, String> {
    let client = connect_and_auth(config)?;
    block_on(async {
        let creds = client
            .list()
            .await
            .map_err(|e| format!("Failed to list credentials: {e}"))?;
        Ok(creds.into_iter().map(|c| (c.path, c.description)).collect())
    })
}

/// Store a credential (sends to vault via a "store" RPC — if supported by the vault).
/// This is a convenience wrapper; the actual storage is handled server-side.
pub fn store_credential(
    config: &VaultConfig,
    path: &str,
    value: &str,
    description: Option<&str>,
) -> Result<(), String> {
    let client = connect_and_auth(config)?;
    block_on(async {
        let params = serde_json::json!({
            "path": path,
            "value": value,
            "description": description,
        });
        // Use the credential.store RPC if available
        client
            .use_credential("_system", "store", params)
            .await
            .map_err(|e| format!("Failed to store credential: {e}"))?;
        Ok(())
    })
}

/// Print vault status info.
pub fn print_status(config: &VaultConfig) -> Result<(), String> {
    if !config.enabled {
        println!("Vault: disabled");
        println!();
        println!("Enable in ~/.config/harness/config.toml:");
        println!("  [vault]");
        println!("  enabled = true");
        println!("  addr = \"127.0.0.1:7600\"");
        return Ok(());
    }

    println!("Vault: enabled");
    println!("  Address: {}", config.addr);
    println!("  Agent:   {}", config.agent_name);

    let pubkey = public_key_hex()?;
    println!("  PubKey:  {}", &pubkey[..16]);

    if is_healthy(config) {
        println!("  Status:  connected");

        // Try to list credentials
        match list_credentials(config) {
            Ok(creds) => {
                println!("  Credentials: {}", creds.len());
                for (path, desc) in &creds {
                    let d = desc.as_deref().unwrap_or("");
                    println!("    {path} {d}");
                }
            }
            Err(e) => println!("  (list failed: {e})"),
        }
    } else {
        println!("  Status:  unreachable");
    }

    Ok(())
}

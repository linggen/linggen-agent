use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

// ---------------------------------------------------------------------------
// Persisted format: ~/.linggen/credentials.json
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct CredentialEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Credentials {
    /// Keyed by model ID (e.g. "gemini-flash", "groq-llama").
    #[serde(flatten)]
    pub entries: HashMap<String, CredentialEntry>,
}

impl Credentials {
    /// Load from `~/.linggen/credentials.json`. Returns empty if missing or invalid.
    pub fn load(file: &Path) -> Self {
        if !file.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(file) {
            Ok(content) => match serde_json::from_str::<Credentials>(&content) {
                Ok(creds) => creds,
                Err(e) => {
                    warn!("Failed to parse credentials.json: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                warn!("Failed to read credentials.json: {}", e);
                Self::default()
            }
        }
    }

    /// Save to disk. Creates parent directories if needed.
    pub fn save(&self, file: &Path) -> anyhow::Result<()> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(file, json)?;
        Ok(())
    }

    /// Get the API key for a model ID.
    pub fn get_api_key(&self, model_id: &str) -> Option<&str> {
        self.entries
            .get(model_id)
            .and_then(|e| e.api_key.as_deref())
    }

    /// Set the API key for a model ID.
    pub fn set_api_key(&mut self, model_id: &str, api_key: Option<String>) {
        if let Some(key) = api_key {
            self.entries
                .entry(model_id.to_string())
                .or_default()
                .api_key = Some(key);
        } else {
            // Remove the entry if key is None.
            self.entries.remove(model_id);
        }
    }

    /// Return a copy with all keys redacted (for API responses).
    pub fn redacted(&self) -> Self {
        let entries = self
            .entries
            .iter()
            .map(|(id, entry)| {
                let redacted = CredentialEntry {
                    api_key: entry.api_key.as_ref().map(|_| "***".to_string()),
                };
                (id.clone(), redacted)
            })
            .collect();
        Self { entries }
    }
}

/// Default credentials file path: `~/.linggen/credentials.json`.
pub fn credentials_file() -> PathBuf {
    crate::paths::linggen_home().join("credentials.json")
}

/// Resolve the effective API key for a model.
/// Priority: 1) TOML config api_key  2) credentials.json  3) env var LINGGEN_API_KEY_{ID}
pub fn resolve_api_key(
    model_id: &str,
    config_api_key: Option<&str>,
    credentials: &Credentials,
) -> Option<String> {
    // 1. TOML config (backward compatible)
    if let Some(key) = config_api_key {
        if !key.is_empty() {
            return Some(key.to_string());
        }
    }
    // 2. credentials.json
    if let Some(key) = credentials.get_api_key(model_id) {
        if !key.is_empty() {
            return Some(key.to_string());
        }
    }
    // 3. Environment variable: LINGGEN_API_KEY_GEMINI_FLASH (hyphens → underscores, uppercase)
    let env_name = format!(
        "LINGGEN_API_KEY_{}",
        model_id.to_uppercase().replace('-', "_")
    );
    if let Ok(key) = std::env::var(&env_name) {
        if !key.is_empty() {
            return Some(key);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_roundtrip() {
        let tmp = std::env::temp_dir().join("linggen_cred_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("credentials.json");

        let mut creds = Credentials::default();
        creds.set_api_key("gemini-flash", Some("AIza123".to_string()));
        creds.set_api_key("groq-llama", Some("gsk_456".to_string()));
        creds.save(&file).unwrap();

        let loaded = Credentials::load(&file);
        assert_eq!(loaded.get_api_key("gemini-flash"), Some("AIza123"));
        assert_eq!(loaded.get_api_key("groq-llama"), Some("gsk_456"));
        assert_eq!(loaded.get_api_key("unknown"), None);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_credentials_redacted() {
        let mut creds = Credentials::default();
        creds.set_api_key("model-a", Some("secret".to_string()));
        let redacted = creds.redacted();
        assert_eq!(redacted.get_api_key("model-a"), Some("***"));
    }

    #[test]
    fn test_resolve_api_key_priority() {
        let mut creds = Credentials::default();
        creds.set_api_key("m1", Some("from_creds".to_string()));

        // TOML takes priority
        assert_eq!(
            resolve_api_key("m1", Some("from_toml"), &creds),
            Some("from_toml".to_string())
        );
        // Falls back to credentials
        assert_eq!(
            resolve_api_key("m1", None, &creds),
            Some("from_creds".to_string())
        );
        // No key at all
        assert_eq!(resolve_api_key("m2", None, &creds), None);
    }

    #[test]
    fn test_set_api_key_none_removes() {
        let mut creds = Credentials::default();
        creds.set_api_key("m1", Some("key".to_string()));
        assert!(creds.get_api_key("m1").is_some());
        creds.set_api_key("m1", None);
        assert!(creds.get_api_key("m1").is_none());
    }

    #[test]
    fn test_load_missing_file() {
        let creds = Credentials::load(Path::new("/nonexistent/credentials.json"));
        assert!(creds.entries.is_empty());
    }
}

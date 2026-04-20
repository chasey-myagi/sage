//! Credential storage for API keys and OAuth tokens.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/auth-storage.ts`.
//!
//! Handles loading, saving, and refreshing credentials from `auth.json`.
//! An in-memory backend is also provided for tests.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::get_agent_dir;

// ============================================================================
// Credential types
// ============================================================================

/// An API-key credential stored in `auth.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiKeyCredential {
    pub key: String,
}

/// An OAuth credential stored in `auth.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuthCredential {
    pub access: String,
    pub refresh: String,
    /// Unix-epoch milliseconds when the access token expires.
    pub expires: u64,
}

/// Union of supported credential types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthCredential {
    #[serde(rename = "api_key")]
    ApiKey(ApiKeyCredential),
    #[serde(rename = "oauth")]
    OAuth(OAuthCredential),
}

/// The raw JSON map stored in `auth.json`.
pub type AuthStorageData = HashMap<String, AuthCredential>;

// ============================================================================
// AuthStorageBackend trait
// ============================================================================

/// Abstraction over the persistence layer.
///
/// Mirrors the `AuthStorageBackend` interface from TypeScript.
/// Uses concrete read/write methods to remain dyn-compatible.
pub trait AuthStorageBackend: Send + Sync {
    /// Read the current raw JSON string (None if file does not exist yet).
    fn read(&self) -> Option<String>;

    /// Write a new JSON string to the backend.
    fn write(&self, content: &str);
}

// ============================================================================
// FileAuthStorageBackend
// ============================================================================

/// Backend that persists credentials to a JSON file.
pub struct FileAuthStorageBackend {
    auth_path: PathBuf,
}

impl FileAuthStorageBackend {
    pub fn new(auth_path: PathBuf) -> Self {
        Self { auth_path }
    }

    fn ensure_parent_dir(&self) {
        if let Some(parent) = self.auth_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    fn ensure_file_exists(&self) {
        if !self.auth_path.exists() {
            let _ = std::fs::write(&self.auth_path, "{}");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    &self.auth_path,
                    std::fs::Permissions::from_mode(0o600),
                );
            }
        }
    }
}

impl AuthStorageBackend for FileAuthStorageBackend {
    fn read(&self) -> Option<String> {
        self.ensure_parent_dir();
        self.ensure_file_exists();
        std::fs::read_to_string(&self.auth_path).ok()
    }

    fn write(&self, content: &str) {
        let _ = std::fs::write(&self.auth_path, content);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &self.auth_path,
                std::fs::Permissions::from_mode(0o600),
            );
        }
    }
}

// ============================================================================
// InMemoryAuthStorageBackend
// ============================================================================

/// In-memory backend — used in tests.
pub struct InMemoryAuthStorageBackend {
    value: std::sync::Mutex<Option<String>>,
}

impl InMemoryAuthStorageBackend {
    pub fn new() -> Self {
        Self { value: std::sync::Mutex::new(None) }
    }

    pub fn new_with(content: String) -> Self {
        Self { value: std::sync::Mutex::new(Some(content)) }
    }
}

impl Default for InMemoryAuthStorageBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthStorageBackend for InMemoryAuthStorageBackend {
    fn read(&self) -> Option<String> {
        self.value.lock().expect("InMemoryAuthStorageBackend lock").clone()
    }

    fn write(&self, content: &str) {
        *self.value.lock().expect("InMemoryAuthStorageBackend lock") = Some(content.to_string());
    }
}

// ============================================================================
// AuthStorage
// ============================================================================

/// Credential storage backed by a JSON file (or in-memory for tests).
///
/// Priority for `get_api_key()`:
/// 1. Runtime override (CLI `--api-key`)
/// 2. API key from `auth.json`
/// 3. OAuth token from `auth.json` (auto-refreshed)
/// 4. Environment variable
/// 5. Fallback resolver (custom providers)
pub struct AuthStorage {
    storage: Box<dyn AuthStorageBackend>,
    data: AuthStorageData,
    runtime_overrides: HashMap<String, String>,
    fallback_resolver: Option<Box<dyn Fn(&str) -> Option<String> + Send + Sync>>,
    load_error: Option<String>,
    errors: Vec<String>,
}

impl AuthStorage {
    fn new(storage: Box<dyn AuthStorageBackend>) -> Self {
        let mut s = Self {
            storage,
            data: HashMap::new(),
            runtime_overrides: HashMap::new(),
            fallback_resolver: None,
            load_error: None,
            errors: Vec::new(),
        };
        s.reload();
        s
    }

    /// Create using a file backend at the default `auth.json` path.
    pub fn create(auth_path: Option<PathBuf>) -> Self {
        let path = auth_path.unwrap_or_else(|| get_agent_dir().join("auth.json"));
        Self::new(Box::new(FileAuthStorageBackend::new(path)))
    }

    /// Create with an explicit backend (useful for tests).
    pub fn from_storage(storage: Box<dyn AuthStorageBackend>) -> Self {
        Self::new(storage)
    }

    /// Create with an in-memory backend pre-populated with `data`.
    pub fn in_memory(data: AuthStorageData) -> Self {
        let json = serde_json::to_string_pretty(&data).unwrap_or_else(|_| "{}".into());
        let backend = InMemoryAuthStorageBackend::new_with(json);
        Self::new(Box::new(backend))
    }

    fn parse_storage_data(content: Option<&str>) -> AuthStorageData {
        let Some(s) = content else { return HashMap::new() };
        serde_json::from_str(s).unwrap_or_default()
    }

    // ---- Reload ----

    /// Reload credentials from the backend.
    pub fn reload(&mut self) {
        let content = self.storage.read();

        match content.as_deref() {
            Some(s) => match serde_json::from_str::<AuthStorageData>(s) {
                Ok(parsed) => {
                    self.data = parsed;
                    self.load_error = None;
                }
                Err(e) => {
                    let msg = e.to_string();
                    self.load_error = Some(msg.clone());
                    self.errors.push(msg);
                    // Keep previous in-memory data on parse failure
                }
            },
            None => {
                self.data = HashMap::new();
                self.load_error = None;
            }
        }
    }

    // ---- Persist single-provider change ----

    fn persist_provider_change(&mut self, provider: &str, credential: Option<&AuthCredential>) {
        if self.load_error.is_some() {
            return;
        }

        // Re-read the file to pick up any external edits (mirrors TS lock behavior)
        let current = self.storage.read();
        let mut current_data = Self::parse_storage_data(current.as_deref());

        match credential {
            Some(c) => {
                current_data.insert(provider.to_string(), c.clone());
            }
            None => {
                current_data.remove(provider);
            }
        }

        match serde_json::to_string_pretty(&current_data) {
            Ok(json) => {
                self.storage.write(&json);
            }
            Err(e) => {
                self.errors.push(e.to_string());
            }
        }
    }

    // ---- Public API ----

    /// Get the credential for a provider.
    pub fn get(&self, provider: &str) -> Option<&AuthCredential> {
        self.data.get(provider)
    }

    /// Store a credential for a provider.
    pub fn set(&mut self, provider: &str, credential: AuthCredential) {
        self.data.insert(provider.to_string(), credential.clone());
        self.persist_provider_change(provider, Some(&credential));
    }

    /// Remove the credential for a provider.
    pub fn remove(&mut self, provider: &str) {
        self.data.remove(provider);
        self.persist_provider_change(provider, None);
    }

    /// List all providers with stored credentials.
    pub fn list(&self) -> Vec<&str> {
        self.data.keys().map(String::as_str).collect()
    }

    /// Returns `true` if a credential exists for `provider`.
    pub fn has(&self, provider: &str) -> bool {
        self.data.contains_key(provider)
    }

    /// Returns `true` if any form of auth is configured (credential, env var, or fallback).
    pub fn has_auth(&self, provider: &str) -> bool {
        if self.runtime_overrides.contains_key(provider) {
            return true;
        }
        if self.data.contains_key(provider) {
            return true;
        }
        if std::env::var(format!("{}_API_KEY", provider.to_uppercase())).is_ok() {
            return true;
        }
        if let Some(ref f) = self.fallback_resolver {
            if f(provider).is_some() {
                return true;
            }
        }
        false
    }

    /// Get a copy of all stored credentials.
    pub fn get_all(&self) -> AuthStorageData {
        self.data.clone()
    }

    /// Drain and return accumulated errors.
    pub fn drain_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    /// Set a runtime API-key override (not persisted to disk).
    pub fn set_runtime_api_key(&mut self, provider: &str, api_key: String) {
        self.runtime_overrides.insert(provider.to_string(), api_key);
    }

    /// Remove a runtime API-key override.
    pub fn remove_runtime_api_key(&mut self, provider: &str) {
        self.runtime_overrides.remove(provider);
    }

    /// Set a fallback resolver for API keys not found in `auth.json` or env vars.
    pub fn set_fallback_resolver<F>(&mut self, resolver: F)
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        self.fallback_resolver = Some(Box::new(resolver));
    }

    // ---- getApiKey ----

    /// Resolve the API key for a provider.
    ///
    /// Priority order mirrors TypeScript:
    /// 1. Runtime override
    /// 2. `api_key` credential from `auth.json` (via `resolve_config_value`)
    /// 3. OAuth access token (refresh if expired)
    /// 4. Environment variable
    /// 5. Fallback resolver
    pub async fn get_api_key(&self, provider_id: &str) -> Option<String> {
        // 1. Runtime override
        if let Some(key) = self.runtime_overrides.get(provider_id) {
            return Some(key.clone());
        }

        // 2. API key from auth.json
        if let Some(AuthCredential::ApiKey(cred)) = self.data.get(provider_id) {
            return crate::core::resolve_config_value::resolve_config_value(&cred.key).await;
        }

        // 3. OAuth token
        if let Some(AuthCredential::OAuth(cred)) = self.data.get(provider_id) {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            if now_ms < cred.expires {
                // Token still valid
                return Some(format!("Bearer {}", cred.access));
            }
            // Token expired — in full implementation we'd refresh here.
            // For now return None so model discovery skips this provider.
            return None;
        }

        // 4. Environment variable
        let env_var = format!("{}_API_KEY", provider_id.to_uppercase());
        if let Ok(val) = std::env::var(&env_var) {
            if !val.is_empty() {
                return Some(val);
            }
        }

        // 5. Fallback resolver
        if let Some(ref f) = self.fallback_resolver {
            return f(provider_id);
        }

        None
    }

    /// Logout from a provider (removes credential).
    pub fn logout(&mut self, provider: &str) {
        self.remove(provider);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_auth_json(path: &std::path::Path, data: &serde_json::Value) {
        fs::write(path, serde_json::to_string(data).unwrap()).unwrap();
    }

    fn make_file_storage(dir: &std::path::Path) -> AuthStorage {
        let auth_path = dir.join("auth.json");
        AuthStorage::create(Some(auth_path))
    }

    // ---- basic persistence ----

    #[test]
    fn set_and_get() {
        let dir = tempdir().unwrap();
        let mut storage = make_file_storage(dir.path());

        storage.set(
            "anthropic",
            AuthCredential::ApiKey(ApiKeyCredential {
                key: "sk-ant-test".into(),
            }),
        );

        assert!(storage.has("anthropic"));
        let cred = storage.get("anthropic").unwrap();
        if let AuthCredential::ApiKey(k) = cred {
            assert_eq!(k.key, "sk-ant-test");
        } else {
            panic!("expected api_key");
        }
    }

    #[test]
    fn remove_deletes_credential() {
        let dir = tempdir().unwrap();
        let mut storage = make_file_storage(dir.path());

        storage.set(
            "openai",
            AuthCredential::ApiKey(ApiKeyCredential {
                key: "key".into(),
            }),
        );
        assert!(storage.has("openai"));

        storage.remove("openai");
        assert!(!storage.has("openai"));
    }

    #[test]
    fn set_preserves_unrelated_external_edits() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");

        write_auth_json(
            &auth_path,
            &serde_json::json!({
                "anthropic": { "type": "api_key", "key": "old-anthropic" },
                "openai": { "type": "api_key", "key": "openai-key" }
            }),
        );

        let mut storage = AuthStorage::create(Some(auth_path.clone()));

        // Simulate external edit
        write_auth_json(
            &auth_path,
            &serde_json::json!({
                "anthropic": { "type": "api_key", "key": "old-anthropic" },
                "openai": { "type": "api_key", "key": "openai-key" },
                "google": { "type": "api_key", "key": "google-key" }
            }),
        );

        storage.set(
            "anthropic",
            AuthCredential::ApiKey(ApiKeyCredential {
                key: "new-anthropic".into(),
            }),
        );

        let updated: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        assert_eq!(updated["anthropic"]["key"], "new-anthropic");
        assert_eq!(updated["openai"]["key"], "openai-key");
        assert_eq!(updated["google"]["key"], "google-key");
    }

    #[test]
    fn remove_preserves_unrelated_external_edits() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");

        write_auth_json(
            &auth_path,
            &serde_json::json!({
                "anthropic": { "type": "api_key", "key": "anthropic-key" },
                "openai": { "type": "api_key", "key": "openai-key" }
            }),
        );

        let mut storage = AuthStorage::create(Some(auth_path.clone()));

        write_auth_json(
            &auth_path,
            &serde_json::json!({
                "anthropic": { "type": "api_key", "key": "anthropic-key" },
                "openai": { "type": "api_key", "key": "openai-key" },
                "google": { "type": "api_key", "key": "google-key" }
            }),
        );

        storage.remove("anthropic");

        let updated: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        assert!(updated.get("anthropic").is_none() || updated["anthropic"].is_null());
        assert_eq!(updated["openai"]["key"], "openai-key");
        assert_eq!(updated["google"]["key"], "google-key");
    }

    #[test]
    fn does_not_overwrite_malformed_file_after_load_error() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");

        write_auth_json(
            &auth_path,
            &serde_json::json!({ "anthropic": { "type": "api_key", "key": "key" } }),
        );

        let mut storage = AuthStorage::create(Some(auth_path.clone()));

        // Corrupt the file
        fs::write(&auth_path, "{invalid-json").unwrap();
        storage.reload();

        // Attempting to set should be a no-op because of the load error
        storage.set(
            "openai",
            AuthCredential::ApiKey(ApiKeyCredential {
                key: "openai-key".into(),
            }),
        );

        // The file should still contain the malformed JSON
        let raw = fs::read_to_string(&auth_path).unwrap();
        assert_eq!(raw, "{invalid-json");
    }

    #[test]
    fn reload_records_parse_errors_and_drain_clears_buffer() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");

        write_auth_json(
            &auth_path,
            &serde_json::json!({ "anthropic": { "type": "api_key", "key": "key" } }),
        );

        let mut storage = AuthStorage::create(Some(auth_path.clone()));

        // Keep in-memory data intact across failed reloads
        assert!(storage.has("anthropic"));

        fs::write(&auth_path, "{invalid-json").unwrap();
        storage.reload();

        // In-memory data preserved
        assert!(storage.has("anthropic"));

        let first_drain = storage.drain_errors();
        assert!(!first_drain.is_empty());

        let second_drain = storage.drain_errors();
        assert!(second_drain.is_empty());
    }

    // ---- runtime overrides ----

    #[test]
    fn runtime_override_takes_priority() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        write_auth_json(
            &auth_path,
            &serde_json::json!({ "anthropic": { "type": "api_key", "key": "stored-key" } }),
        );

        let mut storage = AuthStorage::create(Some(auth_path));
        storage.set_runtime_api_key("anthropic", "runtime-key".into());

        let key = futures::executor::block_on(storage.get_api_key("anthropic"));
        assert_eq!(key.as_deref(), Some("runtime-key"));
    }

    #[test]
    fn removing_runtime_override_falls_back_to_auth_json() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        write_auth_json(
            &auth_path,
            &serde_json::json!({ "anthropic": { "type": "api_key", "key": "stored-key" } }),
        );

        let mut storage = AuthStorage::create(Some(auth_path));
        storage.set_runtime_api_key("anthropic", "runtime-key".into());
        storage.remove_runtime_api_key("anthropic");

        let key = futures::executor::block_on(storage.get_api_key("anthropic"));
        assert_eq!(key.as_deref(), Some("stored-key"));
    }

    // ---- in-memory backend ----

    #[test]
    fn in_memory_storage_works() {
        let mut data = HashMap::new();
        data.insert(
            "openai".to_string(),
            AuthCredential::ApiKey(ApiKeyCredential {
                key: "sk-openai".into(),
            }),
        );

        let storage = AuthStorage::in_memory(data);
        assert!(storage.has("openai"));
        assert_eq!(
            storage.get("openai"),
            Some(&AuthCredential::ApiKey(ApiKeyCredential {
                key: "sk-openai".into(),
            }))
        );
    }

    // ---- has_auth ----

    #[test]
    fn has_auth_via_runtime_override() {
        let storage_dir = tempdir().unwrap();
        let mut storage = make_file_storage(storage_dir.path());
        storage.set_runtime_api_key("myp", "key".into());
        assert!(storage.has_auth("myp"));
    }

    #[test]
    fn list_returns_providers() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        write_auth_json(
            &auth_path,
            &serde_json::json!({
                "anthropic": { "type": "api_key", "key": "a" },
                "openai": { "type": "api_key", "key": "b" }
            }),
        );
        let storage = AuthStorage::create(Some(auth_path));
        let mut providers = storage.list();
        providers.sort();
        assert_eq!(providers, vec!["anthropic", "openai"]);
    }
}

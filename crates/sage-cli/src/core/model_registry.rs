//! Model registry — manages built-in and custom models, API key resolution.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/model-registry.ts`.
//!
//! In the Rust port this is a simplified registry that loads a `models.json`
//! file from the agent dir and exposes lookup / resolution helpers. Full
//! provider registration mirrors the pi-mono structure but delegates to
//! the `ai` crate's registry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ============================================================================
// Types
// ============================================================================

/// A model entry from `models.json` or the built-in list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Provider name (e.g., `"anthropic"`, `"openai"`).
    pub provider: String,
    /// Model ID as used in the API (e.g., `"claude-sonnet-4-5"`).
    pub id: String,
    /// Optional human-readable display name.
    pub name: Option<String>,
    /// Optional context window size in tokens.
    pub context_window: Option<u64>,
    /// Whether the model supports reasoning/thinking.
    pub reasoning: Option<bool>,
}

impl ModelEntry {
    /// Check whether this model's ID or name matches `pattern` (case-insensitive substring).
    pub fn matches_pattern(&self, pattern: &str) -> bool {
        let pat = pattern.to_lowercase();
        if self.id.to_lowercase().contains(&pat) {
            return true;
        }
        if let Some(name) = &self.name
            && name.to_lowercase().contains(&pat)
        {
            return true;
        }
        false
    }
}

/// A registered API provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// API base URL.
    pub base_url: Option<String>,
    /// Environment variable holding the API key.
    pub api_key_env: Option<String>,
    /// Hardcoded API key (not recommended for production).
    pub api_key: Option<String>,
}

// ============================================================================
// ModelRegistry
// ============================================================================

/// Manages available models and custom provider registrations.
pub struct ModelRegistry {
    models: Vec<ModelEntry>,
    providers: HashMap<String, ProviderConfig>,
    models_path: PathBuf,
}

impl ModelRegistry {
    /// Create a new registry, loading models from `models_path` if it exists.
    pub fn new(models_path: impl Into<PathBuf>) -> Self {
        let models_path = models_path.into();
        let models = Self::load_models_file(&models_path).unwrap_or_default();
        Self {
            models,
            providers: HashMap::new(),
            models_path,
        }
    }

    /// Load models from a JSON file.
    fn load_models_file(path: &Path) -> Option<Vec<ModelEntry>> {
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Reload models from disk.
    pub fn reload(&mut self) -> anyhow::Result<()> {
        self.models = Self::load_models_file(&self.models_path).unwrap_or_default();
        Ok(())
    }

    /// Register a custom provider configuration.
    pub fn register_provider(&mut self, name: impl Into<String>, config: ProviderConfig) {
        self.providers.insert(name.into(), config);
    }

    /// Get provider configuration by name.
    pub fn get_provider(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.get(name)
    }

    /// Find models by provider and optional fuzzy-match pattern.
    pub fn find(&self, provider: &str, pattern: &str) -> Option<&ModelEntry> {
        self.models
            .iter()
            .find(|m| m.provider == provider && m.matches_pattern(pattern))
    }

    /// Find all models matching an optional fuzzy pattern across all providers.
    pub fn find_all(&self, pattern: Option<&str>) -> Vec<&ModelEntry> {
        match pattern {
            Some(p) => self
                .models
                .iter()
                .filter(|m| m.matches_pattern(p))
                .collect(),
            None => self.models.iter().collect(),
        }
    }

    /// Get all models for a specific provider.
    pub fn models_for_provider(&self, provider: &str) -> Vec<&ModelEntry> {
        self.models
            .iter()
            .filter(|m| m.provider == provider)
            .collect()
    }

    /// All loaded models.
    pub fn models(&self) -> &[ModelEntry] {
        &self.models
    }

    /// Resolve a CLI model string (possibly `"provider/id"` or just `"id"`) to a ModelEntry.
    ///
    /// Returns `(entry, warning)` — warning is Some if the model was not found exactly
    /// but a fuzzy match was used.
    pub fn resolve_cli_model(&self, cli_provider: Option<&str>, cli_model: &str) -> ResolvedModel {
        // Handle "provider/model" form
        let (provider, model_pat) = if let Some(slash) = cli_model.find('/') {
            let p = &cli_model[..slash];
            let m = &cli_model[slash + 1..];
            (Some(p.to_string()), m.to_string())
        } else {
            (cli_provider.map(String::from), cli_model.to_string())
        };

        // Strip ":thinking" suffix if present
        let (model_pat_clean, _thinking_suffix) = if let Some(colon) = model_pat.rfind(':') {
            let suffix = &model_pat[colon + 1..];
            if ["off", "minimal", "low", "medium", "high", "xhigh"].contains(&suffix) {
                (&model_pat[..colon], Some(suffix.to_string()))
            } else {
                (model_pat.as_str(), None)
            }
        } else {
            (model_pat.as_str(), None)
        };

        // Exact ID match first
        let exact = self.models.iter().find(|m| {
            let provider_match = provider.as_deref().is_none_or(|p| m.provider == p);
            provider_match && m.id == model_pat_clean
        });
        if let Some(entry) = exact {
            return ResolvedModel {
                entry: Some(entry.clone()),
                warning: None,
                error: None,
            };
        }

        // Fuzzy match
        let fuzzy: Vec<&ModelEntry> = self
            .models
            .iter()
            .filter(|m| {
                let provider_match = provider.as_deref().is_none_or(|p| m.provider == p);
                provider_match && m.matches_pattern(model_pat_clean)
            })
            .collect();

        match fuzzy.len() {
            0 => ResolvedModel {
                entry: None,
                warning: None,
                error: Some(format!("No model found matching '{cli_model}'")),
            },
            1 => ResolvedModel {
                entry: Some(fuzzy[0].clone()),
                warning: Some(format!(
                    "Model '{}' matched '{}' by fuzzy search",
                    fuzzy[0].id, cli_model
                )),
                error: None,
            },
            n => ResolvedModel {
                entry: Some(fuzzy[0].clone()),
                warning: Some(format!(
                    "{n} models matched '{cli_model}', using '{}'",
                    fuzzy[0].id
                )),
                error: None,
            },
        }
    }
}

/// Result of resolving a CLI model argument.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub entry: Option<ModelEntry>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_registry_with_models(models: Vec<ModelEntry>) -> (ModelRegistry, TempDir) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("models.json");
        let json = serde_json::to_string(&models).unwrap();
        std::fs::write(&path, json).unwrap();
        let reg = ModelRegistry::new(&path);
        (reg, tmp)
    }

    fn sample_models() -> Vec<ModelEntry> {
        vec![
            ModelEntry {
                provider: "anthropic".to_string(),
                id: "claude-sonnet-4-5".to_string(),
                name: Some("Claude Sonnet".to_string()),
                context_window: Some(200000),
                reasoning: Some(false),
            },
            ModelEntry {
                provider: "openai".to_string(),
                id: "gpt-4o".to_string(),
                name: Some("GPT-4o".to_string()),
                context_window: Some(128000),
                reasoning: Some(false),
            },
        ]
    }

    #[test]
    fn registry_no_file() {
        let reg = ModelRegistry::new("/nonexistent/path/models.json");
        assert!(reg.models().is_empty());
    }

    #[test]
    fn registry_loads_from_file() {
        let (reg, _tmp) = make_registry_with_models(sample_models());
        assert_eq!(reg.models().len(), 2);
    }

    #[test]
    fn find_exact_provider_and_id() {
        let (reg, _tmp) = make_registry_with_models(sample_models());
        let m = reg.find("anthropic", "claude-sonnet-4-5");
        assert!(m.is_some());
        assert_eq!(m.unwrap().id, "claude-sonnet-4-5");
    }

    #[test]
    fn find_all_no_filter() {
        let (reg, _tmp) = make_registry_with_models(sample_models());
        assert_eq!(reg.find_all(None).len(), 2);
    }

    #[test]
    fn find_all_with_pattern() {
        let (reg, _tmp) = make_registry_with_models(sample_models());
        let result = reg.find_all(Some("sonnet"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "claude-sonnet-4-5");
    }

    #[test]
    fn models_for_provider() {
        let (reg, _tmp) = make_registry_with_models(sample_models());
        let anthropic = reg.models_for_provider("anthropic");
        assert_eq!(anthropic.len(), 1);
    }

    #[test]
    fn resolve_cli_model_exact() {
        let (reg, _tmp) = make_registry_with_models(sample_models());
        let r = reg.resolve_cli_model(Some("anthropic"), "claude-sonnet-4-5");
        assert!(r.entry.is_some());
        assert!(r.error.is_none());
    }

    #[test]
    fn resolve_cli_model_provider_slash_id() {
        let (reg, _tmp) = make_registry_with_models(sample_models());
        let r = reg.resolve_cli_model(None, "openai/gpt-4o");
        assert!(r.entry.is_some());
        assert_eq!(r.entry.unwrap().id, "gpt-4o");
    }

    #[test]
    fn resolve_cli_model_not_found() {
        let (reg, _tmp) = make_registry_with_models(sample_models());
        let r = reg.resolve_cli_model(None, "no-such-model");
        assert!(r.entry.is_none());
        assert!(r.error.is_some());
    }

    #[test]
    fn resolve_cli_model_fuzzy_match() {
        let (reg, _tmp) = make_registry_with_models(sample_models());
        let r = reg.resolve_cli_model(None, "sonnet");
        assert!(r.entry.is_some());
        assert!(r.warning.is_some());
    }

    #[test]
    fn register_and_get_provider() {
        let mut reg = ModelRegistry::new("/nonexistent");
        reg.register_provider(
            "custom",
            ProviderConfig {
                base_url: Some("https://api.custom.example".to_string()),
                api_key_env: Some("CUSTOM_API_KEY".to_string()),
                api_key: None,
            },
        );
        let p = reg.get_provider("custom");
        assert!(p.is_some());
        assert_eq!(
            p.unwrap().base_url.as_deref(),
            Some("https://api.custom.example")
        );
    }

    #[test]
    fn model_matches_pattern_case_insensitive() {
        let m = ModelEntry {
            provider: "anthropic".to_string(),
            id: "claude-sonnet-4-5".to_string(),
            name: Some("Claude Sonnet".to_string()),
            context_window: None,
            reasoning: None,
        };
        assert!(m.matches_pattern("Sonnet"));
        assert!(m.matches_pattern("CLAUDE"));
        assert!(!m.matches_pattern("gpt"));
    }
}

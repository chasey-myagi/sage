//! Keybinding definitions and manager for the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/keybindings.ts`.
//!
//! In the TypeScript implementation keybindings control the interactive TUI.
//! This Rust translation provides the keybinding definitions as static data
//! that can be serialised/deserialised from `keybindings.json`.  The actual
//! terminal-input handling is left to the TUI layer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::config::get_agent_dir;

// ============================================================================
// Types
// ============================================================================

/// A single key binding value — either a single key string or a list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeyBinding {
    Single(String),
    Multiple(Vec<String>),
}

impl KeyBinding {
    /// Return the binding as a list of key strings.
    pub fn as_keys(&self) -> Vec<&str> {
        match self {
            KeyBinding::Single(s) => vec![s.as_str()],
            KeyBinding::Multiple(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            KeyBinding::Single(s) => s.is_empty(),
            KeyBinding::Multiple(v) => v.is_empty(),
        }
    }
}

/// Raw user keybindings config (key-id → binding value).
pub type KeybindingsConfig = HashMap<String, KeyBinding>;

/// Metadata for a keybinding entry (default keys + description).
#[derive(Debug, Clone)]
pub struct KeybindingDefinition {
    pub default_keys: KeyBinding,
    pub description: &'static str,
}

// ============================================================================
// Known keybinding IDs
// ============================================================================

/// All application keybinding IDs, matching pi-mono's `KEYBINDINGS` constant.
pub const ALL_KEYBINDING_IDS: &[&str] = &[
    "app.interrupt",
    "app.clear",
    "app.exit",
    "app.suspend",
    "app.thinking.cycle",
    "app.model.cycleForward",
    "app.model.cycleBackward",
    "app.model.select",
    "app.tools.expand",
    "app.thinking.toggle",
    "app.session.toggleNamedFilter",
    "app.editor.external",
    "app.message.followUp",
    "app.message.dequeue",
    "app.clipboard.pasteImage",
    "app.session.new",
    "app.session.tree",
    "app.session.fork",
    "app.session.resume",
    "app.tree.foldOrUp",
    "app.tree.unfoldOrDown",
    "app.session.togglePath",
    "app.session.toggleSort",
    "app.session.rename",
    "app.session.delete",
    "app.session.deleteNoninvasive",
];

// ============================================================================
// Default app keybinding keys
// ============================================================================

static DEFAULT_APP_KEYS: OnceLock<HashMap<&'static str, &'static [&'static str]>> = OnceLock::new();

fn default_app_keys() -> &'static HashMap<&'static str, &'static [&'static str]> {
    DEFAULT_APP_KEYS.get_or_init(|| {
        let entries: &[(&str, &[&str])] = &[
            ("app.interrupt", &["ctrl+c", "escape"]),
            ("app.clear", &["ctrl+l"]),
            ("app.exit", &["ctrl+d"]),
            ("app.thinking.toggle", &["alt+t"]),
            ("app.model.select", &["alt+p"]),
            ("app.clipboard.pasteImage", &["ctrl+v"]),
            // ctrl+x ctrl+e is a chord — handled by the resolver, not here
            ("app.editor.external", &["ctrl+g"]),
            ("app.session.tree", &["ctrl+o"]),
            ("app.thinking.cycle", &["shift+tab"]),
            ("app.tree.foldOrUp", &["alt+left", "ctrl+left"]),
            ("app.tree.unfoldOrDown", &["alt+right", "ctrl+right"]),
        ];
        entries.iter().copied().collect()
    })
}

// Global app keybindings manager (set at startup to include user overrides).
static GLOBAL_APP_KEYBINDINGS: OnceLock<Mutex<Option<KeybindingsManager>>> = OnceLock::new();

fn global_app_keybindings_cell() -> &'static Mutex<Option<KeybindingsManager>> {
    GLOBAL_APP_KEYBINDINGS.get_or_init(|| Mutex::new(None))
}

/// Install a `KeybindingsManager` as the global app keybinding authority.
///
/// Call this at startup after loading `keybindings.json` so that user
/// overrides are respected by `check_app_keybinding`.
pub fn set_app_keybindings(manager: KeybindingsManager) {
    *global_app_keybindings_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(manager);
}

/// Check whether `data` matches any key bound to `action`.
///
/// Consults the global app keybindings manager first (user overrides), then
/// falls back to the built-in defaults.  Chord bindings (space-separated key
/// IDs) are skipped — they require stateful chord tracking via the resolver.
pub fn check_app_keybinding(data: &str, action: &str) -> bool {
    // Prefer user config when installed.
    if let Some(mgr) = global_app_keybindings_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        if let Some(binding) = mgr.get_binding(action) {
            return binding
                .as_keys()
                .iter()
                .any(|k| !k.contains(' ') && tui::keys::matches_key(data, k));
        }
    }
    // Fall back to built-in defaults.
    if let Some(keys) = default_app_keys().get(action) {
        return keys.iter().any(|&k| tui::keys::matches_key(data, k));
    }
    false
}

// ============================================================================
// Validation
// ============================================================================

/// Category of a keybinding configuration warning.
#[derive(Debug, Clone, PartialEq)]
pub enum KeybindingWarningType {
    ParseError,
    Duplicate,
    InvalidAction,
}

/// Severity of a keybinding configuration warning.
#[derive(Debug, Clone, PartialEq)]
pub enum WarningSeverity {
    Error,
    Warning,
}

/// A warning about a keybinding configuration issue.
#[derive(Debug, Clone)]
pub struct KeybindingWarning {
    pub warning_type: KeybindingWarningType,
    pub severity: WarningSeverity,
    pub message: String,
    pub key: Option<String>,
}

/// Validate user keybinding config structure and return any warnings.
///
/// Mirrors the structural checks from CC `keybindings/validate.ts::validateUserConfig`.
pub fn validate_user_config(config: &KeybindingsConfig) -> Vec<KeybindingWarning> {
    let mut warnings = Vec::new();
    for (action, binding) in config {
        if !ALL_KEYBINDING_IDS.contains(&action.as_str()) {
            warnings.push(KeybindingWarning {
                warning_type: KeybindingWarningType::InvalidAction,
                severity: WarningSeverity::Warning,
                message: format!("Unknown keybinding action \"{action}\""),
                key: None,
            });
        }
        for key in binding.as_keys() {
            if key.is_empty() {
                warnings.push(KeybindingWarning {
                    warning_type: KeybindingWarningType::ParseError,
                    severity: WarningSeverity::Error,
                    message: format!("Empty key string for action \"{action}\""),
                    key: Some(String::new()),
                });
            }
        }
    }
    warnings
}

/// Check for duplicate key assignments across different actions.
///
/// Mirrors CC `keybindings/validate.ts::checkDuplicates`.
pub fn check_duplicates(config: &KeybindingsConfig) -> Vec<KeybindingWarning> {
    let mut key_to_actions: HashMap<String, Vec<String>> = HashMap::new();
    for (action, binding) in config {
        for key in binding.as_keys() {
            key_to_actions
                .entry(key.to_lowercase())
                .or_default()
                .push(action.clone());
        }
    }
    let mut warnings = Vec::new();
    for (key, actions) in &key_to_actions {
        if actions.len() > 1 {
            warnings.push(KeybindingWarning {
                warning_type: KeybindingWarningType::Duplicate,
                severity: WarningSeverity::Warning,
                message: format!(
                    "Key \"{key}\" is bound to multiple actions: {}",
                    actions.join(", ")
                ),
                key: Some(key.clone()),
            });
        }
    }
    warnings
}

/// Legacy keybinding name → new name migration map.
///
/// Mirrors `KEYBINDING_NAME_MIGRATIONS` from pi-mono.
pub fn keybinding_name_migrations() -> HashMap<&'static str, &'static str> {
    [
        ("interrupt", "app.interrupt"),
        ("clear", "app.clear"),
        ("exit", "app.exit"),
        ("suspend", "app.suspend"),
        ("cycleThinkingLevel", "app.thinking.cycle"),
        ("cycleModelForward", "app.model.cycleForward"),
        ("cycleModelBackward", "app.model.cycleBackward"),
        ("selectModel", "app.model.select"),
        ("expandTools", "app.tools.expand"),
        ("toggleThinking", "app.thinking.toggle"),
        ("toggleSessionNamedFilter", "app.session.toggleNamedFilter"),
        ("externalEditor", "app.editor.external"),
        ("followUp", "app.message.followUp"),
        ("dequeue", "app.message.dequeue"),
        ("pasteImage", "app.clipboard.pasteImage"),
        ("newSession", "app.session.new"),
        ("tree", "app.session.tree"),
        ("fork", "app.session.fork"),
        ("resume", "app.session.resume"),
        ("treeFoldOrUp", "app.tree.foldOrUp"),
        ("treeUnfoldOrDown", "app.tree.unfoldOrDown"),
        ("toggleSessionPath", "app.session.togglePath"),
        ("toggleSessionSort", "app.session.toggleSort"),
        ("renameSession", "app.session.rename"),
        ("deleteSession", "app.session.delete"),
        ("deleteSessionNoninvasive", "app.session.deleteNoninvasive"),
    ]
    .into_iter()
    .collect()
}

// ============================================================================
// Migration helpers
// ============================================================================

/// Migrate legacy keybinding names in `config` to their canonical equivalents.
///
/// Returns `(migrated_config, was_migrated)`.
///
/// Mirrors pi-mono `migrateKeybindingNames`.
pub fn migrate_keybinding_names(config: KeybindingsConfig) -> (KeybindingsConfig, bool) {
    let migrations = keybinding_name_migrations();
    let mut result: KeybindingsConfig = HashMap::new();
    let mut migrated = false;

    for (key, value) in &config {
        let new_key = migrations
            .get(key.as_str())
            .copied()
            .unwrap_or(key.as_str());
        if new_key != key {
            migrated = true;
        }
        // Skip if the target key already present (avoid overwrite on collision)
        if new_key != key.as_str() && config.contains_key(new_key) {
            migrated = true;
            continue;
        }
        result.insert(new_key.to_string(), value.clone());
    }

    (result, migrated)
}

// ============================================================================
// File I/O
// ============================================================================

/// Load keybindings from a JSON file.  Returns empty map if file absent or
/// invalid.
///
/// Mirrors pi-mono `KeybindingsManager.loadFromFile`.
pub fn load_keybindings_from_file(path: &Path) -> KeybindingsConfig {
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(raw): Result<serde_json::Value, _> = serde_json::from_str(&content) else {
        return HashMap::new();
    };
    let Some(obj) = raw.as_object() else {
        return HashMap::new();
    };

    let mut config = KeybindingsConfig::new();
    for (k, v) in obj {
        if let Some(s) = v.as_str() {
            config.insert(k.clone(), KeyBinding::Single(s.to_string()));
        } else if let Some(arr) = v.as_array() {
            let keys: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
            config.insert(k.clone(), KeyBinding::Multiple(keys));
        }
    }

    let (migrated, _) = migrate_keybinding_names(config);

    // Validate and log any config warnings.
    for w in validate_user_config(&migrated)
        .iter()
        .chain(check_duplicates(&migrated).iter())
    {
        match w.severity {
            WarningSeverity::Error => tracing::error!("keybinding config error: {}", w.message),
            WarningSeverity::Warning => tracing::warn!("keybinding config warning: {}", w.message),
        };
    }

    migrated
}

/// Write keybindings config to a JSON file.
pub fn save_keybindings_to_file(path: &Path, config: &KeybindingsConfig) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(config).map_err(std::io::Error::other)?;
    std::fs::write(path, format!("{json}\n"))
}

/// Migrate the keybindings config file in-place.
///
/// Returns `true` if the file existed and was migrated.
///
/// Mirrors pi-mono `migrateKeybindingsConfigFile`.
pub fn migrate_keybindings_config_file(agent_dir: Option<&Path>) -> bool {
    let dir = agent_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(get_agent_dir);
    let path = dir.join("keybindings.json");

    if !path.exists() {
        return false;
    }

    let raw = load_keybindings_from_file(&path);
    if raw.is_empty() {
        return false;
    }

    let (migrated_config, was_migrated) = migrate_keybinding_names(raw);
    if !was_migrated {
        return false;
    }

    let _ = save_keybindings_to_file(&path, &migrated_config);
    true
}

// ============================================================================
// KeybindingsManager
// ============================================================================

/// Manages keybindings configuration with file-based persistence.
///
/// Mirrors pi-mono `KeybindingsManager`.
pub struct KeybindingsManager {
    user_bindings: KeybindingsConfig,
    config_path: Option<PathBuf>,
}

impl KeybindingsManager {
    /// Create with explicit user bindings (no file backing).
    pub fn new(user_bindings: KeybindingsConfig) -> Self {
        Self {
            user_bindings,
            config_path: None,
        }
    }

    /// Create a `KeybindingsManager` backed by `<agent_dir>/keybindings.json`.
    ///
    /// Mirrors pi-mono `KeybindingsManager.create`.
    pub fn create(agent_dir: Option<&Path>) -> Self {
        let dir = agent_dir
            .map(|p| p.to_path_buf())
            .unwrap_or_else(get_agent_dir);
        let config_path = dir.join("keybindings.json");
        let user_bindings = load_keybindings_from_file(&config_path);
        Self {
            user_bindings,
            config_path: Some(config_path),
        }
    }

    /// Reload bindings from the config file (no-op if no file backing).
    pub fn reload(&mut self) {
        if let Some(ref path) = self.config_path {
            self.user_bindings = load_keybindings_from_file(path);
        }
    }

    /// Return the user-configured binding for `key_id`, if any.
    pub fn get_binding(&self, key_id: &str) -> Option<&KeyBinding> {
        self.user_bindings.get(key_id)
    }

    /// Return a cloned snapshot of the full effective config.
    pub fn get_effective_config(&self) -> KeybindingsConfig {
        self.user_bindings.clone()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp_json(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    #[test]
    fn load_single_binding() {
        let file = write_temp_json(r#"{"app.interrupt": "escape"}"#);
        let config = load_keybindings_from_file(file.path());
        assert_eq!(
            config.get("app.interrupt"),
            Some(&KeyBinding::Single("escape".to_string()))
        );
    }

    #[test]
    fn load_multiple_bindings() {
        let file = write_temp_json(r#"{"app.tree.foldOrUp": ["ctrl+left", "alt+left"]}"#);
        let config = load_keybindings_from_file(file.path());
        assert_eq!(
            config.get("app.tree.foldOrUp"),
            Some(&KeyBinding::Multiple(vec![
                "ctrl+left".to_string(),
                "alt+left".to_string()
            ]))
        );
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let config = load_keybindings_from_file(Path::new("/nonexistent/keybindings.json"));
        assert!(config.is_empty());
    }

    #[test]
    fn migrate_legacy_interrupt_name() {
        let mut raw = KeybindingsConfig::new();
        raw.insert(
            "interrupt".to_string(),
            KeyBinding::Single("escape".to_string()),
        );
        let (migrated, was_migrated) = migrate_keybinding_names(raw);
        assert!(was_migrated);
        assert!(migrated.contains_key("app.interrupt"));
        assert!(!migrated.contains_key("interrupt"));
    }

    #[test]
    fn migrate_no_change_for_canonical_name() {
        let mut raw = KeybindingsConfig::new();
        raw.insert(
            "app.interrupt".to_string(),
            KeyBinding::Single("escape".to_string()),
        );
        let (_migrated, was_migrated) = migrate_keybinding_names(raw);
        assert!(!was_migrated);
    }

    #[test]
    fn keybinding_as_keys_single() {
        let kb = KeyBinding::Single("ctrl+c".to_string());
        assert_eq!(kb.as_keys(), vec!["ctrl+c"]);
    }

    #[test]
    fn keybinding_as_keys_multiple() {
        let kb = KeyBinding::Multiple(vec!["ctrl+left".to_string(), "alt+left".to_string()]);
        assert_eq!(kb.as_keys(), vec!["ctrl+left", "alt+left"]);
    }

    #[test]
    fn keybinding_manager_get_binding() {
        let mut config = KeybindingsConfig::new();
        config.insert(
            "app.exit".to_string(),
            KeyBinding::Single("ctrl+d".to_string()),
        );
        let mgr = KeybindingsManager::new(config);
        assert_eq!(
            mgr.get_binding("app.exit"),
            Some(&KeyBinding::Single("ctrl+d".to_string()))
        );
        assert!(mgr.get_binding("app.interrupt").is_none());
    }

    #[test]
    fn keybinding_manager_effective_config() {
        let mut config = KeybindingsConfig::new();
        config.insert(
            "app.exit".to_string(),
            KeyBinding::Single("ctrl+d".to_string()),
        );
        let mgr = KeybindingsManager::new(config.clone());
        assert_eq!(mgr.get_effective_config(), config);
    }

    #[test]
    fn all_keybinding_ids_non_empty() {
        assert!(!ALL_KEYBINDING_IDS.is_empty());
        for id in ALL_KEYBINDING_IDS {
            assert!(id.starts_with("app."), "id should start with 'app.': {id}");
        }
    }

    #[test]
    fn load_invalid_json_returns_empty() {
        let file = write_temp_json("not valid json {{{");
        let config = load_keybindings_from_file(file.path());
        assert!(config.is_empty());
    }

    #[test]
    fn save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keybindings.json");
        let mut config = KeybindingsConfig::new();
        config.insert(
            "app.interrupt".to_string(),
            KeyBinding::Single("escape".to_string()),
        );
        save_keybindings_to_file(&path, &config).unwrap();
        let reloaded = load_keybindings_from_file(&path);
        assert_eq!(
            reloaded.get("app.interrupt"),
            Some(&KeyBinding::Single("escape".to_string()))
        );
    }
}

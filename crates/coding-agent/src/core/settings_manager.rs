//! Settings manager — loads and merges user + project settings from JSON.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/settings-manager.ts`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::config::CONFIG_DIR_NAME;

// ============================================================================
// Settings types
// ============================================================================

/// Context-window compaction settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSettings {
    /// Whether compaction is enabled (default: true).
    pub enabled: Option<bool>,
    /// Tokens to reserve for prompt + response (default: 16384).
    pub reserve_tokens: Option<u32>,
    /// Tokens to keep as recent context (default: 20000).
    pub keep_recent_tokens: Option<u32>,
}

/// Branch summary settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummarySettings {
    /// Tokens to reserve for prompt + LLM response (default: 16384).
    pub reserve_tokens: Option<u32>,
    /// Skip "Summarize branch?" prompt and default to no summary (default: false).
    pub skip_prompt: Option<bool>,
}

/// Request retry settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrySettings {
    /// Whether retries are enabled (default: true).
    pub enabled: Option<bool>,
    /// Maximum retry attempts (default: 3).
    pub max_retries: Option<u32>,
    /// Base delay in ms for exponential backoff (default: 2000).
    pub base_delay_ms: Option<u64>,
    /// Maximum server-requested delay before failing (default: 60000).
    pub max_delay_ms: Option<u64>,
}

/// Terminal display settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettings {
    /// Show images in terminal (default: true).
    pub show_images: Option<bool>,
    /// Clear empty rows on content shrink (default: false).
    pub clear_on_shrink: Option<bool>,
}

/// Image handling settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSettings {
    /// Auto-resize images to 2000×2000 max (default: true).
    pub auto_resize: Option<bool>,
    /// Block all images from being sent to LLM providers (default: false).
    pub block_images: Option<bool>,
}

/// Custom thinking-budget token counts per level.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThinkingBudgetsSettings {
    pub minimal: Option<u32>,
    pub low: Option<u32>,
    pub medium: Option<u32>,
    pub high: Option<u32>,
}

/// Markdown rendering settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownSettings {
    /// Indentation for code blocks (default: `"  "`).
    pub code_block_indent: Option<String>,
}

/// A package source: either a plain string or a filtered-object form.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PackageSource {
    Simple(String),
    Filtered {
        source: String,
        extensions: Option<Vec<String>>,
        skills: Option<Vec<String>>,
        prompts: Option<Vec<String>>,
        themes: Option<Vec<String>>,
    },
}

impl PackageSource {
    pub fn source(&self) -> &str {
        match self {
            PackageSource::Simple(s) => s,
            PackageSource::Filtered { source, .. } => source,
        }
    }
}

/// The full settings schema (user or project).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub last_changelog_version: Option<String>,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub default_thinking_level: Option<String>,
    /// Transport: "sse" (default) or "stream".
    pub transport: Option<String>,
    pub steering_mode: Option<String>,
    pub follow_up_mode: Option<String>,
    pub theme: Option<String>,
    pub compaction: Option<CompactionSettings>,
    pub branch_summary: Option<BranchSummarySettings>,
    pub retry: Option<RetrySettings>,
    pub hide_thinking_block: Option<bool>,
    /// Custom shell path.
    pub shell_path: Option<String>,
    pub quiet_startup: Option<bool>,
    /// Prefix prepended to every bash command.
    pub shell_command_prefix: Option<String>,
    /// Command used for npm-package lookup/install operations (argv-style).
    pub npm_command: Option<Vec<String>>,
    pub collapse_changelog: Option<bool>,
    pub packages: Option<Vec<PackageSource>>,
    pub extensions: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub prompts: Option<Vec<String>>,
    pub themes: Option<Vec<String>>,
    pub enable_skill_commands: Option<bool>,
    pub terminal: Option<TerminalSettings>,
    pub images: Option<ImageSettings>,
    /// Model patterns for cycling (same format as `--models` CLI flag).
    pub enabled_models: Option<Vec<String>>,
    pub double_escape_action: Option<String>,
    pub tree_filter_mode: Option<String>,
    pub thinking_budgets: Option<ThinkingBudgetsSettings>,
    pub editor_padding_x: Option<u32>,
    pub autocomplete_max_visible: Option<u32>,
    pub show_hardware_cursor: Option<bool>,
    pub markdown: Option<MarkdownSettings>,
    /// Custom session storage directory.
    pub session_dir: Option<String>,
}

/// Scope for settings (which settings.json file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsScope {
    Global,
    Project,
}

impl std::fmt::Display for SettingsScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsScope::Global => write!(f, "global"),
            SettingsScope::Project => write!(f, "project"),
        }
    }
}

/// An error that occurred while loading settings from a specific scope.
#[derive(Debug)]
pub struct SettingsError {
    pub scope: SettingsScope,
    pub error: anyhow::Error,
}

// ============================================================================
// Deep-merge helper
// ============================================================================

/// Deep-merge `overrides` into `base`, returning a new merged `Settings`.
/// Project/override values take precedence; nested objects are merged recursively.
pub fn merge_settings(base: &Settings, overrides: &Settings) -> Settings {
    macro_rules! pick {
        ($field:ident) => {
            overrides.$field.clone().or_else(|| base.$field.clone())
        };
    }
    // Merge nested objects field by field (same semantics as TS `{ ...base, ...override }`)
    let compaction = match (&overrides.compaction, &base.compaction) {
        (Some(o), Some(b)) => Some(CompactionSettings {
            enabled: o.enabled.or(b.enabled),
            reserve_tokens: o.reserve_tokens.or(b.reserve_tokens),
            keep_recent_tokens: o.keep_recent_tokens.or(b.keep_recent_tokens),
        }),
        (Some(o), None) => Some(o.clone()),
        (None, b) => b.clone(),
    };
    let branch_summary = match (&overrides.branch_summary, &base.branch_summary) {
        (Some(o), Some(b)) => Some(BranchSummarySettings {
            reserve_tokens: o.reserve_tokens.or(b.reserve_tokens),
            skip_prompt: o.skip_prompt.or(b.skip_prompt),
        }),
        (Some(o), None) => Some(o.clone()),
        (None, b) => b.clone(),
    };
    let retry = match (&overrides.retry, &base.retry) {
        (Some(o), Some(b)) => Some(RetrySettings {
            enabled: o.enabled.or(b.enabled),
            max_retries: o.max_retries.or(b.max_retries),
            base_delay_ms: o.base_delay_ms.or(b.base_delay_ms),
            max_delay_ms: o.max_delay_ms.or(b.max_delay_ms),
        }),
        (Some(o), None) => Some(o.clone()),
        (None, b) => b.clone(),
    };
    let terminal = match (&overrides.terminal, &base.terminal) {
        (Some(o), Some(b)) => Some(TerminalSettings {
            show_images: o.show_images.or(b.show_images),
            clear_on_shrink: o.clear_on_shrink.or(b.clear_on_shrink),
        }),
        (Some(o), None) => Some(o.clone()),
        (None, b) => b.clone(),
    };
    let images = match (&overrides.images, &base.images) {
        (Some(o), Some(b)) => Some(ImageSettings {
            auto_resize: o.auto_resize.or(b.auto_resize),
            block_images: o.block_images.or(b.block_images),
        }),
        (Some(o), None) => Some(o.clone()),
        (None, b) => b.clone(),
    };
    let markdown = match (&overrides.markdown, &base.markdown) {
        (Some(o), Some(b)) => Some(MarkdownSettings {
            code_block_indent: o
                .code_block_indent
                .clone()
                .or_else(|| b.code_block_indent.clone()),
        }),
        (Some(o), None) => Some(o.clone()),
        (None, b) => b.clone(),
    };
    Settings {
        last_changelog_version: pick!(last_changelog_version),
        default_provider: pick!(default_provider),
        default_model: pick!(default_model),
        default_thinking_level: pick!(default_thinking_level),
        transport: pick!(transport),
        steering_mode: pick!(steering_mode),
        follow_up_mode: pick!(follow_up_mode),
        theme: pick!(theme),
        compaction,
        branch_summary,
        retry,
        hide_thinking_block: pick!(hide_thinking_block),
        shell_path: pick!(shell_path),
        quiet_startup: pick!(quiet_startup),
        shell_command_prefix: pick!(shell_command_prefix),
        npm_command: pick!(npm_command),
        collapse_changelog: pick!(collapse_changelog),
        packages: pick!(packages),
        extensions: pick!(extensions),
        skills: pick!(skills),
        prompts: pick!(prompts),
        themes: pick!(themes),
        enable_skill_commands: pick!(enable_skill_commands),
        terminal,
        images,
        enabled_models: pick!(enabled_models),
        double_escape_action: pick!(double_escape_action),
        tree_filter_mode: pick!(tree_filter_mode),
        thinking_budgets: pick!(thinking_budgets),
        editor_padding_x: pick!(editor_padding_x),
        autocomplete_max_visible: pick!(autocomplete_max_visible),
        show_hardware_cursor: pick!(show_hardware_cursor),
        markdown,
        session_dir: pick!(session_dir),
    }
}

// ============================================================================
// Storage trait
// ============================================================================

/// Storage backend for settings — synchronous, lock-based access.
pub trait SettingsStorage: Send + Sync {
    /// Call `f` with the current raw JSON string (or `None` if not yet persisted),
    /// and write the returned string back if `Some`.
    fn with_lock(&self, scope: SettingsScope, f: &mut dyn FnMut(Option<&str>) -> Option<String>);
}

// ============================================================================
// InMemorySettingsStorage
// ============================================================================

/// In-memory storage backend — no file I/O, useful for tests.
#[derive(Default)]
pub struct InMemorySettingsStorage {
    global: Mutex<Option<String>>,
    project: Mutex<Option<String>>,
}

impl SettingsStorage for InMemorySettingsStorage {
    fn with_lock(&self, scope: SettingsScope, f: &mut dyn FnMut(Option<&str>) -> Option<String>) {
        match scope {
            SettingsScope::Global => {
                let mut guard = self.global.lock().unwrap();
                let current = guard.as_deref();
                if let Some(next) = f(current) {
                    *guard = Some(next);
                }
            }
            SettingsScope::Project => {
                let mut guard = self.project.lock().unwrap();
                let current = guard.as_deref();
                if let Some(next) = f(current) {
                    *guard = Some(next);
                }
            }
        }
    }
}

// ============================================================================
// FileSettingsStorage
// ============================================================================

/// File-based storage backend with mutex-based locking.
pub struct FileSettingsStorage {
    global_settings_path: PathBuf,
    project_settings_path: PathBuf,
    /// Per-scope mutex to prevent concurrent writes.
    global_lock: Mutex<()>,
    project_lock: Mutex<()>,
}

impl FileSettingsStorage {
    pub fn new(cwd: impl AsRef<Path>, agent_dir: impl AsRef<Path>) -> Self {
        Self {
            global_settings_path: agent_dir.as_ref().join("settings.json"),
            project_settings_path: cwd.as_ref().join(CONFIG_DIR_NAME).join("settings.json"),
            global_lock: Mutex::new(()),
            project_lock: Mutex::new(()),
        }
    }
}

impl SettingsStorage for FileSettingsStorage {
    fn with_lock(&self, scope: SettingsScope, f: &mut dyn FnMut(Option<&str>) -> Option<String>) {
        let path = match scope {
            SettingsScope::Global => &self.global_settings_path,
            SettingsScope::Project => &self.project_settings_path,
        };
        // Hold scope-specific mutex for the duration of read+write.
        let _guard = match scope {
            SettingsScope::Global => self.global_lock.lock().unwrap(),
            SettingsScope::Project => self.project_lock.lock().unwrap(),
        };

        let current_content = if path.exists() {
            std::fs::read_to_string(path).ok()
        } else {
            None
        };

        let next = f(current_content.as_deref());
        if let Some(content) = next {
            // Only create directory when we actually need to write.
            if let Some(parent) = path.parent() {
                if !parent.exists() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            let _ = std::fs::write(path, content);
        }
    }
}

// ============================================================================
// Settings migration
// ============================================================================

/// Migrate old settings field names to current ones.
fn migrate_settings(raw: serde_json::Value) -> anyhow::Result<Settings> {
    let mut map = match raw {
        serde_json::Value::Object(m) => m,
        other => {
            return serde_json::from_value(other).map_err(|e| anyhow::anyhow!(e));
        }
    };

    // Migrate queueMode -> steeringMode
    if map.contains_key("queueMode") && !map.contains_key("steeringMode") {
        let v = map.remove("queueMode").unwrap();
        map.insert("steeringMode".to_string(), v);
    }

    // Migrate legacy websockets boolean -> transport enum
    if !map.contains_key("transport") {
        if let Some(ws) = map.get("websockets").and_then(|v| v.as_bool()) {
            map.insert(
                "transport".to_string(),
                serde_json::Value::String(if ws { "websocket" } else { "sse" }.to_string()),
            );
            map.remove("websockets");
        }
    }

    // Migrate old skills object format to new array format
    if let Some(skills_val) = map.get("skills").cloned() {
        if let serde_json::Value::Object(skills_obj) = skills_val {
            // enableSkillCommands migration
            if let Some(esc) = skills_obj.get("enableSkillCommands") {
                if !map.contains_key("enableSkillCommands") {
                    map.insert("enableSkillCommands".to_string(), esc.clone());
                }
            }
            // customDirectories migration
            if let Some(dirs) = skills_obj.get("customDirectories") {
                if let serde_json::Value::Array(arr) = dirs {
                    if !arr.is_empty() {
                        map.insert("skills".to_string(), serde_json::Value::Array(arr.clone()));
                    } else {
                        map.remove("skills");
                    }
                } else {
                    map.remove("skills");
                }
            } else {
                map.remove("skills");
            }
        }
    }

    serde_json::from_value(serde_json::Value::Object(map)).map_err(|e| anyhow::anyhow!(e))
}

// ============================================================================
// SettingsManager
// ============================================================================

/// Which Settings field was modified — used for selective persistence.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SettingsField {
    LastChangelogVersion,
    DefaultProvider,
    DefaultModel,
    DefaultThinkingLevel,
    Transport,
    SteeringMode,
    FollowUpMode,
    Theme,
    Compaction,
    BranchSummary,
    Retry,
    HideThinkingBlock,
    ShellPath,
    QuietStartup,
    ShellCommandPrefix,
    NpmCommand,
    CollapseChangelog,
    Packages,
    Extensions,
    Skills,
    Prompts,
    Themes,
    EnableSkillCommands,
    Terminal,
    Images,
    EnabledModels,
    DoubleEscapeAction,
    TreeFilterMode,
    ThinkingBudgets,
    EditorPaddingX,
    AutocompleteMaxVisible,
    ShowHardwareCursor,
    Markdown,
    SessionDir,
}

/// Manages loading and saving of user + project settings.
pub struct SettingsManager {
    storage: Arc<dyn SettingsStorage>,
    global_settings: Settings,
    project_settings: Settings,
    /// Effective merged settings (project overrides global).
    settings: Settings,
    global_settings_load_error: Option<anyhow::Error>,
    project_settings_load_error: Option<anyhow::Error>,
    /// Fields modified in global scope during this session.
    modified_fields: HashSet<SettingsField>,
    /// Nested keys modified within a global field (field -> set of nested keys).
    modified_nested_fields: HashMap<SettingsField, HashSet<String>>,
    /// Fields modified in project scope during this session.
    modified_project_fields: HashSet<SettingsField>,
    modified_project_nested_fields: HashMap<SettingsField, HashSet<String>>,
    /// Accumulated errors.
    errors: Vec<SettingsError>,
}

impl SettingsManager {
    fn new(
        storage: Arc<dyn SettingsStorage>,
        global_settings: Settings,
        project_settings: Settings,
        global_load_error: Option<anyhow::Error>,
        project_load_error: Option<anyhow::Error>,
        initial_errors: Vec<SettingsError>,
    ) -> Self {
        let settings = merge_settings(&global_settings, &project_settings);
        Self {
            storage,
            global_settings,
            project_settings,
            settings,
            global_settings_load_error: global_load_error,
            project_settings_load_error: project_load_error,
            modified_fields: HashSet::new(),
            modified_nested_fields: HashMap::new(),
            modified_project_fields: HashSet::new(),
            modified_project_nested_fields: HashMap::new(),
            errors: initial_errors,
        }
    }

    /// Create a SettingsManager that loads from files.
    pub fn create(cwd: impl AsRef<Path>, agent_dir: impl AsRef<Path>) -> Self {
        let storage = Arc::new(FileSettingsStorage::new(cwd, agent_dir));
        Self::from_storage(storage)
    }

    /// Create a SettingsManager from an arbitrary storage backend.
    pub fn from_storage(storage: Arc<dyn SettingsStorage>) -> Self {
        let global_load = Self::try_load_from_storage(storage.as_ref(), SettingsScope::Global);
        let project_load = Self::try_load_from_storage(storage.as_ref(), SettingsScope::Project);
        let mut initial_errors = Vec::new();
        let global_err = global_load.1;
        let project_err = project_load.1;
        if let Some(ref e) = global_err {
            initial_errors.push(SettingsError {
                scope: SettingsScope::Global,
                error: anyhow::anyhow!("{}", e),
            });
        }
        if let Some(ref e) = project_err {
            initial_errors.push(SettingsError {
                scope: SettingsScope::Project,
                error: anyhow::anyhow!("{}", e),
            });
        }
        Self::new(
            storage,
            global_load.0,
            project_load.0,
            global_err,
            project_err,
            initial_errors,
        )
    }

    /// Create an in-memory SettingsManager (no file I/O).
    pub fn in_memory(global: Settings) -> Self {
        let storage = Arc::new(InMemorySettingsStorage::default());
        // Serialize initial global settings into storage.
        if let Ok(json) = serde_json::to_string(&global) {
            let json_clone = json.clone();
            storage.with_lock(SettingsScope::Global, &mut |_| Some(json_clone.clone()));
        }
        Self::from_storage(storage)
    }

    fn load_from_storage(
        storage: &dyn SettingsStorage,
        scope: SettingsScope,
    ) -> anyhow::Result<Settings> {
        let mut content: Option<String> = None;
        storage.with_lock(scope, &mut |current| {
            content = current.map(|s| s.to_owned());
            None
        });
        match content {
            None => Ok(Settings::default()),
            Some(s) => {
                let raw: serde_json::Value = serde_json::from_str(&s)?;
                migrate_settings(raw)
            }
        }
    }

    fn try_load_from_storage(
        storage: &dyn SettingsStorage,
        scope: SettingsScope,
    ) -> (Settings, Option<anyhow::Error>) {
        match Self::load_from_storage(storage, scope) {
            Ok(s) => (s, None),
            Err(e) => (Settings::default(), Some(e)),
        }
    }

    /// Reload settings from storage, clearing modification tracking.
    pub fn reload(&mut self) {
        let global_load = Self::try_load_from_storage(self.storage.as_ref(), SettingsScope::Global);
        if global_load.1.is_none() {
            self.global_settings = global_load.0;
            self.global_settings_load_error = None;
        } else {
            self.global_settings_load_error = global_load.1;
            if let Some(ref e) = self.global_settings_load_error {
                self.errors.push(SettingsError {
                    scope: SettingsScope::Global,
                    error: anyhow::anyhow!("{}", e),
                });
            }
        }

        self.modified_fields.clear();
        self.modified_nested_fields.clear();
        self.modified_project_fields.clear();
        self.modified_project_nested_fields.clear();

        let project_load =
            Self::try_load_from_storage(self.storage.as_ref(), SettingsScope::Project);
        if project_load.1.is_none() {
            self.project_settings = project_load.0;
            self.project_settings_load_error = None;
        } else {
            self.project_settings_load_error = project_load.1;
            if let Some(ref e) = self.project_settings_load_error {
                self.errors.push(SettingsError {
                    scope: SettingsScope::Project,
                    error: anyhow::anyhow!("{}", e),
                });
            }
        }

        self.settings = merge_settings(&self.global_settings, &self.project_settings);
    }

    /// Apply additional overrides on top of current effective settings.
    pub fn apply_overrides(&mut self, overrides: &Settings) {
        self.settings = merge_settings(&self.settings, overrides);
    }

    /// Drain accumulated parse/IO errors.
    pub fn drain_errors(&mut self) -> Vec<SettingsError> {
        std::mem::take(&mut self.errors)
    }

    /// Get the merged effective settings (project overrides global).
    pub fn get_effective_settings(&self) -> Settings {
        self.settings.clone()
    }

    /// Get the raw global settings.
    pub fn get_global_settings(&self) -> Settings {
        self.global_settings.clone()
    }

    /// Get the raw project settings.
    pub fn get_project_settings(&self) -> Settings {
        self.project_settings.clone()
    }

    // -------------------------------------------------------------------------
    // Modification tracking helpers
    // -------------------------------------------------------------------------

    fn mark_modified(&mut self, field: SettingsField, nested_key: Option<&str>) {
        self.modified_fields.insert(field.clone());
        if let Some(key) = nested_key {
            self.modified_nested_fields
                .entry(field)
                .or_default()
                .insert(key.to_owned());
        }
    }

    fn mark_project_modified(&mut self, field: SettingsField, nested_key: Option<&str>) {
        self.modified_project_fields.insert(field.clone());
        if let Some(key) = nested_key {
            self.modified_project_nested_fields
                .entry(field)
                .or_default()
                .insert(key.to_owned());
        }
    }

    fn clear_modified_scope(&mut self, scope: SettingsScope) {
        match scope {
            SettingsScope::Global => {
                self.modified_fields.clear();
                self.modified_nested_fields.clear();
            }
            SettingsScope::Project => {
                self.modified_project_fields.clear();
                self.modified_project_nested_fields.clear();
            }
        }
    }

    // -------------------------------------------------------------------------
    // Persist helpers
    // -------------------------------------------------------------------------

    /// Merge only the explicitly-modified fields into the on-disk representation,
    /// preserving fields that were externally edited.
    fn persist_scoped_settings(
        &self,
        scope: SettingsScope,
        snapshot: &Settings,
        modified_fields: &HashSet<SettingsField>,
        modified_nested_fields: &HashMap<SettingsField, HashSet<String>>,
    ) {
        let snapshot_value =
            serde_json::to_value(snapshot).unwrap_or(serde_json::Value::Object(Default::default()));
        let snapshot_map = match snapshot_value {
            serde_json::Value::Object(m) => m,
            _ => Default::default(),
        };

        self.storage.with_lock(scope, &mut |current| {
            // Parse current on-disk settings (or start fresh).
            let mut current_map: serde_json::Map<String, serde_json::Value> = current
                .and_then(|s| serde_json::from_str(s).ok())
                .and_then(|v: serde_json::Value| v.as_object().cloned())
                .unwrap_or_default();

            // For each modified field, write from snapshot (possibly merging nested).
            for field in modified_fields {
                let field_key = settings_field_to_json_key(field);
                if let Some(nested_keys) = modified_nested_fields.get(field) {
                    // Merge nested object: keep existing keys, overwrite only modified ones.
                    let snapshot_nested = snapshot_map
                        .get(field_key)
                        .and_then(|v| v.as_object().cloned())
                        .unwrap_or_default();
                    let base_nested = current_map
                        .get(field_key)
                        .and_then(|v| v.as_object().cloned())
                        .unwrap_or_default();
                    let mut merged_nested = base_nested;
                    for key in nested_keys {
                        if let Some(val) = snapshot_nested.get(key) {
                            merged_nested.insert(key.clone(), val.clone());
                        }
                    }
                    current_map.insert(
                        field_key.to_owned(),
                        serde_json::Value::Object(merged_nested),
                    );
                } else if let Some(val) = snapshot_map.get(field_key) {
                    current_map.insert(field_key.to_owned(), val.clone());
                } else {
                    // Field was explicitly set to None — remove it.
                    current_map.remove(field_key);
                }
            }

            Some(
                serde_json::to_string_pretty(&serde_json::Value::Object(current_map))
                    .unwrap_or_default(),
            )
        });
    }

    /// Save global settings (only modified fields) synchronously.
    fn save(&mut self) {
        self.settings = merge_settings(&self.global_settings, &self.project_settings);

        if self.global_settings_load_error.is_some() {
            return;
        }

        let snapshot = self.global_settings.clone();
        let modified_fields = self.modified_fields.clone();
        let modified_nested_fields = self.modified_nested_fields.clone();

        self.persist_scoped_settings(
            SettingsScope::Global,
            &snapshot,
            &modified_fields,
            &modified_nested_fields,
        );
        self.clear_modified_scope(SettingsScope::Global);
    }

    /// Save project settings (only modified fields) synchronously.
    fn save_project_settings(&mut self, settings: Settings) {
        self.project_settings = settings;
        self.settings = merge_settings(&self.global_settings, &self.project_settings);

        if self.project_settings_load_error.is_some() {
            return;
        }

        let snapshot = self.project_settings.clone();
        let modified_fields = self.modified_project_fields.clone();
        let modified_nested_fields = self.modified_project_nested_fields.clone();

        self.persist_scoped_settings(
            SettingsScope::Project,
            &snapshot,
            &modified_fields,
            &modified_nested_fields,
        );
        self.clear_modified_scope(SettingsScope::Project);
    }

    /// Save settings to the specified scope (full write, no field tracking).
    /// Used by external callers that own the full settings object.
    pub fn save_settings(&self, settings: &Settings, scope: SettingsScope) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(settings)?;
        let mut write_error: Option<anyhow::Error> = None;
        self.storage.with_lock(scope, &mut |_| {
            let _ = write_error.take(); // just in case
            Some(json.clone())
        });
        if let Some(e) = write_error {
            return Err(e);
        }
        Ok(())
    }

    // =========================================================================
    // Getters & setters
    // =========================================================================

    pub fn get_last_changelog_version(&self) -> Option<&str> {
        self.settings.last_changelog_version.as_deref()
    }

    pub fn set_last_changelog_version(&mut self, version: &str) {
        self.global_settings.last_changelog_version = Some(version.to_owned());
        self.mark_modified(SettingsField::LastChangelogVersion, None);
        self.save();
    }

    pub fn get_session_dir(&self) -> Option<&str> {
        self.settings.session_dir.as_deref()
    }

    pub fn get_default_provider(&self) -> Option<&str> {
        self.settings.default_provider.as_deref()
    }

    pub fn get_default_model(&self) -> Option<&str> {
        self.settings.default_model.as_deref()
    }

    pub fn set_default_provider(&mut self, provider: &str) {
        self.global_settings.default_provider = Some(provider.to_owned());
        self.mark_modified(SettingsField::DefaultProvider, None);
        self.save();
    }

    pub fn set_default_model(&mut self, model_id: &str) {
        self.global_settings.default_model = Some(model_id.to_owned());
        self.mark_modified(SettingsField::DefaultModel, None);
        self.save();
    }

    pub fn set_default_model_and_provider(&mut self, provider: &str, model_id: &str) {
        self.global_settings.default_provider = Some(provider.to_owned());
        self.global_settings.default_model = Some(model_id.to_owned());
        self.mark_modified(SettingsField::DefaultProvider, None);
        self.mark_modified(SettingsField::DefaultModel, None);
        self.save();
    }

    pub fn get_steering_mode(&self) -> &str {
        self.settings
            .steering_mode
            .as_deref()
            .unwrap_or("one-at-a-time")
    }

    pub fn set_steering_mode(&mut self, mode: &str) {
        self.global_settings.steering_mode = Some(mode.to_owned());
        self.mark_modified(SettingsField::SteeringMode, None);
        self.save();
    }

    pub fn get_follow_up_mode(&self) -> &str {
        self.settings
            .follow_up_mode
            .as_deref()
            .unwrap_or("one-at-a-time")
    }

    pub fn set_follow_up_mode(&mut self, mode: &str) {
        self.global_settings.follow_up_mode = Some(mode.to_owned());
        self.mark_modified(SettingsField::FollowUpMode, None);
        self.save();
    }

    pub fn get_theme(&self) -> Option<&str> {
        self.settings.theme.as_deref()
    }

    pub fn set_theme(&mut self, theme: &str) {
        self.global_settings.theme = Some(theme.to_owned());
        self.mark_modified(SettingsField::Theme, None);
        self.save();
    }

    pub fn get_default_thinking_level(&self) -> Option<&str> {
        self.settings.default_thinking_level.as_deref()
    }

    pub fn set_default_thinking_level(&mut self, level: &str) {
        self.global_settings.default_thinking_level = Some(level.to_owned());
        self.mark_modified(SettingsField::DefaultThinkingLevel, None);
        self.save();
    }

    pub fn get_transport(&self) -> &str {
        self.settings.transport.as_deref().unwrap_or("sse")
    }

    pub fn set_transport(&mut self, transport: &str) {
        self.global_settings.transport = Some(transport.to_owned());
        self.mark_modified(SettingsField::Transport, None);
        self.save();
    }

    pub fn get_compaction_enabled(&self) -> bool {
        self.settings
            .compaction
            .as_ref()
            .and_then(|c| c.enabled)
            .unwrap_or(true)
    }

    pub fn set_compaction_enabled(&mut self, enabled: bool) {
        if self.global_settings.compaction.is_none() {
            self.global_settings.compaction = Some(CompactionSettings::default());
        }
        self.global_settings.compaction.as_mut().unwrap().enabled = Some(enabled);
        self.mark_modified(SettingsField::Compaction, Some("enabled"));
        self.save();
    }

    pub fn get_compaction_reserve_tokens(&self) -> u32 {
        self.settings
            .compaction
            .as_ref()
            .and_then(|c| c.reserve_tokens)
            .unwrap_or(16384)
    }

    pub fn get_compaction_keep_recent_tokens(&self) -> u32 {
        self.settings
            .compaction
            .as_ref()
            .and_then(|c| c.keep_recent_tokens)
            .unwrap_or(20000)
    }

    /// Returns a snapshot of compaction settings with defaults applied.
    pub fn get_compaction_settings(&self) -> CompactionSettings {
        CompactionSettings {
            enabled: Some(self.get_compaction_enabled()),
            reserve_tokens: Some(self.get_compaction_reserve_tokens()),
            keep_recent_tokens: Some(self.get_compaction_keep_recent_tokens()),
        }
    }

    pub fn get_branch_summary_settings(&self) -> BranchSummarySettings {
        BranchSummarySettings {
            reserve_tokens: Some(
                self.settings
                    .branch_summary
                    .as_ref()
                    .and_then(|b| b.reserve_tokens)
                    .unwrap_or(16384),
            ),
            skip_prompt: Some(
                self.settings
                    .branch_summary
                    .as_ref()
                    .and_then(|b| b.skip_prompt)
                    .unwrap_or(false),
            ),
        }
    }

    pub fn get_branch_summary_skip_prompt(&self) -> bool {
        self.settings
            .branch_summary
            .as_ref()
            .and_then(|b| b.skip_prompt)
            .unwrap_or(false)
    }

    pub fn get_retry_enabled(&self) -> bool {
        self.settings
            .retry
            .as_ref()
            .and_then(|r| r.enabled)
            .unwrap_or(true)
    }

    pub fn set_retry_enabled(&mut self, enabled: bool) {
        if self.global_settings.retry.is_none() {
            self.global_settings.retry = Some(RetrySettings::default());
        }
        self.global_settings.retry.as_mut().unwrap().enabled = Some(enabled);
        self.mark_modified(SettingsField::Retry, Some("enabled"));
        self.save();
    }

    /// Returns a snapshot of retry settings with defaults applied.
    pub fn get_retry_settings(&self) -> RetrySettings {
        RetrySettings {
            enabled: Some(self.get_retry_enabled()),
            max_retries: Some(
                self.settings
                    .retry
                    .as_ref()
                    .and_then(|r| r.max_retries)
                    .unwrap_or(3),
            ),
            base_delay_ms: Some(
                self.settings
                    .retry
                    .as_ref()
                    .and_then(|r| r.base_delay_ms)
                    .unwrap_or(2000),
            ),
            max_delay_ms: Some(
                self.settings
                    .retry
                    .as_ref()
                    .and_then(|r| r.max_delay_ms)
                    .unwrap_or(60000),
            ),
        }
    }

    pub fn get_hide_thinking_block(&self) -> bool {
        self.settings.hide_thinking_block.unwrap_or(false)
    }

    pub fn set_hide_thinking_block(&mut self, hide: bool) {
        self.global_settings.hide_thinking_block = Some(hide);
        self.mark_modified(SettingsField::HideThinkingBlock, None);
        self.save();
    }

    pub fn get_shell_path(&self) -> Option<&str> {
        self.settings.shell_path.as_deref()
    }

    pub fn set_shell_path(&mut self, path: Option<&str>) {
        self.global_settings.shell_path = path.map(|s| s.to_owned());
        self.mark_modified(SettingsField::ShellPath, None);
        self.save();
    }

    pub fn get_quiet_startup(&self) -> bool {
        self.settings.quiet_startup.unwrap_or(false)
    }

    pub fn set_quiet_startup(&mut self, quiet: bool) {
        self.global_settings.quiet_startup = Some(quiet);
        self.mark_modified(SettingsField::QuietStartup, None);
        self.save();
    }

    pub fn get_shell_command_prefix(&self) -> Option<&str> {
        self.settings.shell_command_prefix.as_deref()
    }

    pub fn set_shell_command_prefix(&mut self, prefix: Option<&str>) {
        self.global_settings.shell_command_prefix = prefix.map(|s| s.to_owned());
        self.mark_modified(SettingsField::ShellCommandPrefix, None);
        self.save();
    }

    pub fn get_npm_command(&self) -> Option<Vec<String>> {
        self.settings.npm_command.as_ref().map(|v| v.clone())
    }

    pub fn set_npm_command(&mut self, command: Option<Vec<String>>) {
        self.global_settings.npm_command = command;
        self.mark_modified(SettingsField::NpmCommand, None);
        self.save();
    }

    pub fn get_collapse_changelog(&self) -> bool {
        self.settings.collapse_changelog.unwrap_or(false)
    }

    pub fn set_collapse_changelog(&mut self, collapse: bool) {
        self.global_settings.collapse_changelog = Some(collapse);
        self.mark_modified(SettingsField::CollapseChangelog, None);
        self.save();
    }

    pub fn get_packages(&self) -> Vec<PackageSource> {
        self.settings.packages.clone().unwrap_or_default()
    }

    pub fn set_packages(&mut self, packages: Vec<PackageSource>) {
        self.global_settings.packages = Some(packages);
        self.mark_modified(SettingsField::Packages, None);
        self.save();
    }

    pub fn set_project_packages(&mut self, packages: Vec<PackageSource>) {
        let mut project = self.project_settings.clone();
        project.packages = Some(packages);
        self.mark_project_modified(SettingsField::Packages, None);
        self.save_project_settings(project);
    }

    pub fn get_extension_paths(&self) -> Vec<String> {
        self.settings.extensions.clone().unwrap_or_default()
    }

    pub fn set_extension_paths(&mut self, paths: Vec<String>) {
        self.global_settings.extensions = Some(paths);
        self.mark_modified(SettingsField::Extensions, None);
        self.save();
    }

    pub fn set_project_extension_paths(&mut self, paths: Vec<String>) {
        let mut project = self.project_settings.clone();
        project.extensions = Some(paths);
        self.mark_project_modified(SettingsField::Extensions, None);
        self.save_project_settings(project);
    }

    pub fn get_skill_paths(&self) -> Vec<String> {
        self.settings.skills.clone().unwrap_or_default()
    }

    pub fn set_skill_paths(&mut self, paths: Vec<String>) {
        self.global_settings.skills = Some(paths);
        self.mark_modified(SettingsField::Skills, None);
        self.save();
    }

    pub fn set_project_skill_paths(&mut self, paths: Vec<String>) {
        let mut project = self.project_settings.clone();
        project.skills = Some(paths);
        self.mark_project_modified(SettingsField::Skills, None);
        self.save_project_settings(project);
    }

    pub fn get_prompt_template_paths(&self) -> Vec<String> {
        self.settings.prompts.clone().unwrap_or_default()
    }

    pub fn set_prompt_template_paths(&mut self, paths: Vec<String>) {
        self.global_settings.prompts = Some(paths);
        self.mark_modified(SettingsField::Prompts, None);
        self.save();
    }

    pub fn set_project_prompt_template_paths(&mut self, paths: Vec<String>) {
        let mut project = self.project_settings.clone();
        project.prompts = Some(paths);
        self.mark_project_modified(SettingsField::Prompts, None);
        self.save_project_settings(project);
    }

    pub fn get_theme_paths(&self) -> Vec<String> {
        self.settings.themes.clone().unwrap_or_default()
    }

    pub fn set_theme_paths(&mut self, paths: Vec<String>) {
        self.global_settings.themes = Some(paths);
        self.mark_modified(SettingsField::Themes, None);
        self.save();
    }

    pub fn set_project_theme_paths(&mut self, paths: Vec<String>) {
        let mut project = self.project_settings.clone();
        project.themes = Some(paths);
        self.mark_project_modified(SettingsField::Themes, None);
        self.save_project_settings(project);
    }

    pub fn get_enable_skill_commands(&self) -> bool {
        self.settings.enable_skill_commands.unwrap_or(true)
    }

    pub fn set_enable_skill_commands(&mut self, enabled: bool) {
        self.global_settings.enable_skill_commands = Some(enabled);
        self.mark_modified(SettingsField::EnableSkillCommands, None);
        self.save();
    }

    pub fn get_thinking_budgets(&self) -> Option<&ThinkingBudgetsSettings> {
        self.settings.thinking_budgets.as_ref()
    }

    pub fn get_show_images(&self) -> bool {
        self.settings
            .terminal
            .as_ref()
            .and_then(|t| t.show_images)
            .unwrap_or(true)
    }

    pub fn set_show_images(&mut self, show: bool) {
        if self.global_settings.terminal.is_none() {
            self.global_settings.terminal = Some(TerminalSettings::default());
        }
        self.global_settings.terminal.as_mut().unwrap().show_images = Some(show);
        self.mark_modified(SettingsField::Terminal, Some("showImages"));
        self.save();
    }

    pub fn get_clear_on_shrink(&self) -> bool {
        if let Some(val) = self
            .settings
            .terminal
            .as_ref()
            .and_then(|t| t.clear_on_shrink)
        {
            return val;
        }
        std::env::var("SAGE_CLEAR_ON_SHRINK").as_deref() == Ok("1")
    }

    pub fn set_clear_on_shrink(&mut self, enabled: bool) {
        if self.global_settings.terminal.is_none() {
            self.global_settings.terminal = Some(TerminalSettings::default());
        }
        self.global_settings
            .terminal
            .as_mut()
            .unwrap()
            .clear_on_shrink = Some(enabled);
        self.mark_modified(SettingsField::Terminal, Some("clearOnShrink"));
        self.save();
    }

    pub fn get_image_auto_resize(&self) -> bool {
        self.settings
            .images
            .as_ref()
            .and_then(|i| i.auto_resize)
            .unwrap_or(true)
    }

    pub fn set_image_auto_resize(&mut self, enabled: bool) {
        if self.global_settings.images.is_none() {
            self.global_settings.images = Some(ImageSettings::default());
        }
        self.global_settings.images.as_mut().unwrap().auto_resize = Some(enabled);
        self.mark_modified(SettingsField::Images, Some("autoResize"));
        self.save();
    }

    pub fn get_block_images(&self) -> bool {
        self.settings
            .images
            .as_ref()
            .and_then(|i| i.block_images)
            .unwrap_or(false)
    }

    pub fn set_block_images(&mut self, blocked: bool) {
        if self.global_settings.images.is_none() {
            self.global_settings.images = Some(ImageSettings::default());
        }
        self.global_settings.images.as_mut().unwrap().block_images = Some(blocked);
        self.mark_modified(SettingsField::Images, Some("blockImages"));
        self.save();
    }

    pub fn get_enabled_models(&self) -> Option<Vec<String>> {
        self.settings.enabled_models.clone()
    }

    pub fn set_enabled_models(&mut self, patterns: Option<Vec<String>>) {
        self.global_settings.enabled_models = patterns;
        self.mark_modified(SettingsField::EnabledModels, None);
        self.save();
    }

    pub fn get_double_escape_action(&self) -> &str {
        self.settings
            .double_escape_action
            .as_deref()
            .unwrap_or("tree")
    }

    pub fn set_double_escape_action(&mut self, action: &str) {
        self.global_settings.double_escape_action = Some(action.to_owned());
        self.mark_modified(SettingsField::DoubleEscapeAction, None);
        self.save();
    }

    pub fn get_tree_filter_mode(&self) -> &str {
        let valid = ["default", "no-tools", "user-only", "labeled-only", "all"];
        let mode = self
            .settings
            .tree_filter_mode
            .as_deref()
            .unwrap_or("default");
        if valid.contains(&mode) {
            mode
        } else {
            "default"
        }
    }

    pub fn set_tree_filter_mode(&mut self, mode: &str) {
        self.global_settings.tree_filter_mode = Some(mode.to_owned());
        self.mark_modified(SettingsField::TreeFilterMode, None);
        self.save();
    }

    pub fn get_show_hardware_cursor(&self) -> bool {
        if let Some(v) = self.settings.show_hardware_cursor {
            return v;
        }
        std::env::var("SAGE_HARDWARE_CURSOR").as_deref() == Ok("1")
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        self.global_settings.show_hardware_cursor = Some(enabled);
        self.mark_modified(SettingsField::ShowHardwareCursor, None);
        self.save();
    }

    pub fn get_editor_padding_x(&self) -> u32 {
        self.settings.editor_padding_x.unwrap_or(0)
    }

    pub fn set_editor_padding_x(&mut self, padding: u32) {
        self.global_settings.editor_padding_x = Some(padding.min(3));
        self.mark_modified(SettingsField::EditorPaddingX, None);
        self.save();
    }

    pub fn get_autocomplete_max_visible(&self) -> u32 {
        self.settings.autocomplete_max_visible.unwrap_or(5)
    }

    pub fn set_autocomplete_max_visible(&mut self, max_visible: u32) {
        self.global_settings.autocomplete_max_visible = Some(max_visible.clamp(3, 20));
        self.mark_modified(SettingsField::AutocompleteMaxVisible, None);
        self.save();
    }

    pub fn get_code_block_indent(&self) -> &str {
        self.settings
            .markdown
            .as_ref()
            .and_then(|m| m.code_block_indent.as_deref())
            .unwrap_or("  ")
    }
}

/// Map a `SettingsField` to its camelCase JSON key.
fn settings_field_to_json_key(field: &SettingsField) -> &'static str {
    match field {
        SettingsField::LastChangelogVersion => "lastChangelogVersion",
        SettingsField::DefaultProvider => "defaultProvider",
        SettingsField::DefaultModel => "defaultModel",
        SettingsField::DefaultThinkingLevel => "defaultThinkingLevel",
        SettingsField::Transport => "transport",
        SettingsField::SteeringMode => "steeringMode",
        SettingsField::FollowUpMode => "followUpMode",
        SettingsField::Theme => "theme",
        SettingsField::Compaction => "compaction",
        SettingsField::BranchSummary => "branchSummary",
        SettingsField::Retry => "retry",
        SettingsField::HideThinkingBlock => "hideThinkingBlock",
        SettingsField::ShellPath => "shellPath",
        SettingsField::QuietStartup => "quietStartup",
        SettingsField::ShellCommandPrefix => "shellCommandPrefix",
        SettingsField::NpmCommand => "npmCommand",
        SettingsField::CollapseChangelog => "collapseChangelog",
        SettingsField::Packages => "packages",
        SettingsField::Extensions => "extensions",
        SettingsField::Skills => "skills",
        SettingsField::Prompts => "prompts",
        SettingsField::Themes => "themes",
        SettingsField::EnableSkillCommands => "enableSkillCommands",
        SettingsField::Terminal => "terminal",
        SettingsField::Images => "images",
        SettingsField::EnabledModels => "enabledModels",
        SettingsField::DoubleEscapeAction => "doubleEscapeAction",
        SettingsField::TreeFilterMode => "treeFilterMode",
        SettingsField::ThinkingBudgets => "thinkingBudgets",
        SettingsField::EditorPaddingX => "editorPaddingX",
        SettingsField::AutocompleteMaxVisible => "autocompleteMaxVisible",
        SettingsField::ShowHardwareCursor => "showHardwareCursor",
        SettingsField::Markdown => "markdown",
        SettingsField::SessionDir => "sessionDir",
    }
}

// ============================================================================
// Keybindings migration
//
// Translated from pi-mono `packages/coding-agent/src/core/keybindings.ts`:
// `migrateKeybindingsConfigFile()` and `KEYBINDING_NAME_MIGRATIONS`.
// ============================================================================

/// Map from old (un-namespaced) keybinding names to new namespaced names.
///
/// Mirrors `KEYBINDING_NAME_MIGRATIONS` from TypeScript.
pub fn keybinding_name_migrations() -> HashMap<&'static str, &'static str> {
    let mut m: HashMap<&'static str, &'static str> = HashMap::new();
    // Editor cursor movement
    m.insert("cursorUp", "tui.editor.cursorUp");
    m.insert("cursorDown", "tui.editor.cursorDown");
    m.insert("cursorLeft", "tui.editor.cursorLeft");
    m.insert("cursorRight", "tui.editor.cursorRight");
    m.insert("cursorWordLeft", "tui.editor.cursorWordLeft");
    m.insert("cursorWordRight", "tui.editor.cursorWordRight");
    m.insert("cursorLineStart", "tui.editor.cursorLineStart");
    m.insert("cursorLineEnd", "tui.editor.cursorLineEnd");
    m.insert("jumpForward", "tui.editor.jumpForward");
    m.insert("jumpBackward", "tui.editor.jumpBackward");
    m.insert("pageUp", "tui.editor.pageUp");
    m.insert("pageDown", "tui.editor.pageDown");
    // Editor deletion
    m.insert("deleteCharBackward", "tui.editor.deleteCharBackward");
    m.insert("deleteCharForward", "tui.editor.deleteCharForward");
    m.insert("deleteWordBackward", "tui.editor.deleteWordBackward");
    m.insert("deleteWordForward", "tui.editor.deleteWordForward");
    m.insert("deleteToLineStart", "tui.editor.deleteToLineStart");
    m.insert("deleteToLineEnd", "tui.editor.deleteToLineEnd");
    m.insert("yank", "tui.editor.yank");
    m.insert("yankPop", "tui.editor.yankPop");
    m.insert("undo", "tui.editor.undo");
    // Input
    m.insert("newLine", "tui.input.newLine");
    m.insert("submit", "tui.input.submit");
    m.insert("tab", "tui.input.tab");
    m.insert("copy", "tui.input.copy");
    // Select
    m.insert("selectUp", "tui.select.up");
    m.insert("selectDown", "tui.select.down");
    m.insert("selectPageUp", "tui.select.pageUp");
    m.insert("selectPageDown", "tui.select.pageDown");
    m.insert("selectConfirm", "tui.select.confirm");
    m.insert("selectCancel", "tui.select.cancel");
    // App
    m.insert("interrupt", "app.interrupt");
    m.insert("clear", "app.clear");
    m.insert("exit", "app.exit");
    m.insert("suspend", "app.suspend");
    m.insert("cycleThinkingLevel", "app.thinking.cycle");
    m.insert("cycleModelForward", "app.model.cycleForward");
    m.insert("cycleModelBackward", "app.model.cycleBackward");
    m.insert("selectModel", "app.model.select");
    m.insert("expandTools", "app.tools.expand");
    m.insert("toggleThinking", "app.thinking.toggle");
    m.insert("toggleSessionNamedFilter", "app.session.toggleNamedFilter");
    m.insert("externalEditor", "app.editor.external");
    m.insert("followUp", "app.message.followUp");
    m.insert("dequeue", "app.message.dequeue");
    m.insert("pasteImage", "app.clipboard.pasteImage");
    m.insert("newSession", "app.session.new");
    m.insert("tree", "app.session.tree");
    m.insert("fork", "app.session.fork");
    m.insert("resume", "app.session.resume");
    m.insert("treeFoldOrUp", "app.tree.foldOrUp");
    m.insert("treeUnfoldOrDown", "app.tree.unfoldOrDown");
    m.insert("toggleSessionPath", "app.session.togglePath");
    m.insert("toggleSessionSort", "app.session.toggleSort");
    m.insert("renameSession", "app.session.rename");
    m.insert("deleteSession", "app.session.delete");
    m.insert("deleteSessionNoninvasive", "app.session.deleteNoninvasive");
    m
}

/// Migrate a raw keybindings config map: rename legacy keys to new namespaced ones.
///
/// If the new name already exists in the map, the old name is dropped (new wins).
///
/// Returns `(migrated_map, was_any_key_renamed)`.
///
/// Mirrors `migrateKeybindingNames()` from TypeScript.
pub fn migrate_keybinding_names(
    raw: &serde_json::Map<String, serde_json::Value>,
) -> (serde_json::Map<String, serde_json::Value>, bool) {
    let migrations = keybinding_name_migrations();
    let mut result = serde_json::Map::new();
    let mut migrated = false;

    for (key, value) in raw {
        let new_key = migrations
            .get(key.as_str())
            .copied()
            .unwrap_or(key.as_str());
        if new_key != key.as_str() {
            migrated = true;
            // If the new name already exists in the source, skip this old entry
            if raw.contains_key(new_key) {
                continue;
            }
        }
        result.insert(new_key.to_string(), value.clone());
    }

    (result, migrated)
}

/// Migrate the `keybindings.json` file in `agent_dir` to use namespaced key names.
///
/// Returns `true` if any migration was performed and the file was rewritten.
///
/// Mirrors `migrateKeybindingsConfigFile()` from TypeScript.
pub fn migrate_keybindings_config_file(agent_dir: &Path) -> bool {
    let config_path = agent_dir.join("keybindings.json");

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let raw: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let obj = match raw.as_object() {
        Some(o) => o,
        None => return false,
    };

    let (migrated_map, was_migrated) = migrate_keybinding_names(obj);
    if !was_migrated {
        return false;
    }

    if let Ok(json) = serde_json::to_string_pretty(&serde_json::Value::Object(migrated_map)) {
        let _ = std::fs::write(&config_path, format!("{json}\n"));
    }

    true
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ─── Helpers ────────────────────────────────────────────────────────────

    fn make_manager_with_files(
        global_json: Option<&str>,
        project_json: Option<&str>,
    ) -> (SettingsManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agent");
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(project_dir.join(CONFIG_DIR_NAME)).unwrap();

        if let Some(json) = global_json {
            fs::write(agent_dir.join("settings.json"), json).unwrap();
        }
        if let Some(json) = project_json {
            fs::write(
                project_dir.join(CONFIG_DIR_NAME).join("settings.json"),
                json,
            )
            .unwrap();
        }

        let mgr = SettingsManager::create(&project_dir, &agent_dir);
        (mgr, tmp)
    }

    fn agent_settings_path(tmp: &TempDir) -> PathBuf {
        tmp.path().join("agent").join("settings.json")
    }

    fn project_settings_path(tmp: &TempDir) -> PathBuf {
        tmp.path()
            .join("project")
            .join(CONFIG_DIR_NAME)
            .join("settings.json")
    }

    fn read_json(path: &Path) -> serde_json::Value {
        let content = fs::read_to_string(path).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    // ─── Basic loading ───────────────────────────────────────────────────────

    #[test]
    fn default_when_no_files() {
        let (mgr, _tmp) = make_manager_with_files(None, None);
        assert!(mgr.get_default_provider().is_none());
    }

    #[test]
    fn global_settings_loaded() {
        let (mgr, _tmp) = make_manager_with_files(Some(r#"{"defaultProvider":"anthropic"}"#), None);
        assert_eq!(mgr.get_default_provider(), Some("anthropic"));
    }

    #[test]
    fn project_overrides_global() {
        let (mgr, _tmp) = make_manager_with_files(
            Some(r#"{"defaultProvider":"openai","defaultModel":"gpt-4o"}"#),
            Some(r#"{"defaultProvider":"anthropic"}"#),
        );
        assert_eq!(mgr.get_default_provider(), Some("anthropic"));
        assert_eq!(mgr.get_default_model(), Some("gpt-4o"));
    }

    #[test]
    fn drain_errors_empty_on_valid_files() {
        let (mut mgr, _tmp) = make_manager_with_files(Some(r#"{}"#), None);
        assert!(mgr.drain_errors().is_empty());
    }

    #[test]
    fn drain_errors_on_invalid_json() {
        let (mut mgr, _tmp) = make_manager_with_files(Some("not json!"), None);
        let errors = mgr.drain_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].scope, SettingsScope::Global);
    }

    #[test]
    fn drain_errors_both_invalid() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agent");
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(project_dir.join(CONFIG_DIR_NAME)).unwrap();
        fs::write(agent_dir.join("settings.json"), "{ invalid global json").unwrap();
        fs::write(
            project_dir.join(CONFIG_DIR_NAME).join("settings.json"),
            "{ invalid project json",
        )
        .unwrap();
        let mut mgr = SettingsManager::create(&project_dir, &agent_dir);
        let errors = mgr.drain_errors();
        assert_eq!(errors.len(), 2);
        let scopes: HashSet<_> = errors.iter().map(|e| e.scope).collect();
        assert!(scopes.contains(&SettingsScope::Global));
        assert!(scopes.contains(&SettingsScope::Project));
        assert!(mgr.drain_errors().is_empty());
    }

    // ─── Defaults ────────────────────────────────────────────────────────────

    #[test]
    fn get_quiet_startup_defaults_false() {
        let (mgr, _tmp) = make_manager_with_files(None, None);
        assert!(!mgr.get_quiet_startup());
    }

    #[test]
    fn get_image_auto_resize_defaults_true() {
        let (mgr, _tmp) = make_manager_with_files(None, None);
        assert!(mgr.get_image_auto_resize());
    }

    // ─── Reload ──────────────────────────────────────────────────────────────

    #[test]
    fn reload_global_settings_from_disk() {
        let (mut mgr, tmp) = make_manager_with_files(
            Some(r#"{"theme":"dark","extensions":["/before.ts"]}"#),
            None,
        );
        fs::write(
            agent_settings_path(&tmp),
            r#"{"theme":"light","extensions":["/after.ts"],"defaultModel":"claude-sonnet"}"#,
        )
        .unwrap();
        mgr.reload();
        assert_eq!(mgr.get_theme(), Some("light"));
        assert_eq!(mgr.get_extension_paths(), vec!["/after.ts"]);
        assert_eq!(mgr.get_default_model(), Some("claude-sonnet"));
    }

    #[test]
    fn reload_keeps_previous_settings_on_invalid_file() {
        let (mut mgr, tmp) = make_manager_with_files(Some(r#"{"theme":"dark"}"#), None);
        fs::write(agent_settings_path(&tmp), "{ invalid json").unwrap();
        mgr.reload();
        // Previous global settings preserved.
        assert_eq!(mgr.get_theme(), Some("dark"));
    }

    // ─── Error tracking ──────────────────────────────────────────────────────

    #[test]
    fn collect_and_clear_load_errors() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agent");
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(project_dir.join(CONFIG_DIR_NAME)).unwrap();
        fs::write(agent_dir.join("settings.json"), "{ invalid global json").unwrap();
        fs::write(
            project_dir.join(CONFIG_DIR_NAME).join("settings.json"),
            "{ invalid project json",
        )
        .unwrap();

        let mut mgr = SettingsManager::create(&project_dir, &agent_dir);
        let errors = mgr.drain_errors();
        assert_eq!(errors.len(), 2);
        let scopes: Vec<_> = errors.iter().map(|e| format!("{}", e.scope)).collect();
        assert!(scopes.contains(&"global".to_string()));
        assert!(scopes.contains(&"project".to_string()));
        assert!(mgr.drain_errors().is_empty());
    }

    // ─── Project dir creation ─────────────────────────────────────────────────

    #[test]
    fn no_project_dir_created_when_only_reading() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agent");
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(agent_dir.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();

        let mgr = SettingsManager::create(&project_dir, &agent_dir);
        assert!(!project_dir.join(CONFIG_DIR_NAME).exists());
        assert_eq!(mgr.get_theme(), Some("dark"));
    }

    #[test]
    fn project_dir_created_when_writing_project_settings() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agent");
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(agent_dir.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();

        let mut mgr = SettingsManager::create(&project_dir, &agent_dir);
        assert!(!project_dir.join(CONFIG_DIR_NAME).exists());

        mgr.set_project_packages(vec![PackageSource::Simple("npm:test-pkg".to_string())]);

        assert!(project_dir.join(CONFIG_DIR_NAME).exists());
        assert!(
            project_dir
                .join(CONFIG_DIR_NAME)
                .join("settings.json")
                .exists()
        );
    }

    // ─── shellCommandPrefix ───────────────────────────────────────────────────

    #[test]
    fn load_shell_command_prefix() {
        let (mgr, _tmp) = make_manager_with_files(
            Some(r#"{"shellCommandPrefix":"shopt -s expand_aliases"}"#),
            None,
        );
        assert_eq!(
            mgr.get_shell_command_prefix(),
            Some("shopt -s expand_aliases")
        );
    }

    #[test]
    fn shell_command_prefix_undefined_when_not_set() {
        let (mgr, _tmp) = make_manager_with_files(Some(r#"{"theme":"dark"}"#), None);
        assert!(mgr.get_shell_command_prefix().is_none());
    }

    #[test]
    fn shell_command_prefix_preserved_when_saving_unrelated_settings() {
        let (mut mgr, tmp) = make_manager_with_files(
            Some(r#"{"shellCommandPrefix":"shopt -s expand_aliases"}"#),
            None,
        );
        mgr.set_theme("light");

        let saved = read_json(&agent_settings_path(&tmp));
        assert_eq!(saved["shellCommandPrefix"], "shopt -s expand_aliases");
        assert_eq!(saved["theme"], "light");
    }

    // ─── sessionDir ──────────────────────────────────────────────────────────

    #[test]
    fn session_dir_undefined_when_not_set() {
        let (mgr, _tmp) = make_manager_with_files(Some(r#"{"theme":"dark"}"#), None);
        assert!(mgr.get_session_dir().is_none());
    }

    #[test]
    fn session_dir_returns_global_value() {
        let (mgr, _tmp) = make_manager_with_files(Some(r#"{"sessionDir":"/tmp/sessions"}"#), None);
        assert_eq!(mgr.get_session_dir(), Some("/tmp/sessions"));
    }

    #[test]
    fn session_dir_project_overrides_global() {
        let (mgr, _tmp) = make_manager_with_files(
            Some(r#"{"sessionDir":"/global/sessions"}"#),
            Some(r#"{"sessionDir":"./sessions"}"#),
        );
        assert_eq!(mgr.get_session_dir(), Some("./sessions"));
    }

    // ─── packages ────────────────────────────────────────────────────────────

    #[test]
    fn keep_local_only_extensions_in_extensions_array() {
        let (mgr, _tmp) = make_manager_with_files(
            Some(r#"{"extensions":["/local/ext.ts","./relative/ext.ts"]}"#),
            None,
        );
        assert!(mgr.get_packages().is_empty());
        assert_eq!(
            mgr.get_extension_paths(),
            vec!["/local/ext.ts", "./relative/ext.ts"]
        );
    }

    #[test]
    fn handle_packages_with_filtering_objects() {
        let json = r#"{"packages":["npm:simple-pkg",{"source":"npm:shitty-extensions","extensions":["extensions/oracle.ts"],"skills":[]}]}"#;
        let (mgr, _tmp) = make_manager_with_files(Some(json), None);
        let packages = mgr.get_packages();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].source(), "npm:simple-pkg");
        assert_eq!(packages[1].source(), "npm:shitty-extensions");
    }

    // ─── External edit preservation (bug tests) ───────────────────────────────

    #[test]
    fn preserve_enabled_models_when_changing_thinking_level() {
        let settings_path_json = r#"{"theme":"dark","defaultModel":"claude-sonnet"}"#;
        let (mut mgr, tmp) = make_manager_with_files(Some(settings_path_json), None);
        let path = agent_settings_path(&tmp);

        // Simulate external edit adding enabledModels
        let mut current: serde_json::Value = read_json(&path);
        current["enabledModels"] = serde_json::json!(["claude-opus-4-5", "gpt-5.2-codex"]);
        fs::write(&path, serde_json::to_string(&current).unwrap()).unwrap();

        mgr.set_default_thinking_level("high");

        let saved = read_json(&path);
        assert_eq!(
            saved["enabledModels"],
            serde_json::json!(["claude-opus-4-5", "gpt-5.2-codex"])
        );
        assert_eq!(saved["defaultThinkingLevel"], "high");
        assert_eq!(saved["theme"], "dark");
    }

    #[test]
    fn preserve_custom_settings_when_changing_theme() {
        let (mut mgr, tmp) =
            make_manager_with_files(Some(r#"{"defaultModel":"claude-sonnet"}"#), None);
        let path = agent_settings_path(&tmp);

        let mut current: serde_json::Value = read_json(&path);
        current["shellPath"] = serde_json::json!("/bin/zsh");
        current["extensions"] = serde_json::json!(["/path/to/extension.ts"]);
        fs::write(&path, serde_json::to_string(&current).unwrap()).unwrap();

        mgr.set_theme("light");

        let saved = read_json(&path);
        assert_eq!(saved["shellPath"], "/bin/zsh");
        assert_eq!(
            saved["extensions"],
            serde_json::json!(["/path/to/extension.ts"])
        );
        assert_eq!(saved["theme"], "light");
    }

    #[test]
    fn in_memory_changes_override_file_changes_for_same_key() {
        let (mut mgr, tmp) = make_manager_with_files(Some(r#"{"theme":"dark"}"#), None);
        let path = agent_settings_path(&tmp);

        let mut current: serde_json::Value = read_json(&path);
        current["defaultThinkingLevel"] = serde_json::json!("low");
        fs::write(&path, serde_json::to_string(&current).unwrap()).unwrap();

        mgr.set_default_thinking_level("high");

        let saved = read_json(&path);
        assert_eq!(saved["defaultThinkingLevel"], "high");
    }

    #[test]
    fn preserve_packages_array_when_changing_unrelated_setting() {
        let (mut mgr, tmp) = make_manager_with_files(
            Some(r#"{"theme":"dark","packages":["npm:pi-mcp-adapter"]}"#),
            None,
        );
        assert_eq!(mgr.get_packages().len(), 1);

        let path = agent_settings_path(&tmp);
        let mut current: serde_json::Value = read_json(&path);
        current["packages"] = serde_json::json!([]);
        fs::write(&path, serde_json::to_string_pretty(&current).unwrap()).unwrap();

        mgr.set_theme("light");

        let saved = read_json(&path);
        assert_eq!(saved["packages"], serde_json::json!([]));
        assert_eq!(saved["theme"], "light");
    }

    #[test]
    fn preserve_extensions_array_when_changing_unrelated_setting() {
        let (mut mgr, tmp) = make_manager_with_files(
            Some(r#"{"theme":"dark","extensions":["/old/extension.ts"]}"#),
            None,
        );

        let path = agent_settings_path(&tmp);
        let mut current: serde_json::Value = read_json(&path);
        current["extensions"] = serde_json::json!(["/new/extension.ts"]);
        fs::write(&path, serde_json::to_string_pretty(&current).unwrap()).unwrap();

        mgr.set_default_thinking_level("high");

        let saved = read_json(&path);
        assert_eq!(
            saved["extensions"],
            serde_json::json!(["/new/extension.ts"])
        );
    }

    #[test]
    fn preserve_external_project_settings_changes() {
        let proj_json = r#"{"extensions":["./old-extension.ts"],"prompts":["./old-prompt.md"]}"#;
        let (mut mgr, tmp) = make_manager_with_files(None, Some(proj_json));
        let proj_path = project_settings_path(&tmp);

        let mut current: serde_json::Value = read_json(&proj_path);
        current["prompts"] = serde_json::json!(["./new-prompt.md"]);
        fs::write(&proj_path, serde_json::to_string_pretty(&current).unwrap()).unwrap();

        mgr.set_project_extension_paths(vec!["./updated-extension.ts".to_string()]);

        let saved = read_json(&proj_path);
        assert_eq!(saved["prompts"], serde_json::json!(["./new-prompt.md"]));
        assert_eq!(
            saved["extensions"],
            serde_json::json!(["./updated-extension.ts"])
        );
    }

    #[test]
    fn in_memory_project_changes_override_external_for_same_field() {
        let proj_json = r#"{"extensions":["./initial-extension.ts"]}"#;
        let (mut mgr, tmp) = make_manager_with_files(None, Some(proj_json));
        let proj_path = project_settings_path(&tmp);

        let mut current: serde_json::Value = read_json(&proj_path);
        current["extensions"] = serde_json::json!(["./external-extension.ts"]);
        fs::write(&proj_path, serde_json::to_string_pretty(&current).unwrap()).unwrap();

        mgr.set_project_extension_paths(vec!["./in-memory-extension.ts".to_string()]);

        let saved = read_json(&proj_path);
        assert_eq!(
            saved["extensions"],
            serde_json::json!(["./in-memory-extension.ts"])
        );
    }

    // ─── Migration ───────────────────────────────────────────────────────────

    #[test]
    fn migrate_queue_mode_to_steering_mode() {
        let (mgr, _tmp) = make_manager_with_files(Some(r#"{"queueMode":"all"}"#), None);
        assert_eq!(mgr.get_steering_mode(), "all");
    }

    #[test]
    fn migrate_websockets_true_to_transport_websocket() {
        let (mgr, _tmp) = make_manager_with_files(Some(r#"{"websockets":true}"#), None);
        assert_eq!(mgr.get_transport(), "websocket");
    }

    #[test]
    fn migrate_websockets_false_to_transport_sse() {
        let (mgr, _tmp) = make_manager_with_files(Some(r#"{"websockets":false}"#), None);
        assert_eq!(mgr.get_transport(), "sse");
    }

    // ─── In-memory storage ───────────────────────────────────────────────────

    #[test]
    fn in_memory_manager_works() {
        let mut settings = Settings::default();
        settings.theme = Some("dark".to_string());
        let mut mgr = SettingsManager::in_memory(settings);
        assert_eq!(mgr.get_theme(), Some("dark"));
        mgr.set_theme("light");
        assert_eq!(mgr.get_theme(), Some("light"));
    }

    // ─── Keybindings migration (from keybindings-migration.test.ts) ──────────

    /// "rewrites old key names to namespaced ids"
    #[test]
    fn keybindings_migrate_old_names_to_namespaced() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"cursorUp": ["up", "ctrl+p"], "expandTools": "ctrl+x"}"#;
        fs::write(dir.path().join("keybindings.json"), content).unwrap();

        let changed = migrate_keybindings_config_file(dir.path());
        assert!(changed, "expected migration to report changes");

        let migrated_str = fs::read_to_string(dir.path().join("keybindings.json")).unwrap();
        let migrated: serde_json::Value = serde_json::from_str(&migrated_str).unwrap();

        assert_eq!(
            migrated["tui.editor.cursorUp"],
            serde_json::json!(["up", "ctrl+p"]),
            "cursorUp should be migrated to tui.editor.cursorUp"
        );
        assert_eq!(
            migrated["app.tools.expand"],
            serde_json::json!("ctrl+x"),
            "expandTools should be migrated to app.tools.expand"
        );
        assert!(
            migrated.get("cursorUp").is_none(),
            "old key cursorUp should be removed"
        );
        assert!(
            migrated.get("expandTools").is_none(),
            "old key expandTools should be removed"
        );
    }

    /// "keeps the namespaced value when old and new names both exist"
    #[test]
    fn keybindings_new_name_wins_when_both_exist() {
        let dir = TempDir::new().unwrap();
        // Both old name ("expandTools") and new name ("app.tools.expand") present.
        // The new name value ("ctrl+y") should win.
        let content = r#"{"expandTools": "ctrl+x", "app.tools.expand": "ctrl+y"}"#;
        fs::write(dir.path().join("keybindings.json"), content).unwrap();

        let changed = migrate_keybindings_config_file(dir.path());
        assert!(changed, "expected migration to report changes");

        let migrated_str = fs::read_to_string(dir.path().join("keybindings.json")).unwrap();
        let migrated: serde_json::Value = serde_json::from_str(&migrated_str).unwrap();

        assert_eq!(
            migrated["app.tools.expand"],
            serde_json::json!("ctrl+y"),
            "new name value should be kept"
        );
        assert!(
            migrated.get("expandTools").is_none(),
            "old key expandTools should be removed"
        );
    }

    /// No migration should occur when there are no old keys.
    #[test]
    fn keybindings_no_migration_when_already_namespaced() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"app.tools.expand": "ctrl+x"}"#;
        fs::write(dir.path().join("keybindings.json"), content).unwrap();

        let changed = migrate_keybindings_config_file(dir.path());
        assert!(!changed, "no old keys → no migration needed");
    }
}

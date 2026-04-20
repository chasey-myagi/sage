/// Global keybinding registry and manager.
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use crate::keys::matches_key;

// =============================================================================
// Types
// =============================================================================

pub type Keybinding = String;
pub use crate::keys::KeyId;
pub type KeybindingsConfig = HashMap<String, Vec<KeyId>>;

#[derive(Debug, Clone)]
pub struct KeybindingDefinition {
    pub default_keys: Vec<KeyId>,
    pub description: Option<String>,
}

pub type KeybindingDefinitions = HashMap<String, KeybindingDefinition>;

#[derive(Debug, Clone)]
pub struct KeybindingConflict {
    pub key: KeyId,
    pub keybindings: Vec<String>,
}

// =============================================================================
// Default TUI keybindings
// =============================================================================

pub fn default_tui_keybindings() -> KeybindingDefinitions {
    let mut m = HashMap::new();

    let add = |m: &mut KeybindingDefinitions, id: &str, keys: Vec<&str>, desc: &str| {
        m.insert(
            id.to_string(),
            KeybindingDefinition {
                default_keys: keys.into_iter().map(KeyId::from).collect(),
                description: Some(desc.to_string()),
            },
        );
    };

    add(&mut m, "tui.editor.cursorUp", vec!["up"], "Move cursor up");
    add(
        &mut m,
        "tui.editor.cursorDown",
        vec!["down"],
        "Move cursor down",
    );
    add(
        &mut m,
        "tui.editor.cursorLeft",
        vec!["left", "ctrl+b"],
        "Move cursor left",
    );
    add(
        &mut m,
        "tui.editor.cursorRight",
        vec!["right", "ctrl+f"],
        "Move cursor right",
    );
    add(
        &mut m,
        "tui.editor.cursorWordLeft",
        vec!["alt+left", "ctrl+left", "alt+b"],
        "Move cursor word left",
    );
    add(
        &mut m,
        "tui.editor.cursorWordRight",
        vec!["alt+right", "ctrl+right", "alt+f"],
        "Move cursor word right",
    );
    add(
        &mut m,
        "tui.editor.cursorLineStart",
        vec!["home", "ctrl+a"],
        "Move to line start",
    );
    add(
        &mut m,
        "tui.editor.cursorLineEnd",
        vec!["end", "ctrl+e"],
        "Move to line end",
    );
    add(
        &mut m,
        "tui.editor.jumpForward",
        vec!["ctrl+]"],
        "Jump forward to character",
    );
    add(
        &mut m,
        "tui.editor.jumpBackward",
        vec!["ctrl+alt+]"],
        "Jump backward to character",
    );
    add(&mut m, "tui.editor.pageUp", vec!["pageUp"], "Page up");
    add(&mut m, "tui.editor.pageDown", vec!["pageDown"], "Page down");
    add(
        &mut m,
        "tui.editor.deleteCharBackward",
        vec!["backspace"],
        "Delete character backward",
    );
    add(
        &mut m,
        "tui.editor.deleteCharForward",
        vec!["delete", "ctrl+d"],
        "Delete character forward",
    );
    add(
        &mut m,
        "tui.editor.deleteWordBackward",
        vec!["ctrl+w", "alt+backspace"],
        "Delete word backward",
    );
    add(
        &mut m,
        "tui.editor.deleteWordForward",
        vec!["alt+d", "alt+delete"],
        "Delete word forward",
    );
    add(
        &mut m,
        "tui.editor.deleteToLineStart",
        vec!["ctrl+u"],
        "Delete to line start",
    );
    add(
        &mut m,
        "tui.editor.deleteToLineEnd",
        vec!["ctrl+k"],
        "Delete to line end",
    );
    add(&mut m, "tui.editor.yank", vec!["ctrl+y"], "Yank");
    add(&mut m, "tui.editor.yankPop", vec!["alt+y"], "Yank pop");
    add(&mut m, "tui.editor.undo", vec!["ctrl+-"], "Undo");
    add(
        &mut m,
        "tui.input.newLine",
        vec!["shift+enter"],
        "Insert newline",
    );
    add(&mut m, "tui.input.submit", vec!["enter"], "Submit input");
    add(&mut m, "tui.input.tab", vec!["tab"], "Tab / autocomplete");
    add(&mut m, "tui.input.copy", vec!["ctrl+c"], "Copy selection");
    add(&mut m, "tui.select.up", vec!["up"], "Move selection up");
    add(
        &mut m,
        "tui.select.down",
        vec!["down"],
        "Move selection down",
    );
    add(
        &mut m,
        "tui.select.pageUp",
        vec!["pageUp"],
        "Selection page up",
    );
    add(
        &mut m,
        "tui.select.pageDown",
        vec!["pageDown"],
        "Selection page down",
    );
    add(
        &mut m,
        "tui.select.confirm",
        vec!["enter"],
        "Confirm selection",
    );
    add(
        &mut m,
        "tui.select.cancel",
        vec!["escape", "ctrl+c"],
        "Cancel selection",
    );

    m
}

// =============================================================================
// KeybindingsManager
// =============================================================================

pub struct KeybindingsManager {
    definitions: KeybindingDefinitions,
    user_bindings: KeybindingsConfig,
    keys_by_id: HashMap<String, Vec<KeyId>>,
    conflicts: Vec<KeybindingConflict>,
}

impl KeybindingsManager {
    pub fn new(definitions: KeybindingDefinitions, user_bindings: KeybindingsConfig) -> Self {
        let mut mgr = Self {
            definitions,
            user_bindings,
            keys_by_id: HashMap::new(),
            conflicts: Vec::new(),
        };
        mgr.rebuild();
        mgr
    }

    fn rebuild(&mut self) {
        self.keys_by_id.clear();
        self.conflicts.clear();

        // Find user-defined conflicts
        let mut user_claims: HashMap<KeyId, HashSet<String>> = HashMap::new();
        for (keybinding, keys) in &self.user_bindings {
            if !self.definitions.contains_key(keybinding) {
                continue;
            }
            for key in keys {
                user_claims
                    .entry(key.clone())
                    .or_default()
                    .insert(keybinding.clone());
            }
        }
        for (key, keybindings) in &user_claims {
            if keybindings.len() > 1 {
                self.conflicts.push(KeybindingConflict {
                    key: key.clone(),
                    keybindings: keybindings.iter().cloned().collect(),
                });
            }
        }

        // Build keys_by_id
        for (id, definition) in &self.definitions {
            let keys = if let Some(user_keys) = self.user_bindings.get(id) {
                deduplicate(user_keys.clone())
            } else {
                deduplicate(definition.default_keys.clone())
            };
            self.keys_by_id.insert(id.clone(), keys);
        }
    }

    /// Check if input data matches a keybinding.
    pub fn matches(&self, data: &str, keybinding: &str) -> bool {
        let keys = self
            .keys_by_id
            .get(keybinding)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        for key in keys {
            if matches_key(data, key.as_str()) {
                return true;
            }
        }
        false
    }

    /// Get all key ids for a keybinding.
    pub fn get_keys(&self, keybinding: &str) -> Vec<KeyId> {
        self.keys_by_id.get(keybinding).cloned().unwrap_or_default()
    }

    /// Get the definition for a keybinding.
    pub fn get_definition(&self, keybinding: &str) -> Option<&KeybindingDefinition> {
        self.definitions.get(keybinding)
    }

    /// Get all conflicts.
    pub fn get_conflicts(&self) -> Vec<KeybindingConflict> {
        self.conflicts.clone()
    }

    /// Update user bindings.
    pub fn set_user_bindings(&mut self, user_bindings: KeybindingsConfig) {
        self.user_bindings = user_bindings;
        self.rebuild();
    }

    /// Get current user bindings.
    pub fn get_user_bindings(&self) -> KeybindingsConfig {
        self.user_bindings.clone()
    }

    /// Get the resolved bindings (user overrides or defaults).
    pub fn get_resolved_bindings(&self) -> KeybindingsConfig {
        let mut resolved = HashMap::new();
        for id in self.definitions.keys() {
            let keys = self.keys_by_id.get(id).cloned().unwrap_or_default();
            resolved.insert(id.clone(), keys);
        }
        resolved
    }
}

fn deduplicate(keys: Vec<KeyId>) -> Vec<KeyId> {
    let mut seen = HashSet::new();
    keys.into_iter()
        .filter(|k| seen.insert(k.clone()))
        .collect()
}

// =============================================================================
// Global keybindings
// =============================================================================

static GLOBAL_KEYBINDINGS: OnceLock<Mutex<Option<KeybindingsManager>>> = OnceLock::new();

fn global_keybindings_cell() -> &'static Mutex<Option<KeybindingsManager>> {
    GLOBAL_KEYBINDINGS.get_or_init(|| Mutex::new(None))
}

/// Set the global keybindings manager.
pub fn set_keybindings(manager: KeybindingsManager) {
    let mut lock = global_keybindings_cell().lock().unwrap();
    *lock = Some(manager);
}

/// Get the global keybindings manager, creating a default one if not set.
pub fn get_keybindings() -> GlobalKeybindingsGuard {
    // Ensure global manager exists, then return a guard that proxies calls.
    let mut lock = global_keybindings_cell().lock().unwrap();
    if lock.is_none() {
        *lock = Some(KeybindingsManager::new(
            default_tui_keybindings(),
            HashMap::new(),
        ));
    }
    drop(lock);
    GlobalKeybindingsGuard
}

/// Helper to check a keybinding without holding a lock reference.
pub fn check_keybinding(data: &str, keybinding: &str) -> bool {
    let mut lock = global_keybindings_cell().lock().unwrap();
    if lock.is_none() {
        *lock = Some(KeybindingsManager::new(
            default_tui_keybindings(),
            HashMap::new(),
        ));
    }
    lock.as_ref().unwrap().matches(data, keybinding)
}

/// Wrapper that provides deref-like access via get_keybindings().
/// Actually implements Deref by cloning state once - but since KeybindingsManager
/// is complex to clone, we provide a simpler approach with direct methods.
pub struct GlobalKeybindingsGuard;

impl GlobalKeybindingsGuard {
    pub fn matches(&self, data: &str, keybinding: &str) -> bool {
        check_keybinding(data, keybinding)
    }

    pub fn get_keys(&self, keybinding: &str) -> Vec<KeyId> {
        let lock = global_keybindings_cell().lock().unwrap();
        lock.as_ref()
            .map(|m| m.get_keys(keybinding))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mgr() -> KeybindingsManager {
        KeybindingsManager::new(default_tui_keybindings(), HashMap::new())
    }

    #[test]
    fn test_matches_enter() {
        let mgr = make_mgr();
        assert!(mgr.matches("\r", "tui.input.submit"));
    }

    #[test]
    fn test_matches_escape() {
        let mgr = make_mgr();
        assert!(mgr.matches("\x1b", "tui.select.cancel"));
    }

    #[test]
    fn test_matches_ctrl_c() {
        let mgr = make_mgr();
        assert!(mgr.matches("\x03", "tui.select.cancel"));
        assert!(mgr.matches("\x03", "tui.input.copy"));
    }

    #[test]
    fn test_matches_backspace() {
        let mgr = make_mgr();
        assert!(mgr.matches("\x7f", "tui.editor.deleteCharBackward"));
    }

    #[test]
    fn test_matches_arrows() {
        let mgr = make_mgr();
        assert!(mgr.matches("\x1b[A", "tui.editor.cursorUp"));
        assert!(mgr.matches("\x1b[B", "tui.editor.cursorDown"));
    }

    #[test]
    fn test_get_keys() {
        let mgr = make_mgr();
        let keys = mgr.get_keys("tui.editor.cursorLeft");
        assert!(keys.iter().any(|k| k == "left"));
        assert!(keys.iter().any(|k| k == "ctrl+b"));
    }

    #[test]
    fn test_user_override() {
        let mut user = HashMap::new();
        user.insert("tui.input.submit".to_string(), vec![KeyId::from("ctrl+s")]);
        let mgr = KeybindingsManager::new(default_tui_keybindings(), user);
        // Now ctrl+s should match submit
        assert!(mgr.matches("\x13", "tui.input.submit"));
        // enter should no longer match
        assert!(!mgr.matches("\r", "tui.input.submit"));
    }

    #[test]
    fn test_get_definition() {
        let mgr = make_mgr();
        let def = mgr.get_definition("tui.input.submit");
        assert!(def.is_some());
        assert_eq!(def.unwrap().description.as_deref(), Some("Submit input"));
    }

    #[test]
    fn test_no_conflicts_by_default() {
        let mgr = make_mgr();
        // User bindings are empty, so no conflicts
        assert!(mgr.get_conflicts().is_empty());
    }

    #[test]
    fn test_user_conflict_detection() {
        let mut user = HashMap::new();
        user.insert(
            "tui.editor.cursorUp".to_string(),
            vec![KeyId::from("ctrl+x")],
        );
        user.insert(
            "tui.editor.cursorDown".to_string(),
            vec![KeyId::from("ctrl+x")],
        );
        let mgr = KeybindingsManager::new(default_tui_keybindings(), user);
        let conflicts = mgr.get_conflicts();
        assert!(!conflicts.is_empty());
    }

    // ==========================================================================
    // Tests from keybindings.test.ts
    // ==========================================================================

    #[test]
    fn test_does_not_evict_select_confirm_when_input_submit_rebound() {
        // "does not evict selector confirm when input submit is rebound"
        // When user rebinds tui.input.submit to ["enter", "ctrl+enter"],
        // tui.select.confirm should still keep its default ["enter"] key.
        let mut user = HashMap::new();
        user.insert(
            "tui.input.submit".to_string(),
            vec![KeyId::from("enter"), KeyId::from("ctrl+enter")],
        );
        let mgr = KeybindingsManager::new(default_tui_keybindings(), user);

        let submit_keys = mgr.get_keys("tui.input.submit");
        assert!(
            submit_keys.iter().any(|k| k == "enter"),
            "submit should have 'enter'"
        );
        assert!(
            submit_keys.iter().any(|k| k == "ctrl+enter"),
            "submit should have 'ctrl+enter'"
        );

        let confirm_keys = mgr.get_keys("tui.select.confirm");
        assert!(
            confirm_keys.iter().any(|k| k == "enter"),
            "select.confirm should still have 'enter'"
        );
    }

    #[test]
    fn test_does_not_evict_cursor_bindings_when_another_action_reuses_same_key() {
        // "does not evict cursor bindings when another action reuses the same key"
        // When user rebinds tui.select.up to ["up", "ctrl+p"],
        // tui.editor.cursorUp should still keep its default ["up"] key.
        let mut user = HashMap::new();
        user.insert(
            "tui.select.up".to_string(),
            vec![KeyId::from("up"), KeyId::from("ctrl+p")],
        );
        let mgr = KeybindingsManager::new(default_tui_keybindings(), user);

        let select_up_keys = mgr.get_keys("tui.select.up");
        assert!(
            select_up_keys.iter().any(|k| k == "up"),
            "select.up should have 'up'"
        );
        assert!(
            select_up_keys.iter().any(|k| k == "ctrl+p"),
            "select.up should have 'ctrl+p'"
        );

        let cursor_up_keys = mgr.get_keys("tui.editor.cursorUp");
        assert!(
            cursor_up_keys.iter().any(|k| k == "up"),
            "editor.cursorUp should still have 'up'"
        );
    }

    #[test]
    fn test_reports_direct_user_binding_conflicts_without_evicting_defaults() {
        // "still reports direct user binding conflicts without evicting defaults"
        // When user explicitly assigns ctrl+x to BOTH tui.input.submit and tui.select.confirm,
        // those should be reported as conflicts, but other defaults (like tui.editor.cursorLeft)
        // should remain unaffected.
        let mut user = HashMap::new();
        user.insert("tui.input.submit".to_string(), vec![KeyId::from("ctrl+x")]);
        user.insert(
            "tui.select.confirm".to_string(),
            vec![KeyId::from("ctrl+x")],
        );
        let mgr = KeybindingsManager::new(default_tui_keybindings(), user);

        let conflicts = mgr.get_conflicts();
        assert!(!conflicts.is_empty(), "should detect conflict for ctrl+x");
        // Find the ctrl+x conflict
        let ctrl_x_conflict = conflicts.iter().find(|c| c.key == "ctrl+x");
        assert!(ctrl_x_conflict.is_some(), "should have ctrl+x conflict");
        let conflict = ctrl_x_conflict.unwrap();
        assert!(
            conflict
                .keybindings
                .contains(&"tui.input.submit".to_string()),
            "conflict should include tui.input.submit"
        );
        assert!(
            conflict
                .keybindings
                .contains(&"tui.select.confirm".to_string()),
            "conflict should include tui.select.confirm"
        );

        // tui.editor.cursorLeft should still have its defaults (["left", "ctrl+b"])
        let cursor_left_keys = mgr.get_keys("tui.editor.cursorLeft");
        assert!(
            cursor_left_keys.iter().any(|k| k == "left"),
            "cursorLeft should still have 'left'"
        );
        assert!(
            cursor_left_keys.iter().any(|k| k == "ctrl+b"),
            "cursorLeft should still have 'ctrl+b'"
        );
    }

    #[test]
    fn test_keybindings_manager_get_keys_returns_defaults() {
        let mgr = make_mgr();
        // Test several default keybindings from TUI_KEYBINDINGS
        let delete_word = mgr.get_keys("tui.editor.deleteWordBackward");
        assert!(
            delete_word.iter().any(|k| k == "ctrl+w"),
            "deleteWordBackward should have ctrl+w"
        );
        assert!(
            delete_word.iter().any(|k| k == "alt+backspace"),
            "deleteWordBackward should have alt+backspace"
        );

        let delete_to_end = mgr.get_keys("tui.editor.deleteToLineEnd");
        assert!(
            delete_to_end.iter().any(|k| k == "ctrl+k"),
            "deleteToLineEnd should have ctrl+k"
        );

        let yank = mgr.get_keys("tui.editor.yank");
        assert!(
            yank.iter().any(|k| k == "ctrl+y"),
            "yank should have ctrl+y"
        );

        let yank_pop = mgr.get_keys("tui.editor.yankPop");
        assert!(
            yank_pop.iter().any(|k| k == "alt+y"),
            "yankPop should have alt+y"
        );

        let undo = mgr.get_keys("tui.editor.undo");
        assert!(
            undo.iter().any(|k| k == "ctrl+-"),
            "undo should have ctrl+-"
        );
    }

    #[test]
    fn test_keybindings_manager_unknown_keybinding_returns_empty() {
        let mgr = make_mgr();
        let keys = mgr.get_keys("tui.unknown.action");
        assert!(
            keys.is_empty(),
            "unknown keybinding should return empty vec"
        );
    }

    #[test]
    fn test_keybindings_manager_set_user_bindings_updates() {
        let mut mgr = make_mgr();
        let keys_before = mgr.get_keys("tui.input.submit");
        assert!(keys_before.iter().any(|k| k == "enter"));

        let mut user = HashMap::new();
        user.insert("tui.input.submit".to_string(), vec![KeyId::from("ctrl+s")]);
        mgr.set_user_bindings(user);

        let keys_after = mgr.get_keys("tui.input.submit");
        assert!(
            keys_after.iter().any(|k| k == "ctrl+s"),
            "should use new user binding"
        );
        assert!(
            !keys_after.iter().any(|k| k == "enter"),
            "should not have old default"
        );
    }

    #[test]
    fn test_keybindings_manager_deduplicates_keys() {
        let mut user = HashMap::new();
        user.insert(
            "tui.input.submit".to_string(),
            vec![KeyId::from("enter"), KeyId::from("enter")],
        );
        let mgr = KeybindingsManager::new(default_tui_keybindings(), user);
        let keys = mgr.get_keys("tui.input.submit");
        // Should be deduplicated
        assert_eq!(keys.len(), 1, "duplicate keys should be removed");
        assert_eq!(keys[0], "enter");
    }
}

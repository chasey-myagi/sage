//! Extension runner — executes extensions and manages their lifecycle.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/extensions/runner.ts`.

use std::collections::HashMap;

use serde_json::Value;

use super::types::{
    BeforeAgentStartEventResult, ExtensionContextSnapshot, ExtensionError, ExtensionFlag,
    ExtensionRuntimeState, FlagValue, InputEventResult, InputSource, RegisteredCommand,
    RegisteredTool, ResolvedCommand, ResourcesDiscoverResult, SessionBeforeCompactResult,
    SessionBeforeForkResult, SessionBeforeSwitchResult, SessionBeforeTreeResult,
};

// ============================================================================
// Reserved keybindings (cannot be overridden by extensions)
// ============================================================================

const RESERVED_KEYBINDINGS: &[&str] = &[
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
    "app.editor.external",
    "app.message.followUp",
    "tui.input.submit",
    "tui.select.confirm",
    "tui.select.cancel",
    "tui.input.copy",
    "tui.editor.deleteToLineEnd",
];

// ============================================================================
// Handler function type
// ============================================================================

/// A boxed handler that takes (event_json, context_snapshot) and returns an optional result JSON.
pub type HandlerFn = Box<
    dyn Fn(&Value, &ExtensionContextSnapshot) -> Option<Value> + Send + Sync,
>;

// ============================================================================
// Extension Shortcut (simplified — no TUI KeyId in Rust)
// ============================================================================

#[derive(Debug, Clone)]
pub struct ExtensionShortcut {
    pub shortcut: String,
    pub description: Option<String>,
    pub extension_path: String,
}

// ============================================================================
// Error listener
// ============================================================================

pub type ExtensionErrorListener = Box<dyn Fn(&ExtensionError) + Send + Sync>;

// ============================================================================
// Extension data (per-loaded extension)
// ============================================================================

pub struct ExtensionData {
    pub path: String,
    pub resolved_path: String,
    pub handlers: HashMap<String, Vec<HandlerFn>>,
    pub tools: HashMap<String, RegisteredTool>,
    pub commands: HashMap<String, RegisteredCommand>,
    pub flags: HashMap<String, ExtensionFlag>,
    pub shortcuts: HashMap<String, ExtensionShortcut>,
}

impl ExtensionData {
    pub fn new(path: impl Into<String>, resolved_path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            resolved_path: resolved_path.into(),
            handlers: HashMap::new(),
            tools: HashMap::new(),
            commands: HashMap::new(),
            flags: HashMap::new(),
            shortcuts: HashMap::new(),
        }
    }
}

// ============================================================================
// ExtensionRunner
// ============================================================================

/// Manages loaded extensions and dispatches events to their handlers.
pub struct ExtensionRunner {
    extensions: Vec<ExtensionData>,
    runtime: ExtensionRuntimeState,
    cwd: String,
    has_ui: bool,
    error_listeners: Vec<ExtensionErrorListener>,

    // Context action callbacks
    is_idle_fn: Box<dyn Fn() -> bool + Send + Sync>,
    get_context_usage_fn: Box<dyn Fn() -> Option<Value> + Send + Sync>,
    get_system_prompt_fn: Box<dyn Fn() -> String + Send + Sync>,

    // Command/shortcut diagnostics
    shortcut_diagnostics: Vec<String>,
    command_diagnostics: Vec<String>,
}

impl ExtensionRunner {
    /// Create a new `ExtensionRunner` with no extensions loaded.
    pub fn new(cwd: impl Into<String>, runtime: ExtensionRuntimeState) -> Self {
        Self {
            extensions: Vec::new(),
            runtime,
            cwd: cwd.into(),
            has_ui: false,
            error_listeners: Vec::new(),
            is_idle_fn: Box::new(|| true),
            get_context_usage_fn: Box::new(|| None),
            get_system_prompt_fn: Box::new(String::new),
            shortcut_diagnostics: Vec::new(),
            command_diagnostics: Vec::new(),
        }
    }

    /// Build a context snapshot for handler invocations.
    pub fn create_context_snapshot(&self) -> ExtensionContextSnapshot {
        ExtensionContextSnapshot {
            cwd: self.cwd.clone(),
            has_ui: self.has_ui,
        }
    }

    /// Add an extension to the runner.
    pub fn add_extension(&mut self, ext: ExtensionData) {
        self.extensions.push(ext);
    }

    /// Register an error listener. Returns an ID that can be used to remove it.
    pub fn on_error<F>(&mut self, listener: F)
    where
        F: Fn(&ExtensionError) + Send + Sync + 'static,
    {
        self.error_listeners.push(Box::new(listener));
    }

    /// Emit an error to all registered listeners.
    pub fn emit_error(&self, error: &ExtensionError) {
        for listener in &self.error_listeners {
            listener(error);
        }
    }

    /// Whether any extension has registered a handler for the given event type.
    pub fn has_handlers(&self, event_type: &str) -> bool {
        self.extensions.iter().any(|ext| {
            ext.handlers
                .get(event_type)
                .map_or(false, |h| !h.is_empty())
        })
    }

    /// Get all registered tools (first registration per name wins).
    pub fn get_all_registered_tools(&self) -> Vec<&RegisteredTool> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for ext in &self.extensions {
            for (name, tool) in &ext.tools {
                if seen.insert(name.clone()) {
                    result.push(tool);
                }
            }
        }
        result
    }

    /// Get a tool definition by name.
    pub fn get_tool_definition(&self, tool_name: &str) -> Option<&RegisteredTool> {
        for ext in &self.extensions {
            if let Some(tool) = ext.tools.get(tool_name) {
                return Some(tool);
            }
        }
        None
    }

    /// Get all flags across all extensions (first registration wins).
    pub fn get_flags(&self) -> HashMap<String, &ExtensionFlag> {
        let mut all_flags = HashMap::new();
        for ext in &self.extensions {
            for (name, flag) in &ext.flags {
                all_flags.entry(name.clone()).or_insert(flag);
            }
        }
        all_flags
    }

    /// Set a flag value in the shared runtime.
    pub fn set_flag_value(&mut self, name: &str, value: FlagValue) {
        self.runtime.flag_values.insert(name.to_string(), value);
    }

    /// Get all flag values.
    pub fn get_flag_values(&self) -> &HashMap<String, FlagValue> {
        &self.runtime.flag_values
    }

    /// Resolve registered commands across all extensions, assigning unique invocation names.
    pub fn get_registered_commands(&mut self) -> Vec<ResolvedCommand> {
        self.command_diagnostics.clear();

        let mut commands: Vec<&RegisteredCommand> = Vec::new();
        let mut counts: HashMap<String, usize> = HashMap::new();

        for ext in &self.extensions {
            for cmd in ext.commands.values() {
                *counts.entry(cmd.name.clone()).or_insert(0) += 1;
                commands.push(cmd);
            }
        }

        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut taken = std::collections::HashSet::new();

        commands
            .into_iter()
            .map(|cmd| {
                let occurrence = {
                    let e = seen.entry(cmd.name.clone()).or_insert(0);
                    *e += 1;
                    *e
                };

                let base_invocation = if *counts.get(&cmd.name).unwrap_or(&1) > 1 {
                    format!("{}:{}", cmd.name, occurrence)
                } else {
                    cmd.name.clone()
                };

                let mut invocation_name = base_invocation.clone();
                if taken.contains(&invocation_name) {
                    let mut suffix = occurrence;
                    loop {
                        suffix += 1;
                        invocation_name = format!("{}:{}", cmd.name, suffix);
                        if !taken.contains(&invocation_name) {
                            break;
                        }
                    }
                }

                taken.insert(invocation_name.clone());
                ResolvedCommand {
                    name: cmd.name.clone(),
                    invocation_name,
                    source_info: cmd.source_info.clone(),
                    description: cmd.description.clone(),
                }
            })
            .collect()
    }

    /// Find a command by its invocation name.
    pub fn get_command(&mut self, name: &str) -> Option<ResolvedCommand> {
        self.get_registered_commands()
            .into_iter()
            .find(|c| c.invocation_name == name)
    }

    /// Get all extension paths.
    pub fn get_extension_paths(&self) -> Vec<&str> {
        self.extensions.iter().map(|e| e.path.as_str()).collect()
    }

    // =========================================================================
    // Generic event emit
    // =========================================================================

    /// Emit a generic event to all extensions that have a handler for it.
    /// Returns the last non-None handler result (for `session_before_*` events).
    pub fn emit(&self, event_type: &str, event: &Value) -> Option<Value> {
        let ctx = self.create_context_snapshot();
        let mut result: Option<Value> = None;

        for ext in &self.extensions {
            let handlers = match ext.handlers.get(event_type) {
                Some(h) if !h.is_empty() => h,
                _ => continue,
            };

            for handler in handlers {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handler(event, &ctx)
                })) {
                    Ok(Some(r)) => {
                        // For session_before_* events check cancel
                        if let Some(cancel) = r.get("cancel").and_then(|v| v.as_bool()) {
                            result = Some(r.clone());
                            if cancel {
                                return result;
                            }
                        } else {
                            result = Some(r);
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        self.emit_error(&super::types::ExtensionError {
                            extension_path: ext.path.clone(),
                            event: event_type.to_string(),
                            error: "Handler panicked".to_string(),
                            stack: None,
                        });
                    }
                }
            }
        }

        result
    }

    // =========================================================================
    // Specialized emitters
    // =========================================================================

    /// Emit the `session_shutdown` event.
    pub fn emit_session_shutdown(&self) {
        let event = serde_json::json!({ "type": "session_shutdown" });
        self.emit("session_shutdown", &event);
    }

    /// Emit `before_agent_start` event.
    pub fn emit_before_agent_start(
        &self,
        prompt: &str,
        system_prompt: &str,
    ) -> BeforeAgentStartEventResult {
        let event = serde_json::json!({
            "type": "before_agent_start",
            "prompt": prompt,
            "systemPrompt": system_prompt,
        });
        let ctx = self.create_context_snapshot();
        let mut result = BeforeAgentStartEventResult::default();

        for ext in &self.extensions {
            let handlers = match ext.handlers.get("before_agent_start") {
                Some(h) if !h.is_empty() => h,
                _ => continue,
            };
            for handler in handlers {
                if let Some(r) = handler(&event, &ctx) {
                    if let Some(sp) = r.get("systemPrompt").and_then(|v| v.as_str()) {
                        result.system_prompt = Some(sp.to_string());
                    }
                }
            }
        }

        result
    }

    /// Emit `resources_discover` event and collect paths from all handlers.
    pub fn emit_resources_discover(
        &self,
        cwd: &str,
        reason: &str,
    ) -> ResourcesDiscoverResult {
        let event = serde_json::json!({
            "type": "resources_discover",
            "cwd": cwd,
            "reason": reason,
        });
        let ctx = self.create_context_snapshot();
        let mut result = ResourcesDiscoverResult::default();

        for ext in &self.extensions {
            let handlers = match ext.handlers.get("resources_discover") {
                Some(h) if !h.is_empty() => h,
                _ => continue,
            };
            for handler in handlers {
                if let Some(r) = handler(&event, &ctx) {
                    if let Some(arr) = r.get("skillPaths").and_then(|v| v.as_array()) {
                        for p in arr {
                            if let Some(s) = p.as_str() {
                                result.skill_paths.push(s.to_string());
                            }
                        }
                    }
                    if let Some(arr) = r.get("promptPaths").and_then(|v| v.as_array()) {
                        for p in arr {
                            if let Some(s) = p.as_str() {
                                result.prompt_paths.push(s.to_string());
                            }
                        }
                    }
                    if let Some(arr) = r.get("themePaths").and_then(|v| v.as_array()) {
                        for p in arr {
                            if let Some(s) = p.as_str() {
                                result.theme_paths.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }

        result
    }

    /// Emit `input` event. Returns the final text after all transforms.
    pub fn emit_input(
        &self,
        text: &str,
        source: InputSource,
    ) -> InputEventResult {
        let ctx = self.create_context_snapshot();
        let mut current_text = text.to_string();

        for ext in &self.extensions {
            let handlers = match ext.handlers.get("input") {
                Some(h) if !h.is_empty() => h,
                _ => continue,
            };
            for handler in handlers {
                let event = serde_json::json!({
                    "type": "input",
                    "text": current_text,
                    "source": format!("{:?}", source).to_lowercase(),
                });
                if let Some(r) = handler(&event, &ctx) {
                    if let Some(action) = r.get("action").and_then(|v| v.as_str()) {
                        match action {
                            "handled" => return InputEventResult::Handled,
                            "transform" => {
                                if let Some(t) = r.get("text").and_then(|v| v.as_str()) {
                                    current_text = t.to_string();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if current_text != text {
            InputEventResult::Transform { text: current_text }
        } else {
            InputEventResult::Continue
        }
    }

    // =========================================================================
    // Session before events (can cancel)
    // =========================================================================

    pub fn emit_session_before_switch(&self, reason: &str, target: Option<&str>) -> SessionBeforeSwitchResult {
        let event = serde_json::json!({
            "type": "session_before_switch",
            "reason": reason,
            "targetSessionFile": target,
        });
        let r = self.emit("session_before_switch", &event);
        if let Some(v) = r {
            return SessionBeforeSwitchResult {
                cancel: v.get("cancel").and_then(|c| c.as_bool()).unwrap_or(false),
            };
        }
        SessionBeforeSwitchResult::default()
    }

    pub fn emit_session_before_fork(&self, entry_id: &str) -> SessionBeforeForkResult {
        let event = serde_json::json!({
            "type": "session_before_fork",
            "entryId": entry_id,
        });
        let r = self.emit("session_before_fork", &event);
        if let Some(v) = r {
            return SessionBeforeForkResult {
                cancel: v.get("cancel").and_then(|c| c.as_bool()).unwrap_or(false),
                skip_conversation_restore: v
                    .get("skipConversationRestore")
                    .and_then(|c| c.as_bool())
                    .unwrap_or(false),
            };
        }
        SessionBeforeForkResult::default()
    }

    pub fn emit_session_before_compact(&self, custom_instructions: Option<&str>) -> SessionBeforeCompactResult {
        let event = serde_json::json!({
            "type": "session_before_compact",
            "customInstructions": custom_instructions,
        });
        let r = self.emit("session_before_compact", &event);
        if let Some(v) = r {
            return SessionBeforeCompactResult {
                cancel: v.get("cancel").and_then(|c| c.as_bool()).unwrap_or(false),
            };
        }
        SessionBeforeCompactResult::default()
    }

    pub fn emit_session_before_tree(
        &self,
        target_id: &str,
        custom_instructions: Option<&str>,
    ) -> SessionBeforeTreeResult {
        let event = serde_json::json!({
            "type": "session_before_tree",
            "targetId": target_id,
            "customInstructions": custom_instructions,
        });
        let r = self.emit("session_before_tree", &event);
        if let Some(v) = r {
            return SessionBeforeTreeResult {
                cancel: v.get("cancel").and_then(|c| c.as_bool()).unwrap_or(false),
                custom_instructions: v
                    .get("customInstructions")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string()),
            };
        }
        SessionBeforeTreeResult::default()
    }
}

// ============================================================================
// Helper: emit session_shutdown if runner exists and has handlers
// ============================================================================

/// Helper to emit `session_shutdown` event if the runner has registered handlers.
pub fn emit_session_shutdown_event(runner: Option<&ExtensionRunner>) -> bool {
    if let Some(r) = runner {
        if r.has_handlers("session_shutdown") {
            r.emit_session_shutdown();
            return true;
        }
    }
    false
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::extensions::{
        loader::create_extension_runtime,
        types::{InputEventResult, InputSource},
    };

    fn make_runner() -> ExtensionRunner {
        ExtensionRunner::new("/tmp", create_extension_runtime())
    }

    // ── extensions-input-event.test.ts ────────────────────────────────────────

    #[test]
    fn test_emit_input_returns_continue_with_no_handlers() {
        let runner = make_runner();
        let result = runner.emit_input("hello", InputSource::Interactive);
        assert!(matches!(result, InputEventResult::Continue));
    }

    #[test]
    fn test_has_handlers_false_when_no_extensions() {
        let runner = make_runner();
        assert!(!runner.has_handlers("input"));
    }

    #[test]
    fn test_has_handlers_true_after_adding_handler() {
        let mut runner = make_runner();
        let mut ext = ExtensionData::new("ext1", "/ext1");
        ext.handlers
            .entry("input".to_string())
            .or_default()
            .push(Box::new(|_event, _ctx| None));
        runner.add_extension(ext);
        assert!(runner.has_handlers("input"));
    }

    #[test]
    fn test_emit_input_transform_changes_text() {
        let mut runner = make_runner();
        let mut ext = ExtensionData::new("ext1", "/ext1");
        ext.handlers
            .entry("input".to_string())
            .or_default()
            .push(Box::new(|event, _ctx| {
                let original = event.get("text")?.as_str()?.to_string();
                Some(serde_json::json!({
                    "action": "transform",
                    "text": format!("T:{}", original)
                }))
            }));
        runner.add_extension(ext);

        let result = runner.emit_input("hi", InputSource::Interactive);
        assert!(matches!(result, InputEventResult::Transform { text } if text == "T:hi"));
    }

    #[test]
    fn test_emit_input_handled_short_circuits() {
        let mut runner = make_runner();

        // First extension returns "handled"
        let mut ext1 = ExtensionData::new("ext1", "/ext1");
        ext1.handlers
            .entry("input".to_string())
            .or_default()
            .push(Box::new(|_event, _ctx| {
                Some(serde_json::json!({ "action": "handled" }))
            }));
        runner.add_extension(ext1);

        // Second extension would transform
        let mut ext2 = ExtensionData::new("ext2", "/ext2");
        ext2.handlers
            .entry("input".to_string())
            .or_default()
            .push(Box::new(|_event, _ctx| {
                // Should never be called
                Some(serde_json::json!({ "action": "transform", "text": "SHOULD_NOT_APPEAR" }))
            }));
        runner.add_extension(ext2);

        let result = runner.emit_input("hello", InputSource::Interactive);
        assert!(matches!(result, InputEventResult::Handled));
    }

    #[test]
    fn test_emit_input_chains_transforms_across_handlers() {
        let mut runner = make_runner();

        let mut ext1 = ExtensionData::new("ext1", "/ext1");
        ext1.handlers
            .entry("input".to_string())
            .or_default()
            .push(Box::new(|event, _ctx| {
                let t = event.get("text")?.as_str()?.to_string();
                Some(serde_json::json!({ "action": "transform", "text": format!("{}[1]", t) }))
            }));
        runner.add_extension(ext1);

        let mut ext2 = ExtensionData::new("ext2", "/ext2");
        ext2.handlers
            .entry("input".to_string())
            .or_default()
            .push(Box::new(|event, _ctx| {
                let t = event.get("text")?.as_str()?.to_string();
                Some(serde_json::json!({ "action": "transform", "text": format!("{}[2]", t) }))
            }));
        runner.add_extension(ext2);

        let result = runner.emit_input("X", InputSource::Interactive);
        // Input event in runner processes each extension sequentially but the second
        // handler receives a fresh event from the updated current_text only if it's in
        // a different extension. Both handlers in ext1 would see each other's changes.
        // The result should be X[1][2]
        if let InputEventResult::Transform { text } = result {
            assert!(text.contains("[1]"), "should contain [1]");
            assert!(text.contains("[2]"), "should contain [2]");
        } else {
            panic!("Expected Transform result");
        }
    }

    #[test]
    fn test_emit_input_continue_when_handler_returns_none() {
        let mut runner = make_runner();
        let mut ext = ExtensionData::new("ext1", "/ext1");
        ext.handlers
            .entry("input".to_string())
            .or_default()
            .push(Box::new(|_event, _ctx| None));
        runner.add_extension(ext);

        let result = runner.emit_input("x", InputSource::Interactive);
        assert!(matches!(result, InputEventResult::Continue));
    }

    #[test]
    fn test_on_error_receives_errors() {
        let mut runner = make_runner();
        let errors: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let errors_clone = errors.clone();

        runner.on_error(move |e| {
            errors_clone.lock().unwrap().push(e.error.clone());
        });

        runner.emit_error(&super::super::types::ExtensionError {
            extension_path: "test".to_string(),
            event: "input".to_string(),
            error: "boom".to_string(),
            stack: None,
        });

        assert_eq!(errors.lock().unwrap().as_slice(), &["boom"]);
    }

    #[test]
    fn test_emit_input_explicit_continue_action() {
        let mut runner = make_runner();
        let mut ext = ExtensionData::new("ext1", "/ext1");
        ext.handlers
            .entry("input".to_string())
            .or_default()
            .push(Box::new(|_event, _ctx| {
                Some(serde_json::json!({ "action": "continue" }))
            }));
        runner.add_extension(ext);

        let result = runner.emit_input("x", InputSource::Interactive);
        // "continue" action with no text change → Continue
        assert!(matches!(result, InputEventResult::Continue));
    }
}

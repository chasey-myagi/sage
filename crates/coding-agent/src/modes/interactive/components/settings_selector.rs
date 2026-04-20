//! Settings selector component.
//!
//! Translated from `components/settings-selector.ts`.
//!
//! Main settings UI with toggle/submenu items for all configurable options.

use crate::modes::interactive::components::thinking_selector::ThinkingLevel;

// ============================================================================
// Transport
// ============================================================================

/// Network transport preference for providers that support multiple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Sse,
    WebSocket,
    Auto,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Sse => "sse",
            Transport::WebSocket => "websocket",
            Transport::Auto => "auto",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "sse" => Some(Transport::Sse),
            "websocket" => Some(Transport::WebSocket),
            "auto" => Some(Transport::Auto),
            _ => None,
        }
    }
}

// ============================================================================
// DoubleEscapeAction
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleEscapeAction {
    Fork,
    Tree,
    None,
}

impl DoubleEscapeAction {
    pub fn as_str(self) -> &'static str {
        match self {
            DoubleEscapeAction::Fork => "fork",
            DoubleEscapeAction::Tree => "tree",
            DoubleEscapeAction::None => "none",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "fork" => Some(DoubleEscapeAction::Fork),
            "tree" => Some(DoubleEscapeAction::Tree),
            "none" => Some(DoubleEscapeAction::None),
            _ => None,
        }
    }
}

// ============================================================================
// TreeFilterMode
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeFilterMode {
    Default,
    NoTools,
    UserOnly,
    LabeledOnly,
    All,
}

impl TreeFilterMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TreeFilterMode::Default => "default",
            TreeFilterMode::NoTools => "no-tools",
            TreeFilterMode::UserOnly => "user-only",
            TreeFilterMode::LabeledOnly => "labeled-only",
            TreeFilterMode::All => "all",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "default" => Some(TreeFilterMode::Default),
            "no-tools" => Some(TreeFilterMode::NoTools),
            "user-only" => Some(TreeFilterMode::UserOnly),
            "labeled-only" => Some(TreeFilterMode::LabeledOnly),
            "all" => Some(TreeFilterMode::All),
            _ => None,
        }
    }
}

// ============================================================================
// SteeringMode / FollowUpMode
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    All,
    OneAtATime,
}

impl DeliveryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryMode::All => "all",
            DeliveryMode::OneAtATime => "one-at-a-time",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "all" => Some(DeliveryMode::All),
            "one-at-a-time" => Some(DeliveryMode::OneAtATime),
            _ => None,
        }
    }
}

// ============================================================================
// SettingsConfig
// ============================================================================

/// Snapshot of all current settings, passed to the component constructor.
///
/// Mirrors `SettingsConfig` from TypeScript.
#[derive(Debug, Clone)]
pub struct SettingsConfig {
    pub auto_compact: bool,
    pub show_images: bool,
    pub auto_resize_images: bool,
    pub block_images: bool,
    pub enable_skill_commands: bool,
    pub steering_mode: DeliveryMode,
    pub follow_up_mode: DeliveryMode,
    pub transport: Transport,
    pub thinking_level: ThinkingLevel,
    pub available_thinking_levels: Vec<ThinkingLevel>,
    pub current_theme: String,
    pub available_themes: Vec<String>,
    pub hide_thinking_block: bool,
    pub collapse_changelog: bool,
    pub double_escape_action: DoubleEscapeAction,
    pub tree_filter_mode: TreeFilterMode,
    pub show_hardware_cursor: bool,
    pub editor_padding_x: u32,
    pub autocomplete_max_visible: u32,
    pub quiet_startup: bool,
    pub clear_on_shrink: bool,
}

impl Default for SettingsConfig {
    fn default() -> Self {
        Self {
            auto_compact: true,
            show_images: true,
            auto_resize_images: true,
            block_images: false,
            enable_skill_commands: true,
            steering_mode: DeliveryMode::OneAtATime,
            follow_up_mode: DeliveryMode::OneAtATime,
            transport: Transport::Auto,
            thinking_level: ThinkingLevel::Off,
            available_thinking_levels: ThinkingLevel::ALL.to_vec(),
            current_theme: "default".into(),
            available_themes: vec!["default".into()],
            hide_thinking_block: false,
            collapse_changelog: false,
            double_escape_action: DoubleEscapeAction::Tree,
            tree_filter_mode: TreeFilterMode::Default,
            show_hardware_cursor: false,
            editor_padding_x: 0,
            autocomplete_max_visible: 10,
            quiet_startup: false,
            clear_on_shrink: false,
        }
    }
}

// ============================================================================
// SettingsCallbacks
// ============================================================================

/// Callbacks from the settings selector.
pub struct SettingsCallbacks {
    pub on_auto_compact_change: Box<dyn Fn(bool) + Send>,
    pub on_show_images_change: Box<dyn Fn(bool) + Send>,
    pub on_auto_resize_images_change: Box<dyn Fn(bool) + Send>,
    pub on_block_images_change: Box<dyn Fn(bool) + Send>,
    pub on_enable_skill_commands_change: Box<dyn Fn(bool) + Send>,
    pub on_steering_mode_change: Box<dyn Fn(DeliveryMode) + Send>,
    pub on_follow_up_mode_change: Box<dyn Fn(DeliveryMode) + Send>,
    pub on_transport_change: Box<dyn Fn(Transport) + Send>,
    pub on_thinking_level_change: Box<dyn Fn(ThinkingLevel) + Send>,
    pub on_theme_change: Box<dyn Fn(String) + Send>,
    pub on_theme_preview: Option<Box<dyn Fn(String) + Send>>,
    pub on_hide_thinking_block_change: Box<dyn Fn(bool) + Send>,
    pub on_collapse_changelog_change: Box<dyn Fn(bool) + Send>,
    pub on_double_escape_action_change: Box<dyn Fn(DoubleEscapeAction) + Send>,
    pub on_tree_filter_mode_change: Box<dyn Fn(TreeFilterMode) + Send>,
    pub on_show_hardware_cursor_change: Box<dyn Fn(bool) + Send>,
    pub on_editor_padding_x_change: Box<dyn Fn(u32) + Send>,
    pub on_autocomplete_max_visible_change: Box<dyn Fn(u32) + Send>,
    pub on_quiet_startup_change: Box<dyn Fn(bool) + Send>,
    pub on_clear_on_shrink_change: Box<dyn Fn(bool) + Send>,
    pub on_cancel: Box<dyn Fn() + Send>,
}

// ============================================================================
// SettingItem
// ============================================================================

/// A single setting item in the list.
#[derive(Debug, Clone)]
pub struct SettingItem {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub current_value: String,
    pub values: Vec<String>,
}

// ============================================================================
// SettingsSelectorComponent
// ============================================================================

/// Main settings selector component.
pub struct SettingsSelectorComponent {
    pub items: Vec<SettingItem>,
    pub selected_index: usize,
    config: SettingsConfig,
    callbacks: SettingsCallbacks,
}

impl SettingsSelectorComponent {
    /// Create a new settings selector with the given config and callbacks.
    pub fn new(config: SettingsConfig, callbacks: SettingsCallbacks) -> Self {
        let items = Self::build_items(&config);
        Self {
            items,
            selected_index: 0,
            config,
            callbacks,
        }
    }

    fn build_items(cfg: &SettingsConfig) -> Vec<SettingItem> {
        vec![
            SettingItem {
                id: "autocompact",
                label: "Auto-compact",
                description: "Automatically compact context when it gets too large",
                current_value: if cfg.auto_compact { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "steering-mode",
                label: "Steering mode",
                description: "Enter while streaming queues steering messages.",
                current_value: cfg.steering_mode.as_str().into(),
                values: vec!["one-at-a-time".into(), "all".into()],
            },
            SettingItem {
                id: "follow-up-mode",
                label: "Follow-up mode",
                description: "Alt+Enter queues follow-up messages until agent stops.",
                current_value: cfg.follow_up_mode.as_str().into(),
                values: vec!["one-at-a-time".into(), "all".into()],
            },
            SettingItem {
                id: "transport",
                label: "Transport",
                description: "Preferred transport for providers that support multiple transports",
                current_value: cfg.transport.as_str().into(),
                values: vec!["sse".into(), "websocket".into(), "auto".into()],
            },
            SettingItem {
                id: "hide-thinking",
                label: "Hide thinking",
                description: "Hide thinking blocks in assistant responses",
                current_value: if cfg.hide_thinking_block { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "collapse-changelog",
                label: "Collapse changelog",
                description: "Show condensed changelog after updates",
                current_value: if cfg.collapse_changelog { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "quiet-startup",
                label: "Quiet startup",
                description: "Disable verbose printing at startup",
                current_value: if cfg.quiet_startup { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "double-escape-action",
                label: "Double-escape action",
                description: "Action when pressing Escape twice with empty editor",
                current_value: cfg.double_escape_action.as_str().into(),
                values: vec!["tree".into(), "fork".into(), "none".into()],
            },
            SettingItem {
                id: "tree-filter-mode",
                label: "Tree filter mode",
                description: "Default filter when opening /tree",
                current_value: cfg.tree_filter_mode.as_str().into(),
                values: vec!["default".into(), "no-tools".into(), "user-only".into(), "labeled-only".into(), "all".into()],
            },
            SettingItem {
                id: "show-images",
                label: "Show images",
                description: "Render images inline in terminal",
                current_value: if cfg.show_images { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "auto-resize-images",
                label: "Auto-resize images",
                description: "Resize large images to 2000x2000 max for better model compatibility",
                current_value: if cfg.auto_resize_images { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "block-images",
                label: "Block images",
                description: "Prevent images from being sent to LLM providers",
                current_value: if cfg.block_images { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "skill-commands",
                label: "Skill commands",
                description: "Register skills as /skill:name commands",
                current_value: if cfg.enable_skill_commands { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "show-hardware-cursor",
                label: "Show hardware cursor",
                description: "Show the terminal cursor while still positioning it for IME support",
                current_value: if cfg.show_hardware_cursor { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "editor-padding",
                label: "Editor padding",
                description: "Horizontal padding for input editor (0-3)",
                current_value: cfg.editor_padding_x.to_string(),
                values: vec!["0".into(), "1".into(), "2".into(), "3".into()],
            },
            SettingItem {
                id: "autocomplete-max-visible",
                label: "Autocomplete max items",
                description: "Max visible items in autocomplete dropdown (3-20)",
                current_value: cfg.autocomplete_max_visible.to_string(),
                values: vec!["3".into(), "5".into(), "7".into(), "10".into(), "15".into(), "20".into()],
            },
            SettingItem {
                id: "clear-on-shrink",
                label: "Clear on shrink",
                description: "Clear empty rows when content shrinks (may cause flicker)",
                current_value: if cfg.clear_on_shrink { "true" } else { "false" }.into(),
                values: vec!["true".into(), "false".into()],
            },
        ]
    }

    /// Apply a setting change by ID and new value string.
    ///
    /// Mirrors the `switch` in the TypeScript `SettingsList` onChange handler.
    pub fn apply_change(&self, id: &str, new_value: &str) {
        match id {
            "autocompact" => (self.callbacks.on_auto_compact_change)(new_value == "true"),
            "show-images" => (self.callbacks.on_show_images_change)(new_value == "true"),
            "auto-resize-images" => (self.callbacks.on_auto_resize_images_change)(new_value == "true"),
            "block-images" => (self.callbacks.on_block_images_change)(new_value == "true"),
            "skill-commands" => (self.callbacks.on_enable_skill_commands_change)(new_value == "true"),
            "steering-mode" => {
                if let Some(mode) = DeliveryMode::from_str(new_value) {
                    (self.callbacks.on_steering_mode_change)(mode);
                }
            }
            "follow-up-mode" => {
                if let Some(mode) = DeliveryMode::from_str(new_value) {
                    (self.callbacks.on_follow_up_mode_change)(mode);
                }
            }
            "transport" => {
                if let Some(t) = Transport::from_str(new_value) {
                    (self.callbacks.on_transport_change)(t);
                }
            }
            "hide-thinking" => (self.callbacks.on_hide_thinking_block_change)(new_value == "true"),
            "collapse-changelog" => (self.callbacks.on_collapse_changelog_change)(new_value == "true"),
            "quiet-startup" => (self.callbacks.on_quiet_startup_change)(new_value == "true"),
            "double-escape-action" => {
                if let Some(action) = DoubleEscapeAction::from_str(new_value) {
                    (self.callbacks.on_double_escape_action_change)(action);
                }
            }
            "tree-filter-mode" => {
                if let Some(mode) = TreeFilterMode::from_str(new_value) {
                    (self.callbacks.on_tree_filter_mode_change)(mode);
                }
            }
            "show-hardware-cursor" => (self.callbacks.on_show_hardware_cursor_change)(new_value == "true"),
            "editor-padding" => {
                if let Ok(v) = new_value.parse::<u32>() {
                    (self.callbacks.on_editor_padding_x_change)(v);
                }
            }
            "autocomplete-max-visible" => {
                if let Ok(v) = new_value.parse::<u32>() {
                    (self.callbacks.on_autocomplete_max_visible_change)(v);
                }
            }
            "clear-on-shrink" => (self.callbacks.on_clear_on_shrink_change)(new_value == "true"),
            _ => {}
        }
    }

    /// Cancel and close the settings panel.
    pub fn cancel(&self) {
        (self.callbacks.on_cancel)();
    }

    /// Move selection up.
    pub fn select_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Move selection down.
    pub fn select_down(&mut self) {
        if !self.items.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.items.len() - 1);
        }
    }

    /// Toggle the current item to the next available value.
    pub fn cycle_value(&mut self) {
        let (item_id, new_value) = if let Some(item) = self.items.get_mut(self.selected_index) {
            if item.values.is_empty() {
                return;
            }
            let idx = item
                .values
                .iter()
                .position(|v| v == &item.current_value)
                .unwrap_or(0);
            let next = (idx + 1) % item.values.len();
            let new_value = item.values[next].clone();
            item.current_value = new_value.clone();
            let id = item.id;
            (id, new_value)
        } else {
            return;
        };
        self.apply_change(item_id, &new_value);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_noop_callbacks() -> SettingsCallbacks {
        SettingsCallbacks {
            on_auto_compact_change: Box::new(|_| {}),
            on_show_images_change: Box::new(|_| {}),
            on_auto_resize_images_change: Box::new(|_| {}),
            on_block_images_change: Box::new(|_| {}),
            on_enable_skill_commands_change: Box::new(|_| {}),
            on_steering_mode_change: Box::new(|_| {}),
            on_follow_up_mode_change: Box::new(|_| {}),
            on_transport_change: Box::new(|_| {}),
            on_thinking_level_change: Box::new(|_| {}),
            on_theme_change: Box::new(|_| {}),
            on_theme_preview: None,
            on_hide_thinking_block_change: Box::new(|_| {}),
            on_collapse_changelog_change: Box::new(|_| {}),
            on_double_escape_action_change: Box::new(|_| {}),
            on_tree_filter_mode_change: Box::new(|_| {}),
            on_show_hardware_cursor_change: Box::new(|_| {}),
            on_editor_padding_x_change: Box::new(|_| {}),
            on_autocomplete_max_visible_change: Box::new(|_| {}),
            on_quiet_startup_change: Box::new(|_| {}),
            on_clear_on_shrink_change: Box::new(|_| {}),
            on_cancel: Box::new(|| {}),
        }
    }

    #[test]
    fn items_are_created() {
        let comp = SettingsSelectorComponent::new(SettingsConfig::default(), make_noop_callbacks());
        assert!(!comp.items.is_empty());
        assert!(comp.items.iter().any(|i| i.id == "autocompact"));
        assert!(comp.items.iter().any(|i| i.id == "transport"));
    }

    #[test]
    fn apply_change_auto_compact() {
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        let mut callbacks = make_noop_callbacks();
        callbacks.on_auto_compact_change = Box::new(move |v| *called2.lock().unwrap() = v);
        let comp = SettingsSelectorComponent::new(SettingsConfig::default(), callbacks);
        comp.apply_change("autocompact", "true");
        assert!(*called.lock().unwrap());
    }

    #[test]
    fn apply_change_editor_padding() {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let cap2 = captured.clone();
        let mut callbacks = make_noop_callbacks();
        callbacks.on_editor_padding_x_change = Box::new(move |v| *cap2.lock().unwrap() = v);
        let comp = SettingsSelectorComponent::new(SettingsConfig::default(), callbacks);
        comp.apply_change("editor-padding", "2");
        assert_eq!(*captured.lock().unwrap(), 2);
    }

    #[test]
    fn transport_round_trip() {
        for (s, t) in &[
            ("sse", Transport::Sse),
            ("websocket", Transport::WebSocket),
            ("auto", Transport::Auto),
        ] {
            assert_eq!(Transport::from_str(s), Some(*t));
            assert_eq!(t.as_str(), *s);
        }
    }

    #[test]
    fn double_escape_action_round_trip() {
        for (s, a) in &[
            ("fork", DoubleEscapeAction::Fork),
            ("tree", DoubleEscapeAction::Tree),
            ("none", DoubleEscapeAction::None),
        ] {
            assert_eq!(DoubleEscapeAction::from_str(s), Some(*a));
            assert_eq!(a.as_str(), *s);
        }
    }

    #[test]
    fn tree_filter_mode_round_trip() {
        for (s, m) in &[
            ("default", TreeFilterMode::Default),
            ("no-tools", TreeFilterMode::NoTools),
            ("user-only", TreeFilterMode::UserOnly),
            ("labeled-only", TreeFilterMode::LabeledOnly),
            ("all", TreeFilterMode::All),
        ] {
            assert_eq!(TreeFilterMode::from_str(s), Some(*m));
            assert_eq!(m.as_str(), *s);
        }
    }

    #[test]
    fn cycle_value_advances_selection() {
        let mut comp = SettingsSelectorComponent::new(SettingsConfig::default(), make_noop_callbacks());
        // Find the autocompact item
        let idx = comp.items.iter().position(|i| i.id == "autocompact").unwrap();
        comp.selected_index = idx;
        let initial = comp.items[idx].current_value.clone();
        comp.cycle_value();
        let next = comp.items[idx].current_value.clone();
        assert_ne!(initial, next);
        // Cycle again should return to initial
        comp.cycle_value();
        assert_eq!(comp.items[idx].current_value, initial);
    }
}

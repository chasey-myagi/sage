//! Scoped-models selector component.
//!
//! Translated from `components/scoped-models-selector.ts`.
//!
//! Enables/disables models for Ctrl+P cycling. Changes are session-only until
//! explicitly persisted with Ctrl+S.

use std::collections::{HashMap, HashSet};

// ============================================================================
// EnabledIds
// ============================================================================

/// `null` in the TypeScript = all enabled (no filter).
/// `Some(ids)` = explicit ordered list.
#[derive(Debug, Clone)]
pub enum EnabledIds {
    /// All models enabled.
    All,
    /// Explicit ordered list of enabled model IDs.
    List(Vec<String>),
}

impl EnabledIds {
    pub fn is_enabled(&self, id: &str) -> bool {
        match self {
            EnabledIds::All => true,
            EnabledIds::List(ids) => ids.iter().any(|i| i == id),
        }
    }

    pub fn toggle(&self, id: &str) -> EnabledIds {
        match self {
            EnabledIds::All => EnabledIds::List(vec![id.to_string()]),
            EnabledIds::List(ids) => {
                let mut new_ids = ids.clone();
                if let Some(pos) = new_ids.iter().position(|i| i == id) {
                    new_ids.remove(pos);
                } else {
                    new_ids.push(id.to_string());
                }
                EnabledIds::List(new_ids)
            }
        }
    }

    pub fn enable_all(self, all_ids: &[String], target_ids: Option<&[String]>) -> EnabledIds {
        match self {
            EnabledIds::All => EnabledIds::All,
            EnabledIds::List(mut ids) => {
                let targets: &[String] = target_ids.unwrap_or(all_ids);
                for t in targets {
                    if !ids.contains(t) {
                        ids.push(t.clone());
                    }
                }
                if ids.len() == all_ids.len() {
                    EnabledIds::All
                } else {
                    EnabledIds::List(ids)
                }
            }
        }
    }

    pub fn clear_all(self, all_ids: &[String], target_ids: Option<&[String]>) -> EnabledIds {
        match self {
            EnabledIds::All => {
                if let Some(targets) = target_ids {
                    let target_set: HashSet<&str> = targets.iter().map(|s| s.as_str()).collect();
                    EnabledIds::List(
                        all_ids
                            .iter()
                            .filter(|id| !target_set.contains(id.as_str()))
                            .cloned()
                            .collect(),
                    )
                } else {
                    EnabledIds::List(vec![])
                }
            }
            EnabledIds::List(ids) => {
                let fallback_targets: HashSet<String> = ids.iter().cloned().collect();
                let targets: HashSet<String> = target_ids
                    .map(|t| t.iter().cloned().collect())
                    .unwrap_or(fallback_targets);
                EnabledIds::List(ids.into_iter().filter(|id| !targets.contains(id)).collect())
            }
        }
    }

    pub fn move_id(self, all_ids: &[String], id: &str, delta: i64) -> EnabledIds {
        let list = match &self {
            EnabledIds::All => all_ids.to_vec(),
            EnabledIds::List(ids) => ids.clone(),
        };
        let Some(index) = list.iter().position(|i| i == id) else {
            return self;
        };
        let new_index = index as i64 + delta;
        if new_index < 0 || new_index >= list.len() as i64 {
            return self;
        }
        let mut result = list;
        result.swap(index, new_index as usize);
        EnabledIds::List(result)
    }

    /// Return the IDs sorted: enabled first (in their order), then disabled.
    pub fn sorted_ids(&self, all_ids: &[String]) -> Vec<String> {
        match self {
            EnabledIds::All => all_ids.to_vec(),
            EnabledIds::List(enabled) => {
                let enabled_set: HashSet<&str> = enabled.iter().map(|s| s.as_str()).collect();
                let mut result = enabled.clone();
                for id in all_ids {
                    if !enabled_set.contains(id.as_str()) {
                        result.push(id.clone());
                    }
                }
                result
            }
        }
    }
}

// ============================================================================
// Types
// ============================================================================

/// A model entry in the scoped-models selector.
#[derive(Debug, Clone)]
pub struct ScopedModelItem {
    /// `"provider/id"` composite key.
    pub full_id: String,
    pub provider: String,
    pub id: String,
    pub name: Option<String>,
    pub enabled: bool,
}

/// Callbacks for model enable/disable operations.
pub struct ModelsCallbacks {
    pub on_model_toggle: Box<dyn Fn(&str, bool) + Send>,
    pub on_persist: Box<dyn Fn(Vec<String>) + Send>,
    pub on_enable_all: Box<dyn Fn(Vec<String>) + Send>,
    pub on_clear_all: Box<dyn Fn() + Send>,
    pub on_toggle_provider: Box<dyn Fn(&str, Vec<String>, bool) + Send>,
    pub on_cancel: Box<dyn Fn() + Send>,
}

// ============================================================================
// ScopedModelsSelectorComponent
// ============================================================================

/// Component for enabling/disabling models for Ctrl+P cycling.
pub struct ScopedModelsSelectorComponent {
    pub all_ids: Vec<String>,
    pub models_by_id: HashMap<String, ScopedModelItem>,
    pub enabled_ids: EnabledIds,
    pub filtered_items: Vec<ScopedModelItem>,
    pub selected_index: usize,
    pub search_query: String,
    pub is_dirty: bool,
    pub max_visible: usize,
    callbacks: ModelsCallbacks,
}

impl ScopedModelsSelectorComponent {
    /// Create a new scoped-models selector.
    pub fn new(
        all_models: Vec<ScopedModelItem>,
        enabled_model_ids: Option<Vec<String>>,
        callbacks: ModelsCallbacks,
    ) -> Self {
        let mut models_by_id = HashMap::new();
        let mut all_ids = Vec::new();
        for m in &all_models {
            models_by_id.insert(m.full_id.clone(), m.clone());
            all_ids.push(m.full_id.clone());
        }

        let enabled_ids = match enabled_model_ids {
            None => EnabledIds::All,
            Some(ids) => EnabledIds::List(ids),
        };

        let mut comp = Self {
            all_ids,
            models_by_id,
            enabled_ids,
            filtered_items: Vec::new(),
            selected_index: 0,
            search_query: String::new(),
            is_dirty: false,
            max_visible: 15,
            callbacks,
        };
        comp.refresh();
        comp
    }

    fn build_items(&self) -> Vec<ScopedModelItem> {
        self.enabled_ids
            .sorted_ids(&self.all_ids)
            .into_iter()
            .filter_map(|id| {
                let m = self.models_by_id.get(&id)?;
                Some(ScopedModelItem {
                    full_id: m.full_id.clone(),
                    provider: m.provider.clone(),
                    id: m.id.clone(),
                    name: m.name.clone(),
                    enabled: self.enabled_ids.is_enabled(&m.full_id),
                })
            })
            .collect()
    }

    pub fn refresh(&mut self) {
        let all_items = self.build_items();
        let query = self.search_query.to_lowercase();
        self.filtered_items = if query.is_empty() {
            all_items
        } else {
            all_items
                .into_iter()
                .filter(|item| {
                    item.id.to_lowercase().contains(&query)
                        || item.provider.to_lowercase().contains(&query)
                        || item
                            .name
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&query)
                })
                .collect()
        };
        let len = self.filtered_items.len();
        if self.selected_index >= len && len > 0 {
            self.selected_index = len - 1;
        }
    }

    /// Toggle the selected model on/off.
    pub fn toggle_selected(&mut self) {
        if let Some(item) = self.filtered_items.get(self.selected_index).cloned() {
            let was_all = matches!(self.enabled_ids, EnabledIds::All);
            self.enabled_ids = self.enabled_ids.clone().toggle(&item.full_id);
            self.is_dirty = true;
            if was_all {
                (self.callbacks.on_clear_all)();
            }
            let now_enabled = self.enabled_ids.is_enabled(&item.full_id);
            (self.callbacks.on_model_toggle)(&item.full_id, now_enabled);
            self.refresh();
        }
    }

    /// Enable all (filtered or all).
    pub fn enable_all(&mut self) {
        let target_ids: Option<Vec<String>> = if !self.search_query.is_empty() {
            Some(
                self.filtered_items
                    .iter()
                    .map(|i| i.full_id.clone())
                    .collect(),
            )
        } else {
            None
        };
        let all_ids = self.all_ids.clone();
        self.enabled_ids = self
            .enabled_ids
            .clone()
            .enable_all(&all_ids, target_ids.as_deref());
        self.is_dirty = true;
        let target = target_ids.unwrap_or_else(|| self.all_ids.clone());
        (self.callbacks.on_enable_all)(target);
        self.refresh();
    }

    /// Clear all (filtered or all).
    pub fn clear_all(&mut self) {
        let target_ids: Option<Vec<String>> = if !self.search_query.is_empty() {
            Some(
                self.filtered_items
                    .iter()
                    .map(|i| i.full_id.clone())
                    .collect(),
            )
        } else {
            None
        };
        let all_ids = self.all_ids.clone();
        self.enabled_ids = self
            .enabled_ids
            .clone()
            .clear_all(&all_ids, target_ids.as_deref());
        self.is_dirty = true;
        (self.callbacks.on_clear_all)();
        self.refresh();
    }

    /// Toggle all models for the provider of the selected item.
    pub fn toggle_provider(&mut self) {
        if let Some(item) = self.filtered_items.get(self.selected_index).cloned() {
            let provider = item.provider.clone();
            let provider_ids: Vec<String> = self
                .all_ids
                .iter()
                .filter(|id| {
                    self.models_by_id
                        .get(*id)
                        .map(|m| m.provider == provider)
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            let all_enabled = provider_ids
                .iter()
                .all(|id| self.enabled_ids.is_enabled(id));
            let all_ids = self.all_ids.clone();
            self.enabled_ids = if all_enabled {
                self.enabled_ids
                    .clone()
                    .clear_all(&all_ids, Some(&provider_ids))
            } else {
                self.enabled_ids
                    .clone()
                    .enable_all(&all_ids, Some(&provider_ids))
            };
            self.is_dirty = true;
            (self.callbacks.on_toggle_provider)(&provider, provider_ids, !all_enabled);
            self.refresh();
        }
    }

    /// Persist current selection to settings.
    pub fn persist(&mut self) {
        let ids = match &self.enabled_ids {
            EnabledIds::All => self.all_ids.clone(),
            EnabledIds::List(ids) => ids.clone(),
        };
        (self.callbacks.on_persist)(ids);
        self.is_dirty = false;
    }

    /// Cancel.
    pub fn cancel(&self) {
        (self.callbacks.on_cancel)();
    }

    /// Move up.
    pub fn select_up(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.filtered_items.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    /// Move down.
    pub fn select_down(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.filtered_items.len();
    }

    /// Reorder: move selected item up/down within enabled list.
    pub fn reorder(&mut self, delta: i64) {
        if let Some(item) = self.filtered_items.get(self.selected_index).cloned() {
            if self.enabled_ids.is_enabled(&item.full_id) {
                let all_ids = self.all_ids.clone();
                self.enabled_ids = self
                    .enabled_ids
                    .clone()
                    .move_id(&all_ids, &item.full_id, delta);
                self.is_dirty = true;
                // Move selection by delta to track the moved item
                let new_sel = self.selected_index as i64 + delta;
                if new_sel >= 0 && new_sel < self.filtered_items.len() as i64 {
                    self.selected_index = new_sel as usize;
                }
                self.refresh();
            }
        }
    }

    /// Update search query and re-filter.
    pub fn set_search(&mut self, query: impl Into<String>) {
        self.search_query = query.into();
        self.refresh();
    }

    /// Footer status text (mirrors TypeScript).
    pub fn get_footer_text(&self) -> String {
        let enabled_count = match &self.enabled_ids {
            EnabledIds::All => self.all_ids.len(),
            EnabledIds::List(ids) => ids.len(),
        };
        let all_enabled = matches!(self.enabled_ids, EnabledIds::All);
        let count_text = if all_enabled {
            "all enabled".to_string()
        } else {
            format!("{}/{} enabled", enabled_count, self.all_ids.len())
        };
        let parts = [
            "Enter toggle",
            "^A all",
            "^X clear",
            "^P provider",
            "Alt+↑↓ reorder",
            "^S save",
            &count_text,
        ];
        if self.is_dirty {
            format!("  {} (unsaved)", parts.join(" · "))
        } else {
            format!("  {}", parts.join(" · "))
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(provider: &str, id: &str) -> ScopedModelItem {
        ScopedModelItem {
            full_id: format!("{provider}/{id}"),
            provider: provider.to_string(),
            id: id.to_string(),
            name: Some(format!("{id} model")),
            enabled: true,
        }
    }

    fn make_noop_callbacks() -> ModelsCallbacks {
        ModelsCallbacks {
            on_model_toggle: Box::new(|_, _| {}),
            on_persist: Box::new(|_| {}),
            on_enable_all: Box::new(|_| {}),
            on_clear_all: Box::new(|| {}),
            on_toggle_provider: Box::new(|_, _, _| {}),
            on_cancel: Box::new(|| {}),
        }
    }

    #[test]
    fn initial_all_enabled() {
        let models = vec![
            make_model("anthropic", "claude"),
            make_model("openai", "gpt-4o"),
        ];
        let comp = ScopedModelsSelectorComponent::new(models, None, make_noop_callbacks());
        assert!(matches!(comp.enabled_ids, EnabledIds::All));
        assert_eq!(comp.filtered_items.len(), 2);
        assert!(comp.filtered_items[0].enabled);
    }

    #[test]
    fn toggle_moves_from_all_to_list() {
        let models = vec![
            make_model("anthropic", "claude"),
            make_model("openai", "gpt-4o"),
        ];
        let mut comp = ScopedModelsSelectorComponent::new(models, None, make_noop_callbacks());
        comp.toggle_selected();
        assert!(matches!(comp.enabled_ids, EnabledIds::List(_)));
        assert!(comp.is_dirty);
    }

    #[test]
    fn search_filters() {
        let models = vec![
            make_model("anthropic", "claude"),
            make_model("openai", "gpt-4o"),
        ];
        let mut comp = ScopedModelsSelectorComponent::new(models, None, make_noop_callbacks());
        comp.set_search("claude");
        assert_eq!(comp.filtered_items.len(), 1);
        assert_eq!(comp.filtered_items[0].id, "claude");
    }

    #[test]
    fn enable_ids_toggle() {
        let enabled = EnabledIds::All;
        let toggled = enabled.toggle("a/b");
        assert!(matches!(toggled, EnabledIds::List(ref ids) if ids == &["a/b"]));
    }

    #[test]
    fn enable_ids_sorted_ids() {
        let all_ids: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let enabled = EnabledIds::List(vec!["b".into()]);
        let sorted = enabled.sorted_ids(&all_ids);
        assert_eq!(sorted[0], "b");
        assert!(sorted.contains(&"a".to_string()));
        assert!(sorted.contains(&"c".to_string()));
    }

    #[test]
    fn footer_text_all_enabled() {
        let models = vec![make_model("anthropic", "claude")];
        let comp = ScopedModelsSelectorComponent::new(models, None, make_noop_callbacks());
        let footer = comp.get_footer_text();
        assert!(footer.contains("all enabled"));
    }

    #[test]
    fn footer_text_with_unsaved_dirty() {
        let models = vec![make_model("anthropic", "claude")];
        let mut comp = ScopedModelsSelectorComponent::new(models, None, make_noop_callbacks());
        comp.is_dirty = true;
        let footer = comp.get_footer_text();
        assert!(footer.contains("unsaved"));
    }
}

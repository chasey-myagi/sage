//! Config selector component.
//!
//! Translated from `components/config-selector.ts`.
//!
//! TUI component for managing package resources (enable/disable extensions,
//! skills, prompts, themes) from within the interactive session.

// ============================================================================
// ResourceType
// ============================================================================

/// The type of resource being managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    Extensions,
    Skills,
    Prompts,
    Themes,
}

impl ResourceType {
    pub fn label(self) -> &'static str {
        match self {
            ResourceType::Extensions => "Extensions",
            ResourceType::Skills => "Skills",
            ResourceType::Prompts => "Prompts",
            ResourceType::Themes => "Themes",
        }
    }
}

// ============================================================================
// ResourceScope / ResourceOrigin
// ============================================================================

/// Scope of a resource (user-global or project-local).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceScope {
    User,
    Project,
}

/// Origin of a resource (top-level config dir or a package).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceOrigin {
    Package,
    TopLevel,
}

// ============================================================================
// ResourceItem
// ============================================================================

/// A single resource entry (e.g. a specific extension file).
#[derive(Debug, Clone)]
pub struct ResourceItem {
    /// Absolute filesystem path.
    pub path: String,
    /// Whether currently enabled.
    pub enabled: bool,
    /// Human-readable display name.
    pub display_name: String,
    pub resource_type: ResourceType,
    pub scope: ResourceScope,
    pub origin: ResourceOrigin,
    /// Package source name (only meaningful when `origin == Package`).
    pub source: String,
    /// Base directory for pattern generation.
    pub base_dir: Option<String>,
    pub group_key: String,
    pub subgroup_key: String,
}

// ============================================================================
// ResourceSubgroup / ResourceGroup
// ============================================================================

/// A subgroup of resources within a group (one per ResourceType per package).
#[derive(Debug, Clone)]
pub struct ResourceSubgroup {
    pub resource_type: ResourceType,
    pub label: String,
    pub items: Vec<ResourceItem>,
}

/// A top-level group (one per unique package / scope combination).
#[derive(Debug, Clone)]
pub struct ResourceGroup {
    pub key: String,
    pub label: String,
    pub scope: ResourceScope,
    pub origin: ResourceOrigin,
    pub source: String,
    pub subgroups: Vec<ResourceSubgroup>,
}

// ============================================================================
// FlatEntry
// ============================================================================

/// A flattened view entry for rendering.
#[derive(Debug, Clone)]
pub enum FlatEntry {
    Group(ResourceGroup),
    Subgroup { subgroup: ResourceSubgroup, group_label: String },
    Item(ResourceItem),
}

// ============================================================================
// ConfigSelectorComponent
// ============================================================================

/// Component for managing package resources (enable/disable) via a TUI.
///
/// Translated from `ConfigSelectorComponent` in TypeScript.
pub struct ConfigSelectorComponent {
    pub groups: Vec<ResourceGroup>,
    pub flat_items: Vec<FlatEntry>,
    pub filtered_items: Vec<usize>,
    /// Index into `filtered_items` (which themselves index into `flat_items`).
    pub selected_index: usize,
    pub search_query: String,
    pub max_visible: usize,
    pub focused: bool,

    pub on_cancel: Option<Box<dyn Fn() + Send>>,
    pub on_exit: Option<Box<dyn Fn() + Send>>,
    pub on_toggle: Option<Box<dyn Fn(&ResourceItem, bool) + Send>>,
}

impl ConfigSelectorComponent {
    /// Create a new config selector.
    pub fn new(groups: Vec<ResourceGroup>) -> Self {
        let flat_items = Self::build_flat_list(&groups);
        let total = flat_items.len();
        // Start on first item entry
        let first_item = flat_items
            .iter()
            .position(|e| matches!(e, FlatEntry::Item(_)))
            .unwrap_or(0);
        let filtered_items: Vec<usize> = (0..total).collect();
        Self {
            groups,
            flat_items,
            filtered_items,
            selected_index: first_item.min(total.saturating_sub(1)),
            search_query: String::new(),
            max_visible: 15,
            focused: false,
            on_cancel: None,
            on_exit: None,
            on_toggle: None,
        }
    }

    fn build_flat_list(groups: &[ResourceGroup]) -> Vec<FlatEntry> {
        let mut result = Vec::new();
        for group in groups {
            result.push(FlatEntry::Group(group.clone()));
            for subgroup in &group.subgroups {
                result.push(FlatEntry::Subgroup {
                    subgroup: subgroup.clone(),
                    group_label: group.label.clone(),
                });
                for item in &subgroup.items {
                    result.push(FlatEntry::Item(item.clone()));
                }
            }
        }
        result
    }

    /// Filter entries by `query`, updating `filtered_items`.
    pub fn apply_filter(&mut self, query: &str) {
        self.search_query = query.to_string();

        if query.trim().is_empty() {
            self.filtered_items = (0..self.flat_items.len()).collect();
            self.select_first_item();
            return;
        }

        let lower = query.to_lowercase();

        // Find which item indices match
        let matching_items: Vec<usize> = self
            .flat_items
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                if let FlatEntry::Item(item) = entry {
                    if item.display_name.to_lowercase().contains(&lower)
                        || item.resource_type.label().to_lowercase().contains(&lower)
                        || item.path.to_lowercase().contains(&lower)
                    {
                        return Some(i);
                    }
                }
                None
            })
            .collect();

        // For each matching item, also include its group + subgroup headers
        let mut include: std::collections::HashSet<usize> = std::collections::HashSet::new();
        // Walk flat_items to find group/subgroup for each matching item
        let mut current_group_idx: Option<usize> = None;
        let mut current_subgroup_idx: Option<usize> = None;
        for (i, entry) in self.flat_items.iter().enumerate() {
            match entry {
                FlatEntry::Group(_) => {
                    current_group_idx = Some(i);
                    current_subgroup_idx = None;
                }
                FlatEntry::Subgroup { .. } => {
                    current_subgroup_idx = Some(i);
                }
                FlatEntry::Item(_) => {
                    if matching_items.contains(&i) {
                        include.insert(i);
                        if let Some(g) = current_group_idx {
                            include.insert(g);
                        }
                        if let Some(s) = current_subgroup_idx {
                            include.insert(s);
                        }
                    }
                }
            }
        }

        let mut filtered: Vec<usize> = include.into_iter().collect();
        filtered.sort_unstable();
        self.filtered_items = filtered;
        self.select_first_item();
    }

    fn select_first_item(&mut self) {
        let first = self
            .filtered_items
            .iter()
            .position(|&i| matches!(self.flat_items[i], FlatEntry::Item(_)));
        self.selected_index = first.unwrap_or(0);
    }

    fn find_next_item(&self, from: usize, dir: i64) -> usize {
        let mut idx = from as i64 + dir;
        while idx >= 0 && idx < self.filtered_items.len() as i64 {
            let flat_idx = self.filtered_items[idx as usize];
            if matches!(self.flat_items[flat_idx], FlatEntry::Item(_)) {
                return idx as usize;
            }
            idx += dir;
        }
        from
    }

    /// Move selection up.
    pub fn select_up(&mut self) {
        self.selected_index = self.find_next_item(self.selected_index, -1);
    }

    /// Move selection down.
    pub fn select_down(&mut self) {
        self.selected_index = self.find_next_item(self.selected_index, 1);
    }

    /// Toggle the currently selected item.
    pub fn toggle_selected(&mut self) {
        let filtered_idx = self.selected_index;
        if let Some(&flat_idx) = self.filtered_items.get(filtered_idx) {
            if let FlatEntry::Item(item) = &self.flat_items[flat_idx] {
                let new_enabled = !item.enabled;
                let item_clone = item.clone();
                // Update in flat_items
                if let FlatEntry::Item(ref mut i) = self.flat_items[flat_idx] {
                    i.enabled = new_enabled;
                }
                // Update in groups
                for group in &mut self.groups {
                    for subgroup in &mut group.subgroups {
                        for grp_item in &mut subgroup.items {
                            if grp_item.path == item_clone.path
                                && grp_item.resource_type == item_clone.resource_type
                            {
                                grp_item.enabled = new_enabled;
                            }
                        }
                    }
                }
                if let Some(cb) = &self.on_toggle {
                    cb(&item_clone, new_enabled);
                }
            }
        }
    }

    /// Cancel (Escape).
    pub fn cancel(&self) {
        if let Some(cb) = &self.on_cancel {
            cb();
        }
    }

    /// Exit (Ctrl+C).
    pub fn exit(&self) {
        if let Some(cb) = &self.on_exit {
            cb();
        }
    }

    /// Get the currently selected item, if any.
    pub fn selected_item(&self) -> Option<&ResourceItem> {
        let flat_idx = *self.filtered_items.get(self.selected_index)?;
        if let FlatEntry::Item(item) = &self.flat_items[flat_idx] {
            Some(item)
        } else {
            None
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(name: &str, enabled: bool) -> ResourceItem {
        ResourceItem {
            path: format!("/tmp/{name}.ts"),
            enabled,
            display_name: name.to_string(),
            resource_type: ResourceType::Extensions,
            scope: ResourceScope::User,
            origin: ResourceOrigin::TopLevel,
            source: "auto".to_string(),
            base_dir: None,
            group_key: "user:auto".to_string(),
            subgroup_key: "user:auto:extensions".to_string(),
        }
    }

    fn make_group(items: Vec<ResourceItem>) -> ResourceGroup {
        ResourceGroup {
            key: "user:top-level:auto".to_string(),
            label: "User (~/.pi/agent/)".to_string(),
            scope: ResourceScope::User,
            origin: ResourceOrigin::TopLevel,
            source: "auto".to_string(),
            subgroups: vec![ResourceSubgroup {
                resource_type: ResourceType::Extensions,
                label: "Extensions".to_string(),
                items,
            }],
        }
    }

    #[test]
    fn initial_state_selects_first_item() {
        let group = make_group(vec![make_item("ext1", true), make_item("ext2", false)]);
        let sel = ConfigSelectorComponent::new(vec![group]);
        assert!(sel.selected_item().is_some());
        assert_eq!(sel.selected_item().unwrap().display_name, "ext1");
    }

    #[test]
    fn select_down_advances_to_next_item() {
        let group = make_group(vec![make_item("a", true), make_item("b", true)]);
        let mut sel = ConfigSelectorComponent::new(vec![group]);
        sel.select_down();
        assert_eq!(sel.selected_item().unwrap().display_name, "b");
    }

    #[test]
    fn select_up_stays_on_first_item() {
        let group = make_group(vec![make_item("a", true), make_item("b", true)]);
        let mut sel = ConfigSelectorComponent::new(vec![group]);
        sel.select_up();
        assert_eq!(sel.selected_item().unwrap().display_name, "a");
    }

    #[test]
    fn toggle_changes_enabled() {
        let group = make_group(vec![make_item("ext1", true)]);
        let mut sel = ConfigSelectorComponent::new(vec![group]);
        sel.toggle_selected();
        assert!(!sel.selected_item().unwrap().enabled);
        sel.toggle_selected();
        assert!(sel.selected_item().unwrap().enabled);
    }

    #[test]
    fn filter_narrows_items() {
        let group = make_group(vec![make_item("alpha", true), make_item("beta", true)]);
        let mut sel = ConfigSelectorComponent::new(vec![group]);
        sel.apply_filter("alpha");
        // Only alpha item should remain
        assert_eq!(sel.selected_item().unwrap().display_name, "alpha");
        // Only one item visible
        let item_count = sel
            .filtered_items
            .iter()
            .filter(|&&i| matches!(sel.flat_items[i], FlatEntry::Item(_)))
            .count();
        assert_eq!(item_count, 1);
    }

    #[test]
    fn clear_filter_restores_all() {
        let group = make_group(vec![make_item("a", true), make_item("b", true)]);
        let mut sel = ConfigSelectorComponent::new(vec![group]);
        sel.apply_filter("a");
        sel.apply_filter("");
        let item_count = sel
            .filtered_items
            .iter()
            .filter(|&&i| matches!(sel.flat_items[i], FlatEntry::Item(_)))
            .count();
        assert_eq!(item_count, 2);
    }

    #[test]
    fn cancel_calls_callback() {
        let group = make_group(vec![]);
        let mut sel = ConfigSelectorComponent::new(vec![group]);
        let called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        sel.on_cancel = Some(Box::new(move || *called2.lock().unwrap() = true));
        sel.cancel();
        assert!(*called.lock().unwrap());
    }

    #[test]
    fn resource_type_labels() {
        assert_eq!(ResourceType::Extensions.label(), "Extensions");
        assert_eq!(ResourceType::Skills.label(), "Skills");
        assert_eq!(ResourceType::Prompts.label(), "Prompts");
        assert_eq!(ResourceType::Themes.label(), "Themes");
    }
}

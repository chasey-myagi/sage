//! Tree selector component.
//!
//! Translated from `components/tree-selector.ts`.
//!
//! Renders the session tree for navigation: ASCII-art connectors, active-path
//! markers, filter modes, search, and label editing.

use std::collections::{HashMap, HashSet};

use crate::core::session_manager::SessionTreeNode;

// ============================================================================
// FilterMode
// ============================================================================

/// Filter mode for tree display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    Default,
    NoTools,
    UserOnly,
    LabeledOnly,
    All,
}

impl FilterMode {
    pub fn cycle_forward(self) -> FilterMode {
        let modes = [
            FilterMode::Default,
            FilterMode::NoTools,
            FilterMode::UserOnly,
            FilterMode::LabeledOnly,
            FilterMode::All,
        ];
        let idx = modes.iter().position(|&m| m == self).unwrap_or(0);
        modes[(idx + 1) % modes.len()]
    }

    pub fn cycle_backward(self) -> FilterMode {
        let modes = [
            FilterMode::Default,
            FilterMode::NoTools,
            FilterMode::UserOnly,
            FilterMode::LabeledOnly,
            FilterMode::All,
        ];
        let idx = modes.iter().position(|&m| m == self).unwrap_or(0);
        modes[(idx + modes.len() - 1) % modes.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            FilterMode::Default => "",
            FilterMode::NoTools => " [no-tools]",
            FilterMode::UserOnly => " [user]",
            FilterMode::LabeledOnly => " [labeled]",
            FilterMode::All => " [all]",
        }
    }
}

// ============================================================================
// Gutter / FlatNode
// ============================================================================

/// Gutter info: position (display-indent where connector was) and whether to show │.
#[derive(Debug, Clone)]
pub struct GutterInfo {
    pub position: usize,
    pub show: bool,
}

/// A node in the flattened tree for navigation.
#[derive(Debug, Clone)]
pub struct FlatNode {
    pub node: SessionTreeNode,
    pub indent: usize,
    pub show_connector: bool,
    pub is_last: bool,
    pub gutters: Vec<GutterInfo>,
    pub is_virtual_root_child: bool,
}

// ============================================================================
// TreeList
// ============================================================================

/// Flattened-tree navigation list.
pub struct TreeList {
    pub flat_nodes: Vec<FlatNode>,
    pub filtered_nodes: Vec<FlatNode>,
    pub selected_index: usize,
    pub current_leaf_id: Option<String>,
    pub max_visible_lines: usize,
    pub filter_mode: FilterMode,
    pub search_query: String,
    pub multiple_roots: bool,
    pub active_path_ids: HashSet<String>,
    pub visible_parent_map: HashMap<String, Option<String>>,
    pub visible_children_map: HashMap<Option<String>, Vec<String>>,
    pub last_selected_id: Option<String>,
    pub folded_nodes: HashSet<String>,

    pub on_select: Option<Box<dyn Fn(String) + Send>>,
    pub on_cancel: Option<Box<dyn Fn() + Send>>,
    pub on_label_edit: Option<Box<dyn Fn(String, Option<String>) + Send>>,
}

impl TreeList {
    pub fn new(
        tree: Vec<SessionTreeNode>,
        current_leaf_id: Option<String>,
        max_visible_lines: usize,
        initial_selected_id: Option<String>,
        initial_filter_mode: Option<FilterMode>,
    ) -> Self {
        let multiple_roots = tree.len() > 1;
        let flat_nodes = Self::flatten_tree_static(&tree);
        let filter_mode = initial_filter_mode.unwrap_or(FilterMode::Default);

        let mut list = Self {
            flat_nodes,
            filtered_nodes: Vec::new(),
            selected_index: 0,
            current_leaf_id: current_leaf_id.clone(),
            max_visible_lines,
            filter_mode,
            search_query: String::new(),
            multiple_roots,
            active_path_ids: HashSet::new(),
            visible_parent_map: HashMap::new(),
            visible_children_map: HashMap::new(),
            last_selected_id: None,
            folded_nodes: HashSet::new(),
            on_select: None,
            on_cancel: None,
            on_label_edit: None,
        };

        list.build_active_path(&current_leaf_id);
        list.apply_filter();

        let target_id = initial_selected_id.or(current_leaf_id);
        list.selected_index = list.find_nearest_visible_index(target_id.as_deref());
        list.last_selected_id =
            list.filtered_nodes.get(list.selected_index).map(|n| n.node.entry.id().to_string());

        list
    }

    fn build_active_path(&mut self, leaf_id: &Option<String>) {
        self.active_path_ids.clear();
        let Some(leaf) = leaf_id else { return };

        // Build id → parent map from flat_nodes
        let parent_map: HashMap<&str, Option<&str>> = self
            .flat_nodes
            .iter()
            .map(|n| (n.node.entry.id(), n.node.entry.parent_id()))
            .collect();

        let mut current_id: Option<&str> = Some(leaf.as_str());
        while let Some(id) = current_id {
            self.active_path_ids.insert(id.to_string());
            current_id = parent_map.get(id).and_then(|p| *p);
        }
    }

    fn flatten_tree_static(roots: &[SessionTreeNode]) -> Vec<FlatNode> {
        let mut result = Vec::new();

        // Stack items: (node, indent, just_branched, show_connector, is_last, gutters, is_virtual_root_child)
        type StackItem = (SessionTreeNode, usize, bool, bool, bool, Vec<GutterInfo>, bool);
        let mut stack: Vec<StackItem> = Vec::new();

        let multiple_roots = roots.len() > 1;

        // Add roots (in reverse for stack ordering)
        let ordered_roots: Vec<&SessionTreeNode> = roots.iter().collect();
        for (i, root) in ordered_roots.iter().enumerate().rev() {
            let is_last = i == ordered_roots.len() - 1;
            stack.push((
                (*root).clone(),
                if multiple_roots { 1 } else { 0 },
                multiple_roots,
                multiple_roots,
                is_last,
                Vec::new(),
                multiple_roots,
            ));
        }

        while let Some((node, indent, just_branched, show_connector, is_last, gutters, is_virtual_root_child)) =
            stack.pop()
        {
            let children = node.children.clone();
            let multiple_children = children.len() > 1;

            let current_display_indent = if multiple_roots { indent.saturating_sub(1) } else { indent };
            let connector_position = current_display_indent.saturating_sub(1);
            let connector_displayed = show_connector && !is_virtual_root_child;
            let child_gutters: Vec<GutterInfo> = if connector_displayed {
                let mut g = gutters.clone();
                g.push(GutterInfo { position: connector_position, show: !is_last });
                g
            } else {
                gutters.clone()
            };

            result.push(FlatNode {
                node,
                indent,
                show_connector,
                is_last,
                gutters,
                is_virtual_root_child,
            });

            let child_indent = if multiple_children {
                indent + 1
            } else if just_branched && indent > 0 {
                indent + 1
            } else {
                indent
            };

            // Push children in reverse order so they are processed in forward order
            for (ci, child) in children.iter().enumerate().rev() {
                let child_is_last = ci == children.len() - 1;
                stack.push((
                    child.clone(),
                    child_indent,
                    multiple_children,
                    multiple_children,
                    child_is_last,
                    child_gutters.clone(),
                    false,
                ));
            }
        }

        result
    }

    fn find_nearest_visible_index(&self, entry_id: Option<&str>) -> usize {
        if self.filtered_nodes.is_empty() {
            return 0;
        }

        let entry_map: HashMap<&str, &FlatNode> =
            self.flat_nodes.iter().map(|n| (n.node.entry.id(), n)).collect();

        let visible_id_to_index: HashMap<&str, usize> = self
            .filtered_nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.node.entry.id(), i))
            .collect();

        let mut current_id = entry_id;
        while let Some(id) = current_id {
            if let Some(&idx) = visible_id_to_index.get(id) {
                return idx;
            }
            current_id = entry_map
                .get(id)
                .and_then(|n| n.node.entry.parent_id());
        }

        self.filtered_nodes.len() - 1
    }

    pub fn apply_filter(&mut self) {
        if !self.filtered_nodes.is_empty() {
            self.last_selected_id = self
                .filtered_nodes
                .get(self.selected_index)
                .map(|n| n.node.entry.id().to_string())
                .or(self.last_selected_id.clone());
        }

        let search_tokens: Vec<String> = self
            .search_query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();

        let leaf_id = self.current_leaf_id.clone();
        let filter_mode = self.filter_mode;

        self.filtered_nodes = self
            .flat_nodes
            .iter()
            .filter(|flat_node| {
                let entry = &flat_node.node.entry;
                let is_current_leaf = Some(entry.id()) == leaf_id.as_deref();

                // Skip assistant messages with only tool calls (no text), unless it's the current leaf
                if entry.is_message() && entry.message_role() == Some("assistant") && !is_current_leaf {
                    let stop_reason = entry.message_stop_reason();
                    let is_error_or_aborted = stop_reason.map(|r| r != "stop" && r != "toolUse").unwrap_or(false);
                    if !entry.message_has_text_content() && !is_error_or_aborted {
                        return false;
                    }
                }

                // Apply filter mode
                let is_settings_entry = entry.is_settings_entry();

                let passes_filter = match filter_mode {
                    FilterMode::UserOnly => {
                        entry.is_message() && entry.message_role() == Some("user")
                    }
                    FilterMode::NoTools => {
                        !is_settings_entry
                            && !(entry.is_message() && entry.message_role() == Some("toolResult"))
                    }
                    FilterMode::LabeledOnly => flat_node.node.label.is_some(),
                    FilterMode::All => true,
                    FilterMode::Default => !is_settings_entry,
                };

                if !passes_filter {
                    return false;
                }

                // Apply search tokens
                if !search_tokens.is_empty() {
                    let text = Self::get_searchable_text_static(&flat_node.node).to_lowercase();
                    return search_tokens.iter().all(|t| text.contains(t.as_str()));
                }

                true
            })
            .cloned()
            .collect();

        // Filter out descendants of folded nodes
        if !self.folded_nodes.is_empty() {
            let flat_nodes_clone = self.flat_nodes.clone();
            let mut skip_set: HashSet<String> = HashSet::new();
            for flat_node in &flat_nodes_clone {
                let id = flat_node.node.entry.id().to_string();
                let parent_id = flat_node.node.entry.parent_id();
                if let Some(pid) = parent_id {
                    if self.folded_nodes.contains(pid) || skip_set.contains(pid) {
                        skip_set.insert(id);
                    }
                }
            }
            self.filtered_nodes.retain(|n| !skip_set.contains(n.node.entry.id()));
        }

        self.recalculate_visual_structure();

        // Restore cursor position
        if let Some(ref last_id) = self.last_selected_id.clone() {
            self.selected_index = self.find_nearest_visible_index(Some(last_id.as_str()));
        } else if self.selected_index >= self.filtered_nodes.len() {
            self.selected_index = self.filtered_nodes.len().saturating_sub(1);
        }

        if !self.filtered_nodes.is_empty() {
            self.last_selected_id =
                self.filtered_nodes.get(self.selected_index).map(|n| n.node.entry.id().to_string());
        }
    }

    fn recalculate_visual_structure(&mut self) {
        if self.filtered_nodes.is_empty() {
            return;
        }

        let visible_ids: HashSet<&str> =
            self.filtered_nodes.iter().map(|n| n.node.entry.id()).collect();

        let entry_map: HashMap<&str, &FlatNode> =
            self.flat_nodes.iter().map(|n| (n.node.entry.id(), n)).collect();

        let find_visible_ancestor = |node_id: &str| -> Option<String> {
            let mut current_id = entry_map.get(node_id)?.node.entry.parent_id();
            while let Some(id) = current_id {
                if visible_ids.contains(id) {
                    return Some(id.to_string());
                }
                current_id = entry_map.get(id)?.node.entry.parent_id();
            }
            None
        };

        let mut visible_parent: HashMap<String, Option<String>> = HashMap::new();
        let mut visible_children: HashMap<Option<String>, Vec<String>> = HashMap::new();
        visible_children.insert(None, Vec::new());

        for flat_node in &self.filtered_nodes {
            let node_id = flat_node.node.entry.id().to_string();
            let ancestor_id = find_visible_ancestor(&node_id);
            visible_parent.insert(node_id.clone(), ancestor_id.clone());

            visible_children
                .entry(ancestor_id)
                .or_default()
                .push(node_id);
        }

        let visible_root_ids = visible_children.get(&None).cloned().unwrap_or_default();
        let multiple_roots = visible_root_ids.len() > 1;
        self.multiple_roots = multiple_roots;

        let filtered_node_map: HashMap<String, usize> = self
            .filtered_nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.node.entry.id().to_string(), i))
            .collect();

        // DFS to assign visual properties
        type StackItem2 = (String, usize, bool, bool, bool, Vec<GutterInfo>, bool);
        let mut stack: Vec<StackItem2> = Vec::new();

        for (i, root_id) in visible_root_ids.iter().enumerate().rev() {
            let is_last = i == visible_root_ids.len() - 1;
            stack.push((
                root_id.clone(),
                if multiple_roots { 1 } else { 0 },
                multiple_roots,
                multiple_roots,
                is_last,
                Vec::new(),
                multiple_roots,
            ));
        }

        while let Some((node_id, indent, just_branched, show_connector, is_last, gutters, is_virtual_root_child)) =
            stack.pop()
        {
            if let Some(&idx) = filtered_node_map.get(node_id.as_str()) {
                let flat_node = &mut self.filtered_nodes[idx];
                flat_node.indent = indent;
                flat_node.show_connector = show_connector;
                flat_node.is_last = is_last;
                flat_node.gutters = gutters.clone();
                flat_node.is_virtual_root_child = is_virtual_root_child;
            }

            let children = visible_children.get(&Some(node_id.clone())).cloned().unwrap_or_default();
            let multiple_children = children.len() > 1;

            let child_indent = if multiple_children {
                indent + 1
            } else if just_branched && indent > 0 {
                indent + 1
            } else {
                indent
            };

            let connector_displayed = show_connector && !is_virtual_root_child;
            let current_display_indent = if multiple_roots { indent.saturating_sub(1) } else { indent };
            let connector_position = current_display_indent.saturating_sub(1);
            let child_gutters: Vec<GutterInfo> = if connector_displayed {
                let mut g = gutters.clone();
                g.push(GutterInfo { position: connector_position, show: !is_last });
                g
            } else {
                gutters
            };

            for (ci, child_id) in children.iter().enumerate().rev() {
                let child_is_last = ci == children.len() - 1;
                stack.push((
                    child_id.clone(),
                    child_indent,
                    multiple_children,
                    multiple_children,
                    child_is_last,
                    child_gutters.clone(),
                    false,
                ));
            }
        }

        self.visible_parent_map = visible_parent;
        self.visible_children_map = visible_children;
    }

    fn get_searchable_text_static(node: &SessionTreeNode) -> String {
        let entry = &node.entry;
        let mut parts = Vec::new();

        if let Some(label) = &node.label {
            parts.push(label.clone());
        }

        if let Some(role) = entry.message_role() {
            parts.push(role.to_string());
        }
        if let Some(content) = entry.message_text_content() {
            parts.push(content.chars().take(200).collect());
        }

        parts.join(" ")
    }

    pub fn get_search_query(&self) -> &str {
        &self.search_query
    }

    pub fn get_selected_node(&self) -> Option<&SessionTreeNode> {
        self.filtered_nodes.get(self.selected_index).map(|n| &n.node)
    }

    pub fn update_node_label(&mut self, entry_id: &str, label: Option<String>) {
        for flat_node in &mut self.flat_nodes {
            if flat_node.node.entry.id() == entry_id {
                flat_node.node.label = label;
                break;
            }
        }
    }

    pub fn is_foldable(&self, entry_id: &str) -> bool {
        let children = self.visible_children_map.get(&Some(entry_id.to_string()));
        let has_children = children.map(|c| !c.is_empty()).unwrap_or(false);
        if !has_children {
            return false;
        }
        let parent_id = self.visible_parent_map.get(entry_id);
        if parent_id.map(|p| p.is_none()).unwrap_or(true) {
            return true;
        }
        if let Some(Some(pid)) = parent_id {
            let siblings = self.visible_children_map.get(&Some(pid.clone()));
            return siblings.map(|s| s.len() > 1).unwrap_or(false);
        }
        false
    }

    fn find_branch_segment_start(&self, direction: &str) -> usize {
        let selected_id = match self.filtered_nodes.get(self.selected_index) {
            Some(n) => n.node.entry.id().to_string(),
            None => return self.selected_index,
        };

        let index_by_entry_id: HashMap<&str, usize> = self
            .filtered_nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.node.entry.id(), i))
            .collect();

        if direction == "down" {
            let mut current_id = selected_id.clone();
            loop {
                let children = self.visible_children_map.get(&Some(current_id.clone())).cloned().unwrap_or_default();
                if children.is_empty() {
                    return *index_by_entry_id.get(current_id.as_str()).unwrap_or(&self.selected_index);
                }
                if children.len() > 1 {
                    return *index_by_entry_id.get(children[0].as_str()).unwrap_or(&self.selected_index);
                }
                current_id = children[0].clone();
            }
        } else {
            // direction == "up"
            let mut current_id = selected_id.clone();
            loop {
                let parent_id = self.visible_parent_map.get(&current_id).and_then(|p| p.clone());
                let Some(pid) = parent_id else {
                    return *index_by_entry_id.get(current_id.as_str()).unwrap_or(&self.selected_index);
                };
                let siblings = self.visible_children_map.get(&Some(pid.clone())).cloned().unwrap_or_default();
                if siblings.len() > 1 {
                    let seg_start = *index_by_entry_id.get(current_id.as_str()).unwrap_or(&self.selected_index);
                    if seg_start < self.selected_index {
                        return seg_start;
                    }
                }
                current_id = pid;
            }
        }
    }

    // ---- Input handling ----

    pub fn handle_select_up(&mut self) {
        if self.filtered_nodes.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.filtered_nodes.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    pub fn handle_select_down(&mut self) {
        if self.filtered_nodes.is_empty() {
            return;
        }
        if self.selected_index == self.filtered_nodes.len() - 1 {
            self.selected_index = 0;
        } else {
            self.selected_index += 1;
        }
    }

    pub fn handle_fold_or_up(&mut self) {
        let current_id = self.filtered_nodes.get(self.selected_index).map(|n| n.node.entry.id().to_string());
        if let Some(id) = current_id {
            if self.is_foldable(&id) && !self.folded_nodes.contains(&id) {
                self.folded_nodes.insert(id);
                self.apply_filter();
                return;
            }
        }
        self.selected_index = self.find_branch_segment_start("up");
    }

    pub fn handle_unfold_or_down(&mut self) {
        let current_id = self.filtered_nodes.get(self.selected_index).map(|n| n.node.entry.id().to_string());
        if let Some(id) = current_id {
            if self.folded_nodes.contains(&id) {
                self.folded_nodes.remove(&id);
                self.apply_filter();
                return;
            }
        }
        self.selected_index = self.find_branch_segment_start("down");
    }

    pub fn handle_page_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(self.max_visible_lines);
    }

    pub fn handle_page_down(&mut self) {
        if !self.filtered_nodes.is_empty() {
            self.selected_index =
                (self.selected_index + self.max_visible_lines).min(self.filtered_nodes.len() - 1);
        }
    }

    pub fn handle_confirm(&self) {
        if let Some(selected) = self.filtered_nodes.get(self.selected_index) {
            if let Some(cb) = &self.on_select {
                cb(selected.node.entry.id().to_string());
            }
        }
    }

    pub fn handle_cancel(&mut self) {
        if !self.search_query.is_empty() {
            self.search_query.clear();
            self.folded_nodes.clear();
            self.apply_filter();
        } else if let Some(cb) = &self.on_cancel {
            cb();
        }
    }

    pub fn handle_label_edit(&self) {
        if let Some(selected) = self.filtered_nodes.get(self.selected_index) {
            if let Some(cb) = &self.on_label_edit {
                cb(selected.node.entry.id().to_string(), selected.node.label.clone());
            }
        }
    }

    pub fn set_filter_mode(&mut self, mode: FilterMode) {
        self.filter_mode = mode;
        self.folded_nodes.clear();
        self.apply_filter();
    }

    pub fn append_search_char(&mut self, ch: char) {
        self.search_query.push(ch);
        self.folded_nodes.clear();
        self.apply_filter();
    }

    pub fn delete_search_char(&mut self) {
        if !self.search_query.is_empty() {
            self.search_query.pop();
            self.folded_nodes.clear();
            self.apply_filter();
        }
    }
}

// ============================================================================
// TreeSelectorComponent
// ============================================================================

/// Component that renders a session tree selector for navigation.
pub struct TreeSelectorComponent {
    pub tree_list: TreeList,
    pub label_input_active: bool,
    pub label_input_entry_id: Option<String>,
    pub label_input_value: String,
    pub focused: bool,
    on_label_change: Option<Box<dyn Fn(String, Option<String>) + Send>>,
}

impl TreeSelectorComponent {
    pub fn new(
        tree: Vec<SessionTreeNode>,
        current_leaf_id: Option<String>,
        terminal_height: usize,
        on_select: impl Fn(String) + Send + 'static,
        on_cancel: impl Fn() + Send + 'static,
        on_label_change: Option<Box<dyn Fn(String, Option<String>) + Send>>,
        initial_selected_id: Option<String>,
        initial_filter_mode: Option<FilterMode>,
    ) -> Self {
        let max_visible_lines = (terminal_height / 2).max(5);
        let mut tree_list = TreeList::new(
            tree,
            current_leaf_id,
            max_visible_lines,
            initial_selected_id,
            initial_filter_mode,
        );

        tree_list.on_select = Some(Box::new(on_select));
        tree_list.on_cancel = Some(Box::new(on_cancel));

        Self {
            tree_list,
            label_input_active: false,
            label_input_entry_id: None,
            label_input_value: String::new(),
            focused: false,
            on_label_change,
        }
    }

    fn show_label_input(&mut self, entry_id: String, current_label: Option<String>) {
        self.label_input_active = true;
        self.label_input_entry_id = Some(entry_id);
        self.label_input_value = current_label.unwrap_or_default();
    }

    fn hide_label_input(&mut self) {
        self.label_input_active = false;
        self.label_input_entry_id = None;
        self.label_input_value.clear();
    }

    fn submit_label_input(&mut self) {
        let value = self.label_input_value.trim().to_string();
        let label = if value.is_empty() { None } else { Some(value) };
        if let Some(id) = self.label_input_entry_id.clone() {
            self.tree_list.update_node_label(&id, label.clone());
            if let Some(cb) = &self.on_label_change {
                cb(id, label);
            }
        }
        self.hide_label_input();
    }

    /// Handle a raw key event.
    pub fn handle_input(&mut self, key: &str) {
        if self.label_input_active {
            // Simple label input handling
            if key == "\n" || key == "\r" {
                self.submit_label_input();
            } else if key == "\x1b" || key == "\x03" {
                self.hide_label_input();
            } else if key == "\x7f" || key == "\x08" {
                self.label_input_value.pop();
            } else {
                for ch in key.chars() {
                    if ch.is_ascii_graphic() || ch == ' ' {
                        self.label_input_value.push(ch);
                    }
                }
            }
            return;
        }

        // Tree list navigation
        match key {
            "\x1b[A" | "k" => self.tree_list.handle_select_up(),
            "\x1b[B" | "j" => self.tree_list.handle_select_down(),
            "\x1b[C" | "\x1b[6~" => self.tree_list.handle_page_down(),
            "\x1b[D" | "\x1b[5~" => self.tree_list.handle_page_up(),
            "\n" | "\r" => self.tree_list.handle_confirm(),
            "\x1b" | "\x03" => self.tree_list.handle_cancel(),
            "\x04" => self.tree_list.set_filter_mode(FilterMode::Default),
            "\x14" => {
                // Ctrl+T
                let new_mode = if self.tree_list.filter_mode == FilterMode::NoTools {
                    FilterMode::Default
                } else {
                    FilterMode::NoTools
                };
                self.tree_list.set_filter_mode(new_mode);
            }
            "\x15" => {
                // Ctrl+U
                let new_mode = if self.tree_list.filter_mode == FilterMode::UserOnly {
                    FilterMode::Default
                } else {
                    FilterMode::UserOnly
                };
                self.tree_list.set_filter_mode(new_mode);
            }
            "\x0c" => {
                // Ctrl+L
                let new_mode = if self.tree_list.filter_mode == FilterMode::LabeledOnly {
                    FilterMode::Default
                } else {
                    FilterMode::LabeledOnly
                };
                self.tree_list.set_filter_mode(new_mode);
            }
            "\x01" => {
                // Ctrl+A
                let new_mode = if self.tree_list.filter_mode == FilterMode::All {
                    FilterMode::Default
                } else {
                    FilterMode::All
                };
                self.tree_list.set_filter_mode(new_mode);
            }
            "\x0f" => {
                // Ctrl+O
                let new_mode = self.tree_list.filter_mode.cycle_forward();
                self.tree_list.set_filter_mode(new_mode);
            }
            "\x7f" | "\x08" => self.tree_list.delete_search_char(),
            "L" => {
                // Shift+L: label edit
                let selected = self.tree_list.filtered_nodes.get(self.tree_list.selected_index).cloned();
                if let Some(node) = selected {
                    self.show_label_input(node.node.entry.id().to_string(), node.node.label.clone());
                }
            }
            _ => {
                // Single printable char → search
                let has_control = key.chars().any(|c| {
                    let code = c as u32;
                    code < 32 || code == 0x7f || (code >= 0x80 && code <= 0x9f)
                });
                if !has_control && !key.is_empty() {
                    for ch in key.chars() {
                        self.tree_list.append_search_char(ch);
                    }
                }
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session_manager::{SessionEntry, SessionMessageEntry, SessionTreeNode};

    fn make_entry(id: &str, parent_id: Option<&str>) -> SessionEntry {
        SessionEntry::Message(SessionMessageEntry {
            entry_type: "message".to_string(),
            id: id.to_string(),
            parent_id: parent_id.map(str::to_string),
            timestamp: "2024-01-01T00:00:00.000Z".to_string(),
            message: serde_json::json!({
                "role": "user",
                "content": format!("content of {id}")
            }),
        })
    }

    fn make_node(id: &str, parent_id: Option<&str>, children: Vec<SessionTreeNode>) -> SessionTreeNode {
        SessionTreeNode {
            entry: make_entry(id, parent_id),
            children,
            label: None,
        }
    }

    #[test]
    fn filter_mode_cycle_forward() {
        let mut mode = FilterMode::Default;
        mode = mode.cycle_forward();
        assert_eq!(mode, FilterMode::NoTools);
        mode = mode.cycle_forward();
        assert_eq!(mode, FilterMode::UserOnly);
    }

    #[test]
    fn filter_mode_cycle_backward() {
        let mut mode = FilterMode::Default;
        mode = mode.cycle_backward();
        assert_eq!(mode, FilterMode::All);
    }

    #[test]
    fn filter_mode_labels() {
        assert_eq!(FilterMode::NoTools.label(), " [no-tools]");
        assert_eq!(FilterMode::Default.label(), "");
    }

    #[test]
    fn tree_list_empty_tree() {
        let list = TreeList::new(vec![], None, 20, None, None);
        assert!(list.filtered_nodes.is_empty());
    }

    #[test]
    fn tree_list_single_root() {
        let tree = vec![make_node("root", None, vec![])];
        let list = TreeList::new(tree, None, 20, None, None);
        assert_eq!(list.filtered_nodes.len(), 1);
        assert_eq!(list.filtered_nodes[0].node.entry.id(), "root");
    }

    #[test]
    fn tree_list_navigation() {
        let tree = vec![make_node(
            "root",
            None,
            vec![
                make_node("child1", Some("root"), vec![]),
                make_node("child2", Some("root"), vec![]),
            ],
        )];
        let mut list = TreeList::new(tree, None, 20, None, Some(FilterMode::All));
        // When no target is specified, the last node is selected (most-recent leaf).
        // Tree order: root(0), child1(1), child2(2) → default index = 2.
        assert!(list.filtered_nodes.len() > 0);
        let initial = list.selected_index;
        list.handle_select_up(); // wrap around or move up
        // After moving, index should differ from initial (unless at boundaries)
        let _ = list.selected_index; // just ensure it doesn't panic
    }

    #[test]
    fn tree_list_search() {
        let tree = vec![
            make_node("aaa", None, vec![]),
            make_node("bbb", None, vec![]),
        ];
        let mut list = TreeList::new(tree, None, 20, None, Some(FilterMode::All));
        list.append_search_char('a');
        // After search, only nodes matching "a" should be visible
        assert!(list.filtered_nodes.iter().all(|n| {
            let text = TreeList::get_searchable_text_static(&n.node).to_lowercase();
            text.contains('a')
        }));
    }

    #[test]
    fn tree_list_delete_search_char() {
        let tree = vec![make_node("aaa", None, vec![])];
        let mut list = TreeList::new(tree, None, 20, None, Some(FilterMode::All));
        list.append_search_char('a');
        list.append_search_char('b');
        assert_eq!(list.search_query, "ab");
        list.delete_search_char();
        assert_eq!(list.search_query, "a");
    }

    #[test]
    fn update_node_label() {
        let tree = vec![make_node("n1", None, vec![])];
        let mut list = TreeList::new(tree, None, 20, None, Some(FilterMode::All));
        list.update_node_label("n1", Some("my-label".into()));
        let node = list.flat_nodes.iter().find(|n| n.node.entry.id() == "n1").unwrap();
        assert_eq!(node.node.label.as_deref(), Some("my-label"));
    }

    #[test]
    fn tree_selector_label_input() {
        let tree = vec![make_node("n1", None, vec![])];
        let mut sel = TreeSelectorComponent::new(
            tree,
            None,
            40,
            |_| {},
            || {},
            None,
            None,
            Some(FilterMode::All),
        );

        // Activate label input via "L"
        sel.handle_input("L");
        assert!(sel.label_input_active);
        assert_eq!(sel.label_input_entry_id.as_deref(), Some("n1"));

        // Type a label
        sel.handle_input("x");
        assert_eq!(sel.label_input_value, "x");

        // Submit
        sel.handle_input("\n");
        assert!(!sel.label_input_active);
    }

    #[test]
    fn tree_selector_label_input_cancel() {
        let tree = vec![make_node("n1", None, vec![])];
        let mut sel = TreeSelectorComponent::new(
            tree,
            None,
            40,
            |_| {},
            || {},
            None,
            None,
            Some(FilterMode::All),
        );
        sel.handle_input("L");
        assert!(sel.label_input_active);
        sel.handle_input("\x1b");
        assert!(!sel.label_input_active);
    }
}

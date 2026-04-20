/// Minimal TUI implementation with differential rendering.

use std::sync::Arc;

use crate::keys::is_key_release;
use crate::terminal::Terminal;
use crate::terminal_image::is_image_line;
use crate::utils::{extract_segments, slice_by_column, slice_with_width, visible_width};

// =============================================================================
// Component trait
// =============================================================================

/// Component interface — all UI elements must implement this.
pub trait Component: Send {
    /// Render the component to lines for the given viewport width.
    fn render(&self, width: u16) -> Vec<String>;

    /// Optional handler for keyboard input when component has focus.
    fn handle_input(&mut self, _data: &str) {}

    /// If true, component receives key release events (Kitty protocol).
    fn wants_key_release(&self) -> bool {
        false
    }

    /// Invalidate any cached rendering state.
    fn invalidate(&mut self) {}
}

/// Interface for components that can receive focus and display a hardware cursor.
pub trait Focusable {
    fn focused(&self) -> bool;
    fn set_focused(&mut self, focused: bool);
}

/// Cursor position marker — zero-width APC escape sequence.
/// Components emit this at the cursor position when focused.
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

/// Re-export visible_width for external use.
pub use crate::utils::visible_width as tui_visible_width;

// =============================================================================
// Overlay types
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAnchor {
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    TopCenter,
    BottomCenter,
    LeftCenter,
    RightCenter,
}

#[derive(Debug, Clone, Default)]
pub struct OverlayMargin {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

#[derive(Debug, Clone)]
pub enum SizeValue {
    Absolute(u16),
    Percent(f64),
}

impl SizeValue {
    pub fn resolve(&self, reference: u16) -> u16 {
        match self {
            SizeValue::Absolute(v) => *v,
            SizeValue::Percent(p) => ((reference as f64 * p / 100.0).floor() as u16).max(1),
        }
    }
}

#[derive(Clone, Default)]
pub struct OverlayOptions {
    pub width: Option<SizeValue>,
    pub min_width: Option<u16>,
    pub max_height: Option<SizeValue>,
    pub anchor: Option<OverlayAnchor>,
    pub offset_x: Option<i16>,
    pub offset_y: Option<i16>,
    pub row: Option<SizeValue>,
    pub col: Option<SizeValue>,
    pub margin: Option<OverlayMargin>,
    pub visible: Option<Arc<dyn Fn(u16, u16) -> bool + Send + Sync>>,
    pub non_capturing: bool,
}

impl std::fmt::Debug for OverlayOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlayOptions")
            .field("width", &self.width)
            .field("min_width", &self.min_width)
            .field("max_height", &self.max_height)
            .field("anchor", &self.anchor)
            .field("offset_x", &self.offset_x)
            .field("offset_y", &self.offset_y)
            .field("row", &self.row)
            .field("col", &self.col)
            .field("margin", &self.margin)
            .field("visible", &self.visible.as_ref().map(|_| "<fn>"))
            .field("non_capturing", &self.non_capturing)
            .finish()
    }
}

// =============================================================================
// Container
// =============================================================================

/// A component that contains other components.
pub struct Container {
    pub children: Vec<Box<dyn Component>>,
}

impl Container {
    pub fn new() -> Self {
        Self { children: Vec::new() }
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) {
        self.children.push(component);
    }

    pub fn clear(&mut self) {
        self.children.clear();
    }

    pub fn remove_child_at(&mut self, index: usize) -> Option<Box<dyn Component>> {
        if index < self.children.len() {
            Some(self.children.remove(index))
        } else {
            None
        }
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Container {
    fn render(&self, width: u16) -> Vec<String> {
        let mut lines = Vec::new();
        for child in &self.children {
            lines.extend(child.render(width));
        }
        lines
    }

    fn invalidate(&mut self) {
        for child in &mut self.children {
            child.invalidate();
        }
    }
}

// =============================================================================
// Overlay handle
// =============================================================================

/// Opaque handle returned by `show_overlay`. Used to remove a specific overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayHandle(u64);

// =============================================================================
// Overlay entry
// =============================================================================

struct OverlayEntry {
    id: u64,
    component: Box<dyn Component>,
    options: Option<OverlayOptions>,
    pre_focus_idx: Option<usize>, // index of focused container child before this overlay
    overlay_had_focus: bool,      // true if this overlay's component currently holds focus
    hidden: bool,
    focus_order: u64,
}

// =============================================================================
// TUI
// =============================================================================

type InputListenerResult = Option<(bool, Option<String>)>; // (consume, transformed_data)
type InputListener = Box<dyn Fn(&str) -> InputListenerResult + Send>;

/// Main TUI class for managing terminal UI with differential rendering.
pub struct TUI {
    pub terminal: Box<dyn Terminal>,
    container: Container,
    previous_lines: Vec<String>,
    previous_width: i32,
    previous_height: i32,
    focused_component_idx: Option<usize>, // index into children
    input_listeners: Vec<(u64, InputListener)>,
    next_listener_id: u64,
    pub on_debug: Option<Arc<dyn Fn() + Send + Sync>>,
    render_requested: bool,
    cursor_row: i32,
    hardware_cursor_row: i32,
    input_buffer: String,
    cell_size_query_pending: bool,
    show_hardware_cursor: bool,
    clear_on_shrink: bool,
    max_lines_rendered: usize,
    previous_viewport_top: usize,
    full_redraw_count: u64,
    stopped: bool,
    overlay_stack: Vec<OverlayEntry>,
    focus_order_counter: u64,
    next_overlay_id: u64,
    /// Which overlay (by id) currently holds focus, if any.
    overlay_focused_id: Option<u64>,
}

impl TUI {
    pub fn new(terminal: Box<dyn Terminal>) -> Self {
        let show_hardware_cursor = std::env::var("PI_HARDWARE_CURSOR").as_deref() == Ok("1");
        let clear_on_shrink = std::env::var("PI_CLEAR_ON_SHRINK").as_deref() == Ok("1");
        Self {
            terminal,
            container: Container::new(),
            previous_lines: Vec::new(),
            previous_width: 0,
            previous_height: 0,
            focused_component_idx: None,
            input_listeners: Vec::new(),
            next_listener_id: 0,
            on_debug: None,
            render_requested: false,
            cursor_row: 0,
            hardware_cursor_row: 0,
            input_buffer: String::new(),
            cell_size_query_pending: false,
            show_hardware_cursor,
            clear_on_shrink,
            max_lines_rendered: 0,
            previous_viewport_top: 0,
            full_redraw_count: 0,
            stopped: false,
            overlay_stack: Vec::new(),
            focus_order_counter: 0,
            next_overlay_id: 0,
            overlay_focused_id: None,
        }
    }

    pub fn full_redraws(&self) -> u64 {
        self.full_redraw_count
    }

    pub fn get_show_hardware_cursor(&self) -> bool {
        self.show_hardware_cursor
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        if self.show_hardware_cursor == enabled {
            return;
        }
        self.show_hardware_cursor = enabled;
        if !enabled {
            self.terminal.hide_cursor();
        }
        self.request_render(false);
    }

    pub fn get_clear_on_shrink(&self) -> bool {
        self.clear_on_shrink
    }

    pub fn set_clear_on_shrink(&mut self, enabled: bool) {
        self.clear_on_shrink = enabled;
    }

    /// Add a child component to the container.
    pub fn add_child(&mut self, component: Box<dyn Component>) {
        self.container.add_child(component);
    }

    /// Render all children.
    fn render(&self, width: u16) -> Vec<String> {
        self.container.render(width)
    }

    /// Invalidate all components.
    pub fn invalidate(&mut self) {
        self.container.invalidate();
        for overlay in &mut self.overlay_stack {
            overlay.component.invalidate();
        }
    }

    /// Start the TUI.
    pub fn start(&mut self) {
        self.stopped = false;
        // Note: in actual use, we'd set up the terminal with callbacks here
        // For the Rust version, this is typically called from a higher-level coordinator
        self.terminal.hide_cursor();
        self.query_cell_size();
        self.request_render(false);
    }

    /// Add a global input listener. Returns a listener ID to remove later.
    pub fn add_input_listener<F>(&mut self, listener: F) -> u64
    where
        F: Fn(&str) -> InputListenerResult + Send + 'static,
    {
        let id = self.next_listener_id;
        self.next_listener_id += 1;
        self.input_listeners.push((id, Box::new(listener)));
        id
    }

    /// Remove an input listener by ID.
    pub fn remove_input_listener(&mut self, id: u64) {
        self.input_listeners.retain(|(lid, _)| *lid != id);
    }

    fn query_cell_size(&mut self) {
        use crate::terminal_image::get_capabilities;
        if !get_capabilities().images.is_some() {
            return;
        }
        self.cell_size_query_pending = true;
        self.terminal.write("\x1b[16t");
    }

    /// Stop the TUI.
    pub fn stop(&mut self) {
        self.stopped = true;
        if !self.previous_lines.is_empty() {
            let target_row = self.previous_lines.len() as i32;
            let line_diff = target_row - self.hardware_cursor_row;
            if line_diff > 0 {
                self.terminal.write(&format!("\x1b[{line_diff}B"));
            } else if line_diff < 0 {
                self.terminal.write(&format!("\x1b[{}A", -line_diff));
            }
            self.terminal.write("\r\n");
        }
        self.terminal.show_cursor();
        self.terminal.stop();
    }

    /// Request a render on the next tick.
    pub fn request_render(&mut self, force: bool) {
        if force {
            self.previous_lines.clear();
            self.previous_width = -1;
            self.previous_height = -1;
            self.cursor_row = 0;
            self.hardware_cursor_row = 0;
            self.max_lines_rendered = 0;
            self.previous_viewport_top = 0;
        }
        self.render_requested = true;
    }

    /// Handle terminal input.
    pub fn handle_input(&mut self, data: &str) {
        // Process input listeners
        if !self.input_listeners.is_empty() {
            let mut current = data.to_string();
            let ids: Vec<u64> = self.input_listeners.iter().map(|(id, _)| *id).collect();
            for id in ids {
                if let Some((_, listener)) = self.input_listeners.iter().find(|(lid, _)| *lid == id) {
                    match listener(&current) {
                        Some((true, _)) => return, // consumed
                        Some((false, Some(new_data))) => current = new_data,
                        _ => {}
                    }
                }
            }
            if current.is_empty() {
                return;
            }
            // Fall through with possibly transformed data
        }

        // Cell size query buffering
        if self.cell_size_query_pending {
            self.input_buffer.push_str(data);
            let filtered = self.parse_cell_size_response();
            if filtered.is_empty() {
                return;
            }
            // Continue with filtered data
        }

        // Global debug key
        use crate::keys::matches_key;
        if matches_key(data, "shift+ctrl+d") {
            if let Some(cb) = self.on_debug.clone() {
                cb();
            }
            return;
        }

        // If focused component is an overlay, verify it's still visible.
        if let Some(focused_id) = self.overlay_focused_id {
            if let Some(entry) = self.overlay_stack.iter().find(|e| e.id == focused_id) {
                if !self.is_overlay_visible(entry) {
                    // Focused overlay is no longer visible — redirect focus.
                    let top_id = self
                        .overlay_stack
                        .iter()
                        .rev()
                        .find(|e| e.id != focused_id && self.is_overlay_visible(e))
                        .map(|e| e.id);
                    if let Some(new_id) = top_id {
                        self.overlay_focused_id = Some(new_id);
                        if let Some(e) = self.overlay_stack.iter_mut().find(|e| e.id == new_id) {
                            e.overlay_had_focus = true;
                        }
                    } else {
                        self.overlay_focused_id = None;
                        // Fall back to pre_focus_idx of the (now hidden) overlay.
                        let pre = self
                            .overlay_stack
                            .iter()
                            .find(|e| e.id == focused_id)
                            .and_then(|e| e.pre_focus_idx);
                        self.focused_component_idx = pre;
                    }
                }
            }
        }

        // Filter out key release events unless the focused component opts in,
        // then forward to the focused component.
        if is_key_release(data) {
            // Check if focused component wants key releases.
            let wants_release = if let Some(focused_id) = self.overlay_focused_id {
                self.overlay_stack
                    .iter()
                    .find(|e| e.id == focused_id)
                    .map(|e| e.component.wants_key_release())
                    .unwrap_or(false)
            } else if let Some(idx) = self.focused_component_idx {
                self.container
                    .children
                    .get(idx)
                    .map(|c| c.wants_key_release())
                    .unwrap_or(false)
            } else {
                false
            };
            if !wants_release {
                return;
            }
        }

        // Forward to focused component.
        if let Some(focused_id) = self.overlay_focused_id {
            // Find the overlay by id and call handle_input.
            if let Some(entry) = self.overlay_stack.iter_mut().find(|e| e.id == focused_id) {
                entry.component.handle_input(data);
                self.request_render(false);
            }
        } else if let Some(idx) = self.focused_component_idx {
            if let Some(child) = self.container.children.get_mut(idx) {
                child.handle_input(data);
                self.request_render(false);
            }
        }
    }

    fn parse_cell_size_response(&mut self) -> String {
        let response_re = regex::Regex::new(r"\x1b\[6;(\d+);(\d+)t").unwrap();
        if let Some(caps) = response_re.captures(&self.input_buffer.clone()) {
            let height_px: u32 = caps[1].parse().unwrap_or(0);
            let width_px: u32 = caps[2].parse().unwrap_or(0);
            if height_px > 0 && width_px > 0 {
                use crate::terminal_image::{set_cell_dimensions, CellDimensions};
                set_cell_dimensions(CellDimensions { width_px, height_px });
                self.invalidate();
                self.request_render(false);
            }
            self.input_buffer = response_re.replace(&self.input_buffer.clone(), "").to_string();
            self.cell_size_query_pending = false;
        }

        let partial_re = regex::Regex::new(r"\x1b(\[6?;?[\d;]*)?$").unwrap();
        if partial_re.is_match(&self.input_buffer) {
            let last_char = self.input_buffer.chars().last().unwrap_or('\0');
            if !last_char.is_ascii_alphabetic() && last_char != '~' {
                return String::new();
            }
        }

        let result = self.input_buffer.clone();
        self.input_buffer.clear();
        self.cell_size_query_pending = false;
        result
    }

    fn resolve_overlay_layout(
        options: &Option<OverlayOptions>,
        overlay_height: u16,
        term_width: u16,
        term_height: u16,
    ) -> (u16, u16, u16, Option<u16>) {
        // Returns (width, row, col, max_height)
        let opt = options.as_ref();
        let margin = opt
            .and_then(|o| o.margin.as_ref())
            .cloned()
            .unwrap_or_default();
        let margin_top = margin.top;
        let margin_right = margin.right;
        let margin_bottom = margin.bottom;
        let margin_left = margin.left;

        let avail_width = term_width.saturating_sub(margin_left + margin_right).max(1);
        let avail_height = term_height.saturating_sub(margin_top + margin_bottom).max(1);

        let mut width = opt
            .and_then(|o| o.width.as_ref())
            .map(|sv| sv.resolve(term_width))
            .unwrap_or(avail_width.min(80));
        if let Some(min_w) = opt.and_then(|o| o.min_width) {
            width = width.max(min_w);
        }
        width = width.min(avail_width).max(1);

        let max_height = opt
            .and_then(|o| o.max_height.as_ref())
            .map(|sv| sv.resolve(term_height).min(avail_height).max(1));

        let effective_height = max_height.map(|mh| overlay_height.min(mh)).unwrap_or(overlay_height);

        let anchor = opt.and_then(|o| o.anchor).unwrap_or(OverlayAnchor::Center);

        let row = if let Some(sv) = opt.and_then(|o| o.row.as_ref()) {
            match sv {
                SizeValue::Percent(p) => {
                    let max_row = avail_height.saturating_sub(effective_height) as f64;
                    margin_top + (max_row * p / 100.0).floor() as u16
                }
                SizeValue::Absolute(v) => *v,
            }
        } else {
            margin_top + Self::resolve_anchor_row(anchor, effective_height, avail_height)
        };

        let col = if let Some(sv) = opt.and_then(|o| o.col.as_ref()) {
            match sv {
                SizeValue::Percent(p) => {
                    let max_col = avail_width.saturating_sub(width) as f64;
                    margin_left + (max_col * p / 100.0).floor() as u16
                }
                SizeValue::Absolute(v) => *v,
            }
        } else {
            margin_left + Self::resolve_anchor_col(anchor, width, avail_width)
        };

        let offset_y = opt.and_then(|o| o.offset_y).unwrap_or(0);
        let offset_x = opt.and_then(|o| o.offset_x).unwrap_or(0);

        let row = (row as i32 + offset_y as i32)
            .clamp(margin_top as i32, (term_height.saturating_sub(margin_bottom + effective_height)) as i32)
            as u16;
        let col = (col as i32 + offset_x as i32)
            .clamp(margin_left as i32, (term_width.saturating_sub(margin_right + width)) as i32)
            as u16;

        (width, row, col, max_height)
    }

    fn resolve_anchor_row(anchor: OverlayAnchor, height: u16, avail_height: u16) -> u16 {
        match anchor {
            OverlayAnchor::TopLeft | OverlayAnchor::TopCenter | OverlayAnchor::TopRight => 0,
            OverlayAnchor::BottomLeft | OverlayAnchor::BottomCenter | OverlayAnchor::BottomRight => {
                avail_height.saturating_sub(height)
            }
            OverlayAnchor::LeftCenter | OverlayAnchor::Center | OverlayAnchor::RightCenter => {
                (avail_height.saturating_sub(height)) / 2
            }
        }
    }

    fn resolve_anchor_col(anchor: OverlayAnchor, width: u16, avail_width: u16) -> u16 {
        match anchor {
            OverlayAnchor::TopLeft | OverlayAnchor::LeftCenter | OverlayAnchor::BottomLeft => 0,
            OverlayAnchor::TopRight | OverlayAnchor::RightCenter | OverlayAnchor::BottomRight => {
                avail_width.saturating_sub(width)
            }
            OverlayAnchor::TopCenter | OverlayAnchor::Center | OverlayAnchor::BottomCenter => {
                (avail_width.saturating_sub(width)) / 2
            }
        }
    }

    /// Composite all overlays into content lines.
    fn composite_overlays(
        &self,
        mut lines: Vec<String>,
        term_width: u16,
        term_height: u16,
    ) -> Vec<String> {
        if self.overlay_stack.is_empty() {
            return lines;
        }

        // Pre-render all visible overlays
        let mut rendered: Vec<(Vec<String>, u16, u16, u16)> = Vec::new(); // (lines, row, col, w)
        let mut min_lines_needed = lines.len();

        let mut visible_entries: Vec<&OverlayEntry> = self
            .overlay_stack
            .iter()
            .filter(|e| self.is_overlay_visible(e))
            .collect();
        visible_entries.sort_by_key(|e| e.focus_order);

        for entry in &visible_entries {
            let (width, _, _, max_height) =
                Self::resolve_overlay_layout(&entry.options, 0, term_width, term_height);
            let mut overlay_lines = entry.component.render(width);
            if let Some(mh) = max_height {
                overlay_lines.truncate(mh as usize);
            }
            let (_, row, col, _) = Self::resolve_overlay_layout(
                &entry.options,
                overlay_lines.len() as u16,
                term_width,
                term_height,
            );
            min_lines_needed = min_lines_needed.max(row as usize + overlay_lines.len());
            rendered.push((overlay_lines, row, col, width));
        }

        let working_height = self.max_lines_rendered.max(min_lines_needed);
        while lines.len() < working_height {
            lines.push(String::new());
        }

        let viewport_start = working_height.saturating_sub(term_height as usize);

        for (overlay_lines, row, col, w) in &rendered {
            for (i, ol) in overlay_lines.iter().enumerate() {
                let idx = viewport_start + *row as usize + i;
                if idx < lines.len() {
                    let truncated_ol = if visible_width(ol) > *w as usize {
                        slice_by_column(ol, 0, *w as usize, true)
                    } else {
                        ol.clone()
                    };
                    lines[idx] = Self::composite_line_at(
                        &lines[idx],
                        &truncated_ol,
                        *col as usize,
                        *w as usize,
                        term_width as usize,
                    );
                }
            }
        }

        lines
    }

    const SEGMENT_RESET: &'static str = "\x1b[0m\x1b]8;;\x07";

    fn apply_line_resets(lines: Vec<String>) -> Vec<String> {
        let reset = Self::SEGMENT_RESET;
        lines
            .into_iter()
            .map(|line| {
                if !is_image_line(&line) {
                    format!("{line}{reset}")
                } else {
                    line
                }
            })
            .collect()
    }

    fn composite_line_at(
        base_line: &str,
        overlay_line: &str,
        start_col: usize,
        overlay_width: usize,
        total_width: usize,
    ) -> String {
        if is_image_line(base_line) {
            return base_line.to_string();
        }

        let after_start = start_col + overlay_width;
        let (before, before_w, after, after_w) =
            extract_segments(base_line, start_col, after_start, total_width.saturating_sub(after_start), true);

        let (overlay_text, overlay_w) = slice_with_width(overlay_line, 0, overlay_width, true);

        let before_pad = start_col.saturating_sub(before_w);
        let overlay_pad = overlay_width.saturating_sub(overlay_w);
        let actual_before_w = start_col.max(before_w);
        let actual_overlay_w = overlay_width.max(overlay_w);
        let after_target = total_width.saturating_sub(actual_before_w + actual_overlay_w);
        let after_pad = after_target.saturating_sub(after_w);

        let r = Self::SEGMENT_RESET;
        let result = format!(
            "{before}{}{r}{overlay_text}{}{r}{after}{}",
            " ".repeat(before_pad),
            " ".repeat(overlay_pad),
            " ".repeat(after_pad),
        );

        let rw = visible_width(&result);
        if rw <= total_width {
            result
        } else {
            slice_by_column(&result, 0, total_width, true)
        }
    }

    /// Find and extract cursor position from rendered lines.
    fn extract_cursor_position(lines: &mut Vec<String>, height: u16) -> Option<(usize, usize)> {
        let viewport_top = lines.len().saturating_sub(height as usize);
        for row in (viewport_top..lines.len()).rev() {
            let line = &lines[row];
            if let Some(marker_idx) = line.find(CURSOR_MARKER) {
                let before_marker = &line[..marker_idx];
                let col = visible_width(before_marker);
                let after_marker = line[marker_idx + CURSOR_MARKER.len()..].to_string();
                lines[row] = format!("{before_marker}{after_marker}");
                return Some((row, col));
            }
        }
        None
    }

    /// Perform a render pass. Should be called when render_requested is set.
    pub fn do_render(&mut self) {
        if self.stopped {
            return;
        }
        let width = self.terminal.columns();
        let height = self.terminal.rows();

        let width_changed = self.previous_width != 0 && self.previous_width != width as i32;
        let height_changed = self.previous_height != 0 && self.previous_height != height as i32;

        // Render all components
        let mut new_lines = self.render(width);

        // Composite overlays
        if !self.overlay_stack.is_empty() {
            new_lines = self.composite_overlays(new_lines, width, height);
        }

        // Extract cursor position
        let cursor_pos = Self::extract_cursor_position(&mut new_lines, height);

        // Apply line resets
        let new_lines = Self::apply_line_resets(new_lines);

        let full_render = |term: &dyn Terminal,
                           new_lines: &[String],
                           clear: bool,
                           full_redraw_count: &mut u64,
                           max_lines_rendered: &mut usize,
                           hardware_cursor_row: &mut i32| {
            *full_redraw_count += 1;
            let mut buffer = "\x1b[?2026h".to_string(); // Begin synchronized output
            if clear {
                buffer.push_str("\x1b[2J\x1b[H\x1b[3J");
            }
            for (i, line) in new_lines.iter().enumerate() {
                if i > 0 {
                    buffer.push_str("\r\n");
                }
                buffer.push_str(line);
            }
            buffer.push_str("\x1b[?2026l");
            term.write(&buffer);
            *hardware_cursor_row = new_lines.len().saturating_sub(1) as i32;
            if clear {
                *max_lines_rendered = new_lines.len();
            } else {
                *max_lines_rendered = (*max_lines_rendered).max(new_lines.len());
            }
        };

        // First render
        if self.previous_lines.is_empty() && !width_changed && !height_changed {
            full_render(
                self.terminal.as_ref(),
                &new_lines,
                false,
                &mut self.full_redraw_count,
                &mut self.max_lines_rendered,
                &mut self.hardware_cursor_row,
            );
            self.cursor_row = new_lines.len().saturating_sub(1) as i32;
            let buffer_length = (height as usize).max(new_lines.len());
            self.previous_viewport_top = buffer_length.saturating_sub(height as usize);
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_lines = new_lines;
            self.previous_width = width as i32;
            self.previous_height = height as i32;
            self.render_requested = false;
            return;
        }

        // Width or height changes need full re-render
        if width_changed || height_changed {
            full_render(
                self.terminal.as_ref(),
                &new_lines,
                true,
                &mut self.full_redraw_count,
                &mut self.max_lines_rendered,
                &mut self.hardware_cursor_row,
            );
            self.cursor_row = new_lines.len().saturating_sub(1) as i32;
            let buffer_length = (height as usize).max(new_lines.len());
            self.previous_viewport_top = buffer_length.saturating_sub(height as usize);
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_lines = new_lines;
            self.previous_width = width as i32;
            self.previous_height = height as i32;
            self.render_requested = false;
            return;
        }

        // Content shrunk
        if self.clear_on_shrink
            && new_lines.len() < self.max_lines_rendered
            && self.overlay_stack.is_empty()
        {
            full_render(
                self.terminal.as_ref(),
                &new_lines,
                true,
                &mut self.full_redraw_count,
                &mut self.max_lines_rendered,
                &mut self.hardware_cursor_row,
            );
            self.cursor_row = new_lines.len().saturating_sub(1) as i32;
            let buffer_length = (height as usize).max(new_lines.len());
            self.previous_viewport_top = buffer_length.saturating_sub(height as usize);
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_lines = new_lines;
            self.previous_width = width as i32;
            self.previous_height = height as i32;
            self.render_requested = false;
            return;
        }

        // Shrink to empty — just clear and return
        if new_lines.is_empty() {
            full_render(
                self.terminal.as_ref(),
                &new_lines,
                true,
                &mut self.full_redraw_count,
                &mut self.max_lines_rendered,
                &mut self.hardware_cursor_row,
            );
            self.cursor_row = 0;
            self.previous_viewport_top = 0;
            self.position_hardware_cursor(cursor_pos, 0);
            self.previous_lines = new_lines;
            self.previous_width = width as i32;
            self.previous_height = height as i32;
            self.render_requested = false;
            return;
        }

        // Differential rendering
        let prev_viewport_top = self.previous_viewport_top;
        let max_lines = new_lines.len().max(self.previous_lines.len());
        let mut first_changed: i32 = -1;
        let mut last_changed: i32 = -1;

        for i in 0..max_lines {
            let old_line = self.previous_lines.get(i).map(|s| s.as_str()).unwrap_or("");
            let new_line = new_lines.get(i).map(|s| s.as_str()).unwrap_or("");
            if old_line != new_line {
                if first_changed == -1 {
                    first_changed = i as i32;
                }
                last_changed = i as i32;
            }
        }

        if new_lines.len() > self.previous_lines.len() {
            if first_changed == -1 {
                first_changed = self.previous_lines.len() as i32;
            }
            last_changed = new_lines.len() as i32 - 1;
        }

        if first_changed == -1 {
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_viewport_top = prev_viewport_top;
            self.previous_height = height as i32;
            self.render_requested = false;
            return;
        }

        // If first changed line is above viewport, full redraw.
        // Also trigger full redraw if shrinkage moves the viewport top upward
        // (i.e., new content is shorter than prev_viewport_top + height).
        let new_viewport_top =
            (height as usize).max(new_lines.len()).saturating_sub(height as usize);
        if first_changed < prev_viewport_top as i32 || new_viewport_top < prev_viewport_top {
            full_render(
                self.terminal.as_ref(),
                &new_lines,
                true,
                &mut self.full_redraw_count,
                &mut self.max_lines_rendered,
                &mut self.hardware_cursor_row,
            );
            self.cursor_row = new_lines.len().saturating_sub(1) as i32;
            let buffer_length = (height as usize).max(new_lines.len());
            self.previous_viewport_top = buffer_length.saturating_sub(height as usize);
            self.position_hardware_cursor(cursor_pos, new_lines.len());
            self.previous_lines = new_lines;
            self.previous_width = width as i32;
            self.previous_height = height as i32;
            self.render_requested = false;
            return;
        }

        // Differential update
        let mut buffer = "\x1b[?2026h".to_string();

        let line_diff = first_changed - self.hardware_cursor_row;
        if line_diff > 0 {
            buffer.push_str(&format!("\x1b[{line_diff}B"));
        } else if line_diff < 0 {
            buffer.push_str(&format!("\x1b[{}A", -line_diff));
        }
        buffer.push('\r');

        let render_end = last_changed.min(new_lines.len() as i32 - 1);
        for i in first_changed..=render_end {
            if i > first_changed {
                buffer.push_str("\r\n");
            }
            buffer.push_str("\x1b[2K");
            if let Some(line) = new_lines.get(i as usize) {
                buffer.push_str(line);
            }
        }

        let mut final_cursor_row = render_end;

        // Clear deleted lines
        if self.previous_lines.len() > new_lines.len() {
            if render_end < new_lines.len() as i32 - 1 {
                let move_down = new_lines.len() as i32 - 1 - render_end;
                buffer.push_str(&format!("\x1b[{move_down}B"));
                final_cursor_row = new_lines.len() as i32 - 1;
            }
            for _ in new_lines.len()..self.previous_lines.len() {
                buffer.push_str("\r\n\x1b[2K");
            }
            let extra_lines = self.previous_lines.len() - new_lines.len();
            buffer.push_str(&format!("\x1b[{extra_lines}A"));
        }

        buffer.push_str("\x1b[?2026l");
        self.terminal.write(&buffer);

        self.cursor_row = new_lines.len().saturating_sub(1) as i32;
        self.hardware_cursor_row = final_cursor_row;
        self.max_lines_rendered = self.max_lines_rendered.max(new_lines.len());
        self.previous_viewport_top =
            prev_viewport_top.max((final_cursor_row as usize + 1).saturating_sub(height as usize));

        self.position_hardware_cursor(cursor_pos, new_lines.len());

        self.previous_lines = new_lines;
        self.previous_width = width as i32;
        self.previous_height = height as i32;
        self.render_requested = false;
    }

    fn position_hardware_cursor(
        &mut self,
        cursor_pos: Option<(usize, usize)>,
        total_lines: usize,
    ) {
        if cursor_pos.is_none() || total_lines == 0 {
            self.terminal.hide_cursor();
            return;
        }

        let (target_row, target_col) = cursor_pos.unwrap();
        let target_row = target_row.min(total_lines.saturating_sub(1)) as i32;

        let row_delta = target_row - self.hardware_cursor_row;
        let mut buf = String::new();
        if row_delta > 0 {
            buf.push_str(&format!("\x1b[{row_delta}B"));
        } else if row_delta < 0 {
            buf.push_str(&format!("\x1b[{}A", -row_delta));
        }
        buf.push_str(&format!("\x1b[{}G", target_col + 1));

        if !buf.is_empty() {
            self.terminal.write(&buf);
        }

        self.hardware_cursor_row = target_row;
        if self.show_hardware_cursor {
            self.terminal.show_cursor();
        } else {
            self.terminal.hide_cursor();
        }
    }

    fn is_overlay_visible(&self, entry: &OverlayEntry) -> bool {
        if entry.hidden {
            return false;
        }
        if let Some(ref vis_fn) = entry.options.as_ref().and_then(|o| o.visible.as_ref()) {
            return vis_fn(self.terminal.columns(), self.terminal.rows());
        }
        true
    }

    /// Check if there are any visible overlays.
    pub fn has_overlay(&self) -> bool {
        self.overlay_stack.iter().any(|e| self.is_overlay_visible(e))
    }

    // =========================================================================
    // Public overlay / focus API (mirrors pi-mono showOverlay / hideOverlay / setFocus)
    // =========================================================================

    /// Set focus to a container child by index (None = clear focus).
    ///
    /// Mirrors pi-mono `tui.setFocus(component)`.
    /// Use `None` to clear focus, or `Some(idx)` where `idx` is the child's
    /// position in the container (0-based).
    ///
    /// Calling this clears any overlay-based focus; the overlay continues to be
    /// displayed but input routing reverts to the container child.
    pub fn set_focus(&mut self, child_idx: Option<usize>) {
        // If an overlay currently holds focus, clear that.
        if let Some(focused_id) = self.overlay_focused_id.take() {
            if let Some(entry) = self.overlay_stack.iter_mut().find(|e| e.id == focused_id) {
                entry.overlay_had_focus = false;
                entry.component.invalidate();
            }
        }

        self.focused_component_idx = child_idx;
        self.request_render(false);
    }

    /// Add an overlay component on top of the base content.
    ///
    /// If `options.non_capturing` is false (default), focus is transferred to
    /// the overlay component. Returns an `OverlayHandle` that can be passed to
    /// `hide_overlay` to remove the overlay later.
    ///
    /// Mirrors pi-mono `tui.showOverlay(component, options)`.
    pub fn show_overlay(
        &mut self,
        component: Box<dyn Component>,
        options: OverlayOptions,
    ) -> OverlayHandle {
        let id = self.next_overlay_id;
        self.next_overlay_id += 1;
        self.focus_order_counter += 1;
        let focus_order = self.focus_order_counter;
        let non_capturing = options.non_capturing;

        let entry = OverlayEntry {
            id,
            component,
            options: Some(options),
            pre_focus_idx: self.focused_component_idx,
            overlay_had_focus: false,
            hidden: false,
            focus_order,
        };
        self.overlay_stack.push(entry);

        // Transfer focus to overlay unless it is non-capturing.
        if !non_capturing {
            // Check if the overlay is actually visible (may be gated by `visible` fn).
            let is_visible = {
                let entry = self.overlay_stack.last().unwrap();
                self.is_overlay_visible(entry)
            };
            if is_visible {
                // Clear old container child focus.
                self.focused_component_idx = None;
                let entry = self.overlay_stack.last_mut().unwrap();
                entry.overlay_had_focus = true;
                self.overlay_focused_id = Some(id);
                self.terminal.hide_cursor();
            }
        }

        self.request_render(false);
        OverlayHandle(id)
    }

    /// Remove the overlay identified by `handle`.
    ///
    /// If the overlay currently holds focus, focus is restored to the component
    /// that had focus before the overlay was shown.
    ///
    /// Mirrors pi-mono `tui.hideOverlay()` (but accepts a specific handle rather
    /// than always popping the topmost overlay, to be handle-safe).
    pub fn hide_overlay(&mut self, handle: OverlayHandle) {
        let id = handle.0;
        let idx = self.overlay_stack.iter().position(|e| e.id == id);
        if let Some(pos) = idx {
            let entry = self.overlay_stack.remove(pos);
            // Restore focus if this overlay held it.
            if self.overlay_focused_id == Some(id) {
                self.overlay_focused_id = None;
                // Find the topmost visible non-removed overlay, or fall back to pre_focus_idx.
                let topmost = self.overlay_stack.iter().rev().find(|e| self.is_overlay_visible(e));
                if let Some(top) = topmost {
                    let top_id = top.id;
                    self.overlay_focused_id = Some(top_id);
                    if let Some(top_entry) = self.overlay_stack.iter_mut().find(|e| e.id == top_id) {
                        top_entry.overlay_had_focus = true;
                    }
                } else {
                    // No overlay left with focus — restore container child focus.
                    self.focused_component_idx = entry.pre_focus_idx;
                }
            }
            if self.overlay_stack.is_empty() {
                self.terminal.hide_cursor();
            }
        }
        self.request_render(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::MockTerminal;
    use std::sync::{Arc, Mutex};

    fn make_tui() -> TUI {
        TUI::new(Box::new(MockTerminal::new(80, 24)))
    }

    struct TextComponent(String);

    impl Component for TextComponent {
        fn render(&self, width: u16) -> Vec<String> {
            vec![self.0.clone()]
        }
    }

    /// A test component whose lines can be swapped out after construction.
    struct DynLinesComponent {
        lines: Arc<Mutex<Vec<String>>>,
    }

    impl DynLinesComponent {
        fn new(lines: Vec<String>) -> (Self, Arc<Mutex<Vec<String>>>) {
            let shared = Arc::new(Mutex::new(lines));
            (Self { lines: Arc::clone(&shared) }, shared)
        }
    }

    impl Component for DynLinesComponent {
        fn render(&self, _width: u16) -> Vec<String> {
            self.lines.lock().unwrap().clone()
        }
    }

    /// A resizable mock terminal for render tests.
    struct ResizableMock {
        cols: Arc<Mutex<u16>>,
        rows: Arc<Mutex<u16>>,
        written: Arc<Mutex<Vec<String>>>,
    }

    impl ResizableMock {
        fn new(cols: u16, rows: u16) -> (Self, Arc<Mutex<u16>>, Arc<Mutex<u16>>) {
            let c = Arc::new(Mutex::new(cols));
            let r = Arc::new(Mutex::new(rows));
            let mock = Self {
                cols: Arc::clone(&c),
                rows: Arc::clone(&r),
                written: Arc::new(Mutex::new(Vec::new())),
            };
            (mock, c, r)
        }

        fn all_written(&self) -> String {
            self.written.lock().unwrap().join("")
        }
    }

    impl Terminal for ResizableMock {
        fn start(
            &mut self,
            _on_input: Box<dyn Fn(String) + Send + 'static>,
            _on_resize: Box<dyn Fn() + Send + 'static>,
        ) {
        }
        fn stop(&mut self) {}
        fn write(&self, data: &str) {
            self.written.lock().unwrap().push(data.to_string());
        }
        fn columns(&self) -> u16 {
            *self.cols.lock().unwrap()
        }
        fn rows(&self) -> u16 {
            *self.rows.lock().unwrap()
        }
        fn kitty_protocol_active(&self) -> bool {
            false
        }
        fn move_by(&self, _lines: i32) {}
        fn hide_cursor(&self) {}
        fn show_cursor(&self) {}
        fn clear_line(&self) {}
        fn clear_from_cursor(&self) {}
        fn clear_screen(&self) {}
        fn set_title(&self, _title: &str) {}
    }

    /// Strip ANSI escape sequences from a string for plain-text comparison.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Skip ESC sequences
                match chars.peek() {
                    Some('[') => {
                        chars.next(); // consume '['
                        // consume until a letter (final byte)
                        loop {
                            match chars.next() {
                                Some(c) if c.is_ascii_alphabetic() => break,
                                None => break,
                                _ => {}
                            }
                        }
                    }
                    Some(']') => {
                        chars.next(); // consume ']'
                        // consume until BEL (\x07) or ST (\x1b\\)
                        loop {
                            match chars.next() {
                                Some('\x07') | None => break,
                                Some('\x1b') => {
                                    chars.next(); // consume '\\'
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Some('_') => {
                        chars.next(); // consume '_'
                        // APC — consume until BEL
                        loop {
                            match chars.next() {
                                Some('\x07') | None => break,
                                _ => {}
                            }
                        }
                    }
                    _ => {} // bare ESC, skip
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Get the visible viewport from previous_lines (last `rows` lines), stripping ANSI.
    fn viewport(tui: &TUI) -> Vec<String> {
        let rows = tui.terminal.rows() as usize;
        let lines = &tui.previous_lines;
        let start = lines.len().saturating_sub(rows);
        lines[start..].iter().map(|l| strip_ansi(l)).collect()
    }

    #[test]
    fn test_container_render() {
        let mut tui = make_tui();
        tui.add_child(Box::new(TextComponent("hello".to_string())));
        tui.add_child(Box::new(TextComponent("world".to_string())));
        let lines = tui.render(80);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "hello");
        assert_eq!(lines[1], "world");
    }

    #[test]
    fn test_cursor_marker_extraction() {
        let mut lines = vec![
            format!("hello {}world", CURSOR_MARKER),
            "second line".to_string(),
        ];
        let pos = TUI::extract_cursor_position(&mut lines, 24);
        // "hello " visible width is 6 (5 letters + 1 space)
        assert_eq!(pos, Some((0, 6)));
        // Marker should be stripped
        assert!(!lines[0].contains(CURSOR_MARKER));
    }

    #[test]
    fn test_anchor_resolution() {
        // Center anchor
        assert_eq!(TUI::resolve_anchor_row(OverlayAnchor::Center, 10, 24), 7);
        assert_eq!(TUI::resolve_anchor_col(OverlayAnchor::Center, 40, 80), 20);

        // Top-left anchor
        assert_eq!(TUI::resolve_anchor_row(OverlayAnchor::TopLeft, 10, 24), 0);
        assert_eq!(TUI::resolve_anchor_col(OverlayAnchor::TopLeft, 40, 80), 0);

        // Bottom-right anchor
        assert_eq!(TUI::resolve_anchor_row(OverlayAnchor::BottomRight, 10, 24), 14);
        assert_eq!(TUI::resolve_anchor_col(OverlayAnchor::BottomRight, 40, 80), 40);
    }

    #[test]
    fn test_do_render_first() {
        let mut tui = make_tui();
        tui.add_child(Box::new(TextComponent("hello".to_string())));
        tui.render_requested = true;
        tui.do_render();
        // Should have produced output
        assert!(!tui.previous_lines.is_empty());
    }

    #[test]
    fn test_has_overlay_false() {
        let tui = make_tui();
        assert!(!tui.has_overlay());
    }

    #[test]
    fn test_set_show_hardware_cursor() {
        let mut tui = make_tui();
        tui.set_show_hardware_cursor(true);
        assert!(tui.get_show_hardware_cursor());
        tui.set_show_hardware_cursor(false);
        assert!(!tui.get_show_hardware_cursor());
    }

    // =========================================================================
    // TUI resize handling tests (ported from tui-render.test.ts)
    // =========================================================================

    #[test]
    fn test_do_render_full_redraw_on_height_change() {
        let (mock, _cols, rows_lock) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));
        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        // Initial render
        tui.start();
        tui.do_render();
        let initial_redraws = tui.full_redraws();

        // Resize height — simulate by changing the terminal dimensions
        *rows_lock.lock().unwrap() = 15;
        // do_render sees height changed → full redraw
        tui.do_render();

        assert!(
            tui.full_redraws() > initial_redraws,
            "Height change should trigger full redraw"
        );

        // Content should still be present
        let vp = viewport(&tui);
        assert!(vp.iter().any(|l| l.contains("Line 0")), "Content preserved after height change");
    }

    #[test]
    fn test_do_render_full_redraw_on_width_change() {
        let (mock, cols_lock, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));
        let (comp, _lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();
        let initial_redraws = tui.full_redraws();

        // Resize width
        *cols_lock.lock().unwrap() = 60;
        tui.do_render();

        assert!(
            tui.full_redraws() > initial_redraws,
            "Width change should trigger full redraw"
        );
    }

    // =========================================================================
    // TUI content shrinkage tests
    // =========================================================================

    #[test]
    fn test_do_render_clear_on_shrink() {
        let (mock, _cols, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));
        tui.set_clear_on_shrink(true);

        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
            "Line 4".to_string(),
            "Line 5".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();
        let initial_redraws = tui.full_redraws();

        // Shrink to fewer lines
        *lines_ref.lock().unwrap() = vec!["Line 0".to_string(), "Line 1".to_string()];
        tui.request_render(false);
        tui.do_render();

        assert!(
            tui.full_redraws() > initial_redraws,
            "Content shrinkage should trigger full redraw (clear_on_shrink=true)"
        );

        // Content should still contain the remaining lines
        let vp = viewport(&tui);
        assert!(vp.iter().any(|l| l.contains("Line 0")), "Line 0 preserved");
        assert!(vp.iter().any(|l| l.contains("Line 1")), "Line 1 preserved");
    }

    #[test]
    fn test_do_render_shrink_to_single_line() {
        let (mock, _cols, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));
        tui.set_clear_on_shrink(true);

        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();

        *lines_ref.lock().unwrap() = vec!["Only line".to_string()];
        tui.request_render(false);
        tui.do_render();

        let vp = viewport(&tui);
        assert!(vp.iter().any(|l| l.contains("Only line")), "Single line rendered");
        // The rendered buffer should only have 1 line now
        assert_eq!(tui.previous_lines.len(), 1);
    }

    #[test]
    fn test_do_render_shrink_to_empty() {
        let (mock, _cols, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));
        tui.set_clear_on_shrink(true);

        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();

        *lines_ref.lock().unwrap() = vec![];
        tui.request_render(false);
        tui.do_render();

        // previous_lines should be empty after shrinking to nothing
        assert!(tui.previous_lines.is_empty());
    }

    // =========================================================================
    // TUI differential rendering tests
    // =========================================================================

    #[test]
    fn test_do_render_spinner_only_middle_line_changes() {
        let (mock, _cols, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));

        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Header".to_string(),
            "Working...".to_string(),
            "Footer".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();

        for frame in ["|", "/", "-", "\\"] {
            *lines_ref.lock().unwrap() = vec![
                "Header".to_string(),
                format!("Working {frame}"),
                "Footer".to_string(),
            ];
            tui.request_render(false);
            tui.do_render();

            let vp = viewport(&tui);
            assert!(vp[0].contains("Header"), "Header preserved: {:?}", vp[0]);
            assert!(vp[1].contains(&format!("Working {frame}")), "Spinner updated: {:?}", vp[1]);
            assert!(vp[2].contains("Footer"), "Footer preserved: {:?}", vp[2]);
        }
    }

    #[test]
    fn test_do_render_first_line_changes() {
        let (mock, _cols, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));

        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();

        *lines_ref.lock().unwrap() = vec![
            "CHANGED".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
        ];
        tui.request_render(false);
        tui.do_render();

        let vp = viewport(&tui);
        assert!(vp[0].contains("CHANGED"), "First line changed: {:?}", vp[0]);
        assert!(vp[1].contains("Line 1"), "Line 1 preserved: {:?}", vp[1]);
        assert!(vp[2].contains("Line 2"), "Line 2 preserved: {:?}", vp[2]);
        assert!(vp[3].contains("Line 3"), "Line 3 preserved: {:?}", vp[3]);
    }

    #[test]
    fn test_do_render_last_line_changes() {
        let (mock, _cols, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));

        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();

        *lines_ref.lock().unwrap() = vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
            "CHANGED".to_string(),
        ];
        tui.request_render(false);
        tui.do_render();

        let vp = viewport(&tui);
        assert!(vp[0].contains("Line 0"), "Line 0 preserved");
        assert!(vp[1].contains("Line 1"), "Line 1 preserved");
        assert!(vp[2].contains("Line 2"), "Line 2 preserved");
        assert!(vp[3].contains("CHANGED"), "Last line changed: {:?}", vp[3]);
    }

    #[test]
    fn test_do_render_multiple_non_adjacent_lines_change() {
        let (mock, _cols, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));

        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
            "Line 4".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();

        // Change lines 1 and 3
        *lines_ref.lock().unwrap() = vec![
            "Line 0".to_string(),
            "CHANGED 1".to_string(),
            "Line 2".to_string(),
            "CHANGED 3".to_string(),
            "Line 4".to_string(),
        ];
        tui.request_render(false);
        tui.do_render();

        let vp = viewport(&tui);
        assert!(vp[0].contains("Line 0"), "Line 0 preserved");
        assert!(vp[1].contains("CHANGED 1"), "Line 1 changed");
        assert!(vp[2].contains("Line 2"), "Line 2 preserved");
        assert!(vp[3].contains("CHANGED 3"), "Line 3 changed");
        assert!(vp[4].contains("Line 4"), "Line 4 preserved");
    }

    #[test]
    fn test_do_render_transition_empty_to_content_and_back() {
        let (mock, _cols, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));

        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();

        // Verify initial
        let vp = viewport(&tui);
        assert!(vp.iter().any(|l| l.contains("Line 0")), "Initial content rendered");

        // Clear to empty
        *lines_ref.lock().unwrap() = vec![];
        tui.request_render(false);
        tui.do_render();

        // Add content back
        *lines_ref.lock().unwrap() = vec!["New Line 0".to_string(), "New Line 1".to_string()];
        tui.request_render(false);
        tui.do_render();

        let vp = viewport(&tui);
        assert!(vp.iter().any(|l| l.contains("New Line 0")), "New content rendered: {:?}", vp);
        assert!(vp.iter().any(|l| l.contains("New Line 1")), "New content line 1: {:?}", vp);
    }

    #[test]
    fn test_do_render_shrink_viewport_triggers_full_redraw() {
        // content that fills more than the viewport forces a scroll, then
        // shrinking so the old viewport_top would be above the new content
        // should trigger a full redraw.
        let (mock, _cols, _rows) = ResizableMock::new(20, 5);
        let mut tui = TUI::new(Box::new(mock));

        let (comp, lines_ref) = DynLinesComponent::new(
            (0..12).map(|i| format!("Line {i}")).collect(),
        );
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();
        let initial_redraws = tui.full_redraws();

        // Shrink from 12 lines to 7 — the first changed line (index 7) is above
        // the previous viewport top (12 - 5 = 7), so it triggers a full redraw.
        *lines_ref.lock().unwrap() = (0..7).map(|i| format!("Line {i}")).collect();
        tui.request_render(false);
        tui.do_render();

        assert!(
            tui.full_redraws() > initial_redraws,
            "Shrink should trigger a full redraw"
        );

        // The viewport should now show lines 2–6 (last 5 of 7 lines)
        let vp = viewport(&tui);
        assert!(vp[0].contains("Line 2"), "Viewport[0] = {:?}", vp[0]);
        assert!(vp[4].contains("Line 6"), "Viewport[4] = {:?}", vp[4]);
    }

    #[test]
    fn test_do_render_cursor_tracking_after_shrink() {
        // After shrinking, subsequent differential renders should correctly
        // track which line is changed.
        let (mock, _cols, _rows) = ResizableMock::new(40, 10);
        let mut tui = TUI::new(Box::new(mock));

        let (comp, lines_ref) = DynLinesComponent::new(vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
            "Line 4".to_string(),
        ]);
        tui.add_child(Box::new(comp));

        tui.start();
        tui.do_render();

        // Shrink to 3 identical lines
        *lines_ref.lock().unwrap() = vec![
            "Line 0".to_string(),
            "Line 1".to_string(),
            "Line 2".to_string(),
        ];
        tui.request_render(false);
        tui.do_render();

        // Now change line 1 — should render correctly
        *lines_ref.lock().unwrap() = vec![
            "Line 0".to_string(),
            "CHANGED".to_string(),
            "Line 2".to_string(),
        ];
        tui.request_render(false);
        tui.do_render();

        let vp = viewport(&tui);
        assert!(
            vp[1].contains("CHANGED"),
            "Expected CHANGED on line 1, got: {:?}",
            vp[1]
        );
    }

    #[test]
    fn test_do_render_apply_line_resets() {
        // apply_line_resets should append SEGMENT_RESET after each non-image line
        let lines = vec!["Hello".to_string(), "World".to_string()];
        let reset_lines = TUI::apply_line_resets(lines);
        for line in &reset_lines {
            assert!(
                line.ends_with(TUI::SEGMENT_RESET),
                "Line should end with SEGMENT_RESET: {:?}",
                line
            );
        }
    }
}

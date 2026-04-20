/// Editor component — multi-line text editor.
///
/// Mirrors pi-mono `components/editor.ts`. pi-mono's implementation is 2170 lines
/// and includes bracketed paste, paste markers, autocomplete SelectList integration,
/// kill-ring yank/pop, history navigation, jump mode, and word-wrap layout cursor
/// arithmetic. This Rust port implements the core multi-line editing surface
/// (text buffer, cursor movement, insertion/deletion, submit/change callbacks)
/// plus hooks for the optional features exposed by `EditorComponent`. Advanced
/// pi-mono features can be layered on top incrementally.
use crate::autocomplete::AutocompleteProvider;
use crate::editor_component::EditorComponent;
use crate::keybindings::get_keybindings;
use crate::keys::decode_kitty_printable;
use crate::kill_ring::KillRing;
use crate::tui::{CURSOR_MARKER, Component, Focusable};
use crate::undo_stack::UndoStack;
use crate::utils::{is_punctuation_char, is_whitespace_char, visible_width, wrap_text_with_ansi};

pub type SubmitCallback = Box<dyn Fn(String) + Send + 'static>;
pub type ChangeCallback = Box<dyn Fn(String) + Send + 'static>;
pub type BorderColorFn = Box<dyn Fn(&str) -> String + Send + Sync>;

/// Theme for the editor — colour/border helpers.
pub struct EditorTheme {
    pub border_color: BorderColorFn,
}

impl EditorTheme {
    pub fn new<F>(border_color: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self {
            border_color: Box::new(border_color),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EditorOptions {
    pub padding_x: Option<u16>,
    pub autocomplete_max_visible: Option<usize>,
}

#[derive(Clone, Default)]
struct EditorState {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastAction {
    Kill,
    Yank,
    TypeWord,
}

/// Jump-mode state: waiting for user to type a target character.
#[derive(Debug, Clone, PartialEq, Eq)]
enum JumpMode {
    Forward,
    Backward,
}

/// Multi-line text editor.
#[allow(dead_code)]
pub struct Editor {
    state: EditorState,
    focused: bool,
    theme: EditorTheme,
    padding_x: u16,
    last_width: u16,
    scroll_offset: usize,

    on_submit: Option<SubmitCallback>,
    on_change: Option<ChangeCallback>,

    autocomplete_provider: Option<Box<dyn AutocompleteProvider>>,
    autocomplete_max_visible: usize,

    history: Vec<String>,
    history_index: Option<usize>,

    kill_ring: KillRing,
    last_action: Option<LastAction>,

    undo_stack: UndoStack<EditorState>,

    paste_buffer: String,
    in_paste: bool,

    jump_mode: Option<JumpMode>,

    pub disable_submit: bool,
}

impl Editor {
    pub fn new(theme: EditorTheme, options: EditorOptions) -> Self {
        let padding_x = options.padding_x.unwrap_or(0);
        let autocomplete_max_visible = options.autocomplete_max_visible.unwrap_or(5).clamp(3, 20);

        Self {
            state: EditorState {
                lines: vec![String::new()],
                cursor_line: 0,
                cursor_col: 0,
            },
            focused: false,
            theme,
            padding_x,
            last_width: 80,
            scroll_offset: 0,
            on_submit: None,
            on_change: None,
            autocomplete_provider: None,
            autocomplete_max_visible,
            history: Vec::new(),
            history_index: None,
            kill_ring: KillRing::new(),
            last_action: None,
            undo_stack: UndoStack::new(),
            paste_buffer: String::new(),
            in_paste: false,
            jump_mode: None,
            disable_submit: false,
        }
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(&self.state);
    }

    fn emit_change(&self) {
        if let Some(ref cb) = self.on_change {
            cb(self.render_text());
        }
    }

    fn emit_submit(&self) {
        if let Some(ref cb) = self.on_submit {
            cb(self.render_text());
        }
    }

    fn render_text(&self) -> String {
        self.state.lines.join("\n")
    }

    fn current_line(&self) -> &str {
        &self.state.lines[self.state.cursor_line]
    }

    fn cursor_byte(&self) -> usize {
        self.state
            .lines
            .get(self.state.cursor_line)
            .map(|l| byte_offset_for_col(l, self.state.cursor_col))
            .unwrap_or(0)
    }

    fn handle_newline(&mut self) {
        self.push_undo();
        self.insert_newline_raw();
        self.emit_change();
    }

    /// Insert a newline without pushing an undo snapshot (for atomic operations).
    fn insert_newline_raw(&mut self) {
        let line_idx = self.state.cursor_line;
        let byte = self.cursor_byte();
        let line = &self.state.lines[line_idx];
        let (left, right) = line.split_at(byte);
        let left = left.to_string();
        let right = right.to_string();
        self.state.lines[line_idx] = left;
        self.state.lines.insert(line_idx + 1, right);
        self.state.cursor_line += 1;
        self.state.cursor_col = 0;
        self.last_action = None;
    }

    /// Insert text without pushing undo snapshots (for atomic paste/insert operations).
    fn insert_text_raw(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if text.chars().any(|c| c == '\n') {
            for (i, chunk) in text.split('\n').enumerate() {
                if i > 0 {
                    self.insert_newline_raw();
                }
                if !chunk.is_empty() {
                    let first_ch_ws = chunk.chars().next().is_some_and(is_whitespace_char);
                    let line_idx = self.state.cursor_line;
                    let byte = self.cursor_byte();
                    let line = &mut self.state.lines[line_idx];
                    line.insert_str(byte, chunk);
                    let col_delta = chunk.chars().count();
                    self.state.cursor_col += col_delta;
                    let _ = first_ch_ws; // suppress unused warning
                }
            }
            return;
        }
        let line_idx = self.state.cursor_line;
        let byte = self.cursor_byte();
        let line = &mut self.state.lines[line_idx];
        line.insert_str(byte, text);
        self.state.cursor_col += text.chars().count();
    }

    fn handle_submit(&mut self) {
        if self.disable_submit {
            return;
        }
        // Backslash+Enter workaround: if cursor is immediately after a backslash,
        // remove the backslash and insert a newline instead of submitting.
        let line_idx = self.state.cursor_line;
        let col = self.state.cursor_col;
        let line_chars: Vec<char> = self.state.lines[line_idx].chars().collect();
        if col > 0 && line_chars.get(col - 1) == Some(&'\\') {
            // Remove the backslash and insert a newline
            self.push_undo();
            let mut chars = line_chars;
            chars.remove(col - 1);
            self.state.lines[line_idx] = chars.into_iter().collect();
            self.state.cursor_col -= 1;
            self.handle_newline();
            return;
        }
        let text = self.render_text();
        if !text.is_empty() {
            self.add_to_history(&text.clone());
        }
        self.undo_stack.clear();
        self.emit_submit();
    }

    fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if text.chars().any(|c| c == '\n') {
            for (i, chunk) in text.split('\n').enumerate() {
                if i > 0 {
                    self.handle_newline();
                }
                if !chunk.is_empty() {
                    self.insert_text_nobreak(chunk);
                }
            }
            return;
        }
        self.insert_text_nobreak(text);
    }

    fn insert_text_nobreak(&mut self, text: &str) {
        self.history_index = None; // Exit history browsing mode on any edit
        let first_ch_ws = text.chars().next().is_some_and(is_whitespace_char);
        if first_ch_ws || self.last_action != Some(LastAction::TypeWord) {
            self.push_undo();
        }
        self.last_action = Some(LastAction::TypeWord);

        let line_idx = self.state.cursor_line;
        let byte = self.cursor_byte();
        let line = &mut self.state.lines[line_idx];
        line.insert_str(byte, text);

        // Advance cursor by char count of inserted text.
        let col_delta = text.chars().count();
        self.state.cursor_col += col_delta;
        self.emit_change();
    }

    fn delete_char_backward(&mut self) {
        self.last_action = None;
        if self.state.cursor_col > 0 {
            self.push_undo();
            let line_idx = self.state.cursor_line;
            let col = self.state.cursor_col;
            let line = &self.state.lines[line_idx];
            // Compute the grapheme (here approximated as char) to remove.
            let mut chars: Vec<char> = line.chars().collect();
            chars.remove(col - 1);
            self.state.lines[line_idx] = chars.into_iter().collect();
            self.state.cursor_col -= 1;
            self.emit_change();
        } else if self.state.cursor_line > 0 {
            // Join with previous line.
            self.push_undo();
            let this_line = self.state.lines.remove(self.state.cursor_line);
            self.state.cursor_line -= 1;
            let prev = &mut self.state.lines[self.state.cursor_line];
            let prev_col = prev.chars().count();
            prev.push_str(&this_line);
            self.state.cursor_col = prev_col;
            self.emit_change();
        }
    }

    fn delete_char_forward(&mut self) {
        self.last_action = None;
        let line_idx = self.state.cursor_line;
        let col = self.state.cursor_col;
        let line_char_count = self.state.lines[line_idx].chars().count();
        if col < line_char_count {
            self.push_undo();
            let mut chars: Vec<char> = self.state.lines[line_idx].chars().collect();
            chars.remove(col);
            self.state.lines[line_idx] = chars.into_iter().collect();
            self.emit_change();
        } else if line_idx + 1 < self.state.lines.len() {
            // Join next line into this one.
            self.push_undo();
            let next = self.state.lines.remove(line_idx + 1);
            self.state.lines[line_idx].push_str(&next);
            self.emit_change();
        }
    }

    fn move_cursor_left(&mut self) {
        self.last_action = None;
        if self.state.cursor_col > 0 {
            self.state.cursor_col -= 1;
        } else if self.state.cursor_line > 0 {
            self.state.cursor_line -= 1;
            self.state.cursor_col = self.state.lines[self.state.cursor_line].chars().count();
        }
    }

    fn move_cursor_right(&mut self) {
        self.last_action = None;
        let line_char_count = self.current_line().chars().count();
        if self.state.cursor_col < line_char_count {
            self.state.cursor_col += 1;
        } else if self.state.cursor_line + 1 < self.state.lines.len() {
            self.state.cursor_line += 1;
            self.state.cursor_col = 0;
        }
    }

    fn move_cursor_line_start(&mut self) {
        self.last_action = None;
        self.state.cursor_col = 0;
    }

    fn move_cursor_line_end(&mut self) {
        self.last_action = None;
        self.state.cursor_col = self.current_line().chars().count();
    }

    fn move_cursor_up(&mut self) {
        self.last_action = None;
        if self.state.cursor_line > 0 {
            self.state.cursor_line -= 1;
            let line_char_count = self.current_line().chars().count();
            if self.state.cursor_col > line_char_count {
                self.state.cursor_col = line_char_count;
            }
        }
    }

    fn move_cursor_down(&mut self) {
        self.last_action = None;
        if self.state.cursor_line + 1 < self.state.lines.len() {
            self.state.cursor_line += 1;
            let line_char_count = self.current_line().chars().count();
            if self.state.cursor_col > line_char_count {
                self.state.cursor_col = line_char_count;
            }
        }
    }

    /// Returns true if the editor has a single empty line (truly empty).
    fn is_editor_empty(&self) -> bool {
        self.state.lines.len() == 1 && self.state.lines[0].is_empty()
    }

    /// Returns true if cursor is on the first logical line (line index 0).
    fn is_on_first_line(&self) -> bool {
        self.state.cursor_line == 0
    }

    /// Returns true if cursor is on the last logical line.
    fn is_on_last_line(&self) -> bool {
        self.state.cursor_line + 1 >= self.state.lines.len()
    }

    /// Navigate history. direction = -1 means go to older entries (Up arrow),
    /// direction = 1 means go to newer entries (Down arrow).
    ///
    /// `history` stores entries oldest-first (push appends newest at end).
    /// `history_index` = Some(i) means we are currently showing history[i].
    /// `history_index` = None means we are not in history mode.
    ///
    /// Up from None → Some(history.len()-1) [most recent]
    /// Up from Some(i) where i > 0 → Some(i-1) [older]
    /// Up from Some(0) → stays at Some(0) [already at oldest]
    /// Down from Some(i) where i < history.len()-1 → Some(i+1) [newer]
    /// Down from Some(history.len()-1) → None [exit history, clear editor]
    fn navigate_history(&mut self, direction: i32) {
        self.last_action = None;
        if self.history.is_empty() {
            return;
        }
        let new_index = match self.history_index {
            None => {
                if direction < 0 {
                    // Up from non-history mode → go to most recent
                    Some(self.history.len() - 1)
                } else {
                    // Down from non-history mode → no-op
                    return;
                }
            }
            Some(current) => {
                if direction < 0 {
                    // Up → older
                    if current == 0 {
                        // Already at oldest, stay
                        Some(0)
                    } else {
                        Some(current - 1)
                    }
                } else {
                    // Down → newer
                    if current + 1 >= self.history.len() {
                        // Exit history
                        None
                    } else {
                        Some(current + 1)
                    }
                }
            }
        };

        self.history_index = new_index;

        match new_index {
            None => {
                // Exited history — clear editor
                self.state.lines = vec![String::new()];
                self.state.cursor_line = 0;
                self.state.cursor_col = 0;
                self.emit_change();
            }
            Some(idx) => {
                let text = self.history[idx].clone();
                let lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
                let lines = if lines.is_empty() {
                    vec![String::new()]
                } else {
                    lines
                };
                self.state.cursor_line = lines.len() - 1;
                self.state.cursor_col = lines[self.state.cursor_line].chars().count();
                self.state.lines = lines;
                self.emit_change();
            }
        }
    }

    fn move_word_left(&mut self) {
        self.last_action = None;
        if self.state.cursor_col == 0 {
            if self.state.cursor_line == 0 {
                return;
            }
            self.state.cursor_line -= 1;
            self.state.cursor_col = self.current_line().chars().count();
            return;
        }
        let line_chars: Vec<char> = self.current_line().chars().collect();
        let mut pos = self.state.cursor_col;

        while pos > 0 && is_whitespace_char(line_chars[pos - 1]) {
            pos -= 1;
        }
        if pos > 0 {
            if is_punctuation_char(line_chars[pos - 1]) {
                while pos > 0 && is_punctuation_char(line_chars[pos - 1]) {
                    pos -= 1;
                }
            } else {
                while pos > 0
                    && !is_whitespace_char(line_chars[pos - 1])
                    && !is_punctuation_char(line_chars[pos - 1])
                {
                    pos -= 1;
                }
            }
        }
        self.state.cursor_col = pos;
    }

    fn move_word_right(&mut self) {
        self.last_action = None;
        let line_chars: Vec<char> = self.current_line().chars().collect();
        if self.state.cursor_col >= line_chars.len() {
            if self.state.cursor_line + 1 >= self.state.lines.len() {
                return;
            }
            self.state.cursor_line += 1;
            self.state.cursor_col = 0;
            return;
        }
        let mut pos = self.state.cursor_col;
        while pos < line_chars.len() && is_whitespace_char(line_chars[pos]) {
            pos += 1;
        }
        if pos < line_chars.len() {
            if is_punctuation_char(line_chars[pos]) {
                while pos < line_chars.len() && is_punctuation_char(line_chars[pos]) {
                    pos += 1;
                }
            } else {
                while pos < line_chars.len()
                    && !is_whitespace_char(line_chars[pos])
                    && !is_punctuation_char(line_chars[pos])
                {
                    pos += 1;
                }
            }
        }
        self.state.cursor_col = pos;
    }

    fn delete_word_backward(&mut self) {
        // No-op guard: if at start of document, nothing to delete.
        if self.state.cursor_line == 0 && self.state.cursor_col == 0 {
            return;
        }
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();
        let old_line = self.state.cursor_line;
        let old_col = self.state.cursor_col;
        self.move_word_left();

        if self.state.cursor_line == old_line {
            let line_chars: Vec<char> = self.state.lines[old_line].chars().collect();
            let start = self.state.cursor_col;
            let deleted: String = line_chars[start..old_col].iter().collect();
            self.kill_ring.push(&deleted, true, was_kill);
            let remaining: String = line_chars[..start]
                .iter()
                .chain(line_chars[old_col..].iter())
                .collect();
            self.state.lines[old_line] = remaining;
        } else {
            // Simplification: delete across lines — join them.
            let chunk_above = self.state.lines[self.state.cursor_line][byte_offset_for_col(
                &self.state.lines[self.state.cursor_line],
                self.state.cursor_col,
            )..]
                .to_string();
            let removed_below = self
                .state
                .lines
                .drain(self.state.cursor_line + 1..=old_line)
                .collect::<Vec<_>>();
            let mut deleted_text = chunk_above.clone();
            for (i, l) in removed_below.iter().enumerate() {
                deleted_text.push('\n');
                if i == removed_below.len() - 1 {
                    let byte_old_col = byte_offset_for_col(l, old_col);
                    deleted_text.push_str(&l[..byte_old_col]);
                } else {
                    deleted_text.push_str(l);
                }
            }
            self.kill_ring.push(&deleted_text, true, was_kill);
            // Trim current line to cursor, and join the tail of the final removed line.
            let cur_byte = byte_offset_for_col(
                &self.state.lines[self.state.cursor_line],
                self.state.cursor_col,
            );
            self.state.lines[self.state.cursor_line].truncate(cur_byte);
            if let Some(last) = removed_below.last() {
                let byte_old_col = byte_offset_for_col(last, old_col);
                self.state.lines[self.state.cursor_line].push_str(&last[byte_old_col..]);
            }
        }
        self.last_action = Some(LastAction::Kill);
        self.emit_change();
    }

    fn delete_word_forward(&mut self) {
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();
        let old_line = self.state.cursor_line;
        let old_col = self.state.cursor_col;
        self.move_word_right();

        if self.state.cursor_line == old_line {
            let line_chars: Vec<char> = self.state.lines[old_line].chars().collect();
            let end = self.state.cursor_col;
            let deleted: String = line_chars[old_col..end].iter().collect();
            self.kill_ring.push(&deleted, false, was_kill);
            let remaining: String = line_chars[..old_col]
                .iter()
                .chain(line_chars[end..].iter())
                .collect();
            self.state.lines[old_line] = remaining;
            self.state.cursor_col = old_col;
        } else {
            // Simplification: delete across lines, collapsing them.
            let byte_old = byte_offset_for_col(&self.state.lines[old_line], old_col);
            let tail_on_orig = self.state.lines[old_line][byte_old..].to_string();
            let mut deleted_text = tail_on_orig.clone();
            let removed_middle = self
                .state
                .lines
                .drain((old_line + 1)..self.state.cursor_line)
                .collect::<Vec<_>>();
            for l in &removed_middle {
                deleted_text.push('\n');
                deleted_text.push_str(l);
            }
            let final_col = self.state.cursor_col;
            let final_line = self.state.lines.remove(old_line + 1);
            let byte_final_col = byte_offset_for_col(&final_line, final_col);
            deleted_text.push('\n');
            deleted_text.push_str(&final_line[..byte_final_col]);
            self.state.lines[old_line].truncate(byte_old);
            self.state.lines[old_line].push_str(&final_line[byte_final_col..]);
            self.state.cursor_line = old_line;
            self.state.cursor_col = old_col;
            self.kill_ring.push(&deleted_text, false, was_kill);
        }
        self.last_action = Some(LastAction::Kill);
        self.emit_change();
    }

    fn delete_to_line_end(&mut self) {
        let was_kill = self.last_action == Some(LastAction::Kill);
        let line_idx = self.state.cursor_line;
        let col = self.state.cursor_col;
        let line_char_count = self.state.lines[line_idx].chars().count();
        if col >= line_char_count {
            if line_idx + 1 < self.state.lines.len() {
                self.push_undo();
                let next = self.state.lines.remove(line_idx + 1);
                self.state.lines[line_idx].push_str(&next);
                self.kill_ring.push("\n", false, was_kill);
                self.last_action = Some(LastAction::Kill);
                self.emit_change();
            }
            return;
        }
        self.push_undo();
        let line_chars: Vec<char> = self.state.lines[line_idx].chars().collect();
        let deleted: String = line_chars[col..].iter().collect();
        let kept: String = line_chars[..col].iter().collect();
        self.state.lines[line_idx] = kept;
        self.kill_ring.push(&deleted, false, was_kill);
        self.last_action = Some(LastAction::Kill);
        self.emit_change();
    }

    fn delete_to_line_start(&mut self) {
        let was_kill = self.last_action == Some(LastAction::Kill);
        if self.state.cursor_col == 0 {
            // At line start: merge with previous line (delete the newline above cursor).
            if self.state.cursor_line == 0 {
                return;
            }
            self.push_undo();
            let this_line = self.state.lines.remove(self.state.cursor_line);
            self.state.cursor_line -= 1;
            let prev_len = self.state.lines[self.state.cursor_line].chars().count();
            self.state.lines[self.state.cursor_line].push_str(&this_line);
            self.state.cursor_col = prev_len;
            // We deleted a newline — record it as a backward kill.
            self.kill_ring.push("\n", true, was_kill);
            self.last_action = Some(LastAction::Kill);
            self.emit_change();
            return;
        }
        self.push_undo();
        let line_idx = self.state.cursor_line;
        let col = self.state.cursor_col;
        let line_chars: Vec<char> = self.state.lines[line_idx].chars().collect();
        let deleted: String = line_chars[..col].iter().collect();
        let kept: String = line_chars[col..].iter().collect();
        self.state.lines[line_idx] = kept;
        self.state.cursor_col = 0;
        self.kill_ring.push(&deleted, true, was_kill);
        self.last_action = Some(LastAction::Kill);
        self.emit_change();
    }

    fn yank(&mut self) {
        let text = match self.kill_ring.peek() {
            Some(s) => s.to_string(),
            None => return,
        };
        self.push_undo();
        self.insert_text(&text);
        self.last_action = Some(LastAction::Yank);
    }

    fn yank_pop(&mut self) {
        if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        self.push_undo();
        let prev = self.kill_ring.peek().unwrap_or("").to_string();
        // Remove previously inserted text: its char count determines how many
        // chars to strip on the current line (simplification: assume no newline).
        let to_remove = prev.chars().count();
        let line_idx = self.state.cursor_line;
        let line_chars: Vec<char> = self.state.lines[line_idx].chars().collect();
        let end = self.state.cursor_col;
        let start = end.saturating_sub(to_remove);
        let kept: String = line_chars[..start]
            .iter()
            .chain(line_chars[end..].iter())
            .collect();
        self.state.lines[line_idx] = kept;
        self.state.cursor_col = start;
        self.kill_ring.rotate();
        let text = self.kill_ring.peek().unwrap_or("").to_string();
        self.insert_text(&text);
        self.last_action = Some(LastAction::Yank);
    }

    fn undo(&mut self) {
        if let Some(snapshot) = self.undo_stack.pop() {
            self.state = snapshot;
            self.last_action = None;
            self.emit_change();
        }
    }

    fn handle_paste(&mut self, content: &str) {
        let cleaned = content
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ");
        self.push_undo();
        self.insert_text_raw(&cleaned);
        self.last_action = None;
        self.emit_change();
    }

    /// Jump forward to the next occurrence of `target_char` on the current line.
    ///
    /// Mirrors pi-mono's jump-mode: press Ctrl+] to enter forward-jump mode, then
    /// type a character to jump the cursor past the next occurrence of that char.
    fn jump_forward(&mut self, target: char) {
        let line_chars: Vec<char> = self.state.lines[self.state.cursor_line].chars().collect();
        let start = self.state.cursor_col + 1;
        if let Some(pos) = line_chars[start..].iter().position(|&c| c == target) {
            self.state.cursor_col = start + pos + 1;
        }
    }

    /// Jump backward to the previous occurrence of `target_char` on the current line.
    fn jump_backward(&mut self, target: char) {
        let line_chars: Vec<char> = self.state.lines[self.state.cursor_line].chars().collect();
        let end = self.state.cursor_col;
        if let Some(pos) = line_chars[..end].iter().rposition(|&c| c == target) {
            self.state.cursor_col = pos;
        }
    }
}

impl Focusable for Editor {
    fn focused(&self) -> bool {
        self.focused
    }
    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }
}

impl Component for Editor {
    fn render(&self, width: u16) -> Vec<String> {
        let content_width = (width as usize)
            .saturating_sub(self.padding_x as usize * 2)
            .max(1);
        let pad = " ".repeat(self.padding_x as usize);

        let mut output: Vec<String> = Vec::new();
        for (line_idx, line) in self.state.lines.iter().enumerate() {
            // Word-wrap line to content_width using ANSI-aware wrap.
            let wrapped = wrap_text_with_ansi(line, content_width);
            let wrapped = if wrapped.is_empty() {
                vec![String::new()]
            } else {
                wrapped
            };

            // Determine which visual chunk contains the cursor for this logical line.
            let mut cursor_on_line = None;
            if self.focused && line_idx == self.state.cursor_line {
                let mut remaining = self.state.cursor_col;
                for (wrap_idx, chunk) in wrapped.iter().enumerate() {
                    let chunk_char_count = chunk.chars().count();
                    if remaining <= chunk_char_count {
                        cursor_on_line = Some((wrap_idx, remaining));
                        break;
                    }
                    remaining -= chunk_char_count;
                }
                if cursor_on_line.is_none() {
                    cursor_on_line = Some((
                        wrapped.len() - 1,
                        wrapped.last().map(|c| c.chars().count()).unwrap_or(0),
                    ));
                }
            }

            for (wrap_idx, chunk) in wrapped.iter().enumerate() {
                let line_out = if let Some((cursor_wrap, cursor_col)) = cursor_on_line {
                    if cursor_wrap == wrap_idx {
                        insert_cursor_marker(chunk, cursor_col)
                    } else {
                        chunk.clone()
                    }
                } else {
                    chunk.clone()
                };
                let visible = visible_width(&line_out);
                let padded = if visible < width as usize {
                    format!(
                        "{pad}{line_out}{}",
                        " ".repeat(
                            (width as usize).saturating_sub(visible + self.padding_x as usize * 2)
                        )
                    )
                } else {
                    format!("{pad}{line_out}")
                };
                output.push(padded);
            }
        }
        output
    }

    fn handle_input(&mut self, data: &str) {
        self.last_width = self.last_width.max(1);

        // Bracketed paste handling.
        let mut data = data.to_string();
        if data.contains("\x1b[200~") {
            self.in_paste = true;
            self.paste_buffer.clear();
            data = data.replace("\x1b[200~", "");
        }
        if self.in_paste {
            self.paste_buffer.push_str(&data);
            if let Some(end_idx) = self.paste_buffer.find("\x1b[201~") {
                let paste_content: String = self.paste_buffer[..end_idx].to_string();
                self.handle_paste(&paste_content);
                let after_end = end_idx + "\x1b[201~".len();
                let remaining: String = self.paste_buffer[after_end..].to_string();
                self.paste_buffer.clear();
                self.in_paste = false;
                if !remaining.is_empty() {
                    <Self as Component>::handle_input(self, &remaining);
                }
            }
            return;
        }

        let kb = get_keybindings();

        if kb.matches(&data, "tui.editor.undo") {
            self.undo();
            return;
        }

        if kb.matches(&data, "tui.input.submit") || data == "\n" {
            self.handle_submit();
            return;
        }

        if kb.matches(&data, "tui.editor.newLine") {
            self.handle_newline();
            return;
        }

        if kb.matches(&data, "tui.editor.deleteCharBackward") {
            self.delete_char_backward();
            return;
        }

        if kb.matches(&data, "tui.editor.deleteCharForward") {
            self.delete_char_forward();
            return;
        }

        if kb.matches(&data, "tui.editor.deleteWordBackward") {
            self.delete_word_backward();
            return;
        }

        if kb.matches(&data, "tui.editor.deleteWordForward") {
            self.delete_word_forward();
            return;
        }

        if kb.matches(&data, "tui.editor.deleteToLineStart") {
            self.delete_to_line_start();
            return;
        }

        if kb.matches(&data, "tui.editor.deleteToLineEnd") {
            self.delete_to_line_end();
            return;
        }

        if kb.matches(&data, "tui.editor.yank") {
            self.yank();
            return;
        }
        if kb.matches(&data, "tui.editor.yankPop") {
            self.yank_pop();
            return;
        }

        if kb.matches(&data, "tui.editor.cursorLeft") {
            self.move_cursor_left();
            return;
        }
        if kb.matches(&data, "tui.editor.cursorRight") {
            self.move_cursor_right();
            return;
        }
        if kb.matches(&data, "tui.editor.cursorLineStart") {
            self.move_cursor_line_start();
            return;
        }
        if kb.matches(&data, "tui.editor.cursorLineEnd") {
            self.move_cursor_line_end();
            return;
        }
        if kb.matches(&data, "tui.editor.cursorUp") {
            // History navigation: enter history if empty or already in history
            // and cursor is on the first visual line.
            #[allow(clippy::if_same_then_else)]
            if self.is_editor_empty() {
                self.navigate_history(-1);
            } else if self.history_index.is_some() && self.is_on_first_line() {
                self.navigate_history(-1);
            } else if self.is_on_first_line() {
                // Already at top — jump to start of line (mirrors pi-mono behavior).
                self.move_cursor_line_start();
            } else {
                self.move_cursor_up();
            }
            return;
        }
        if kb.matches(&data, "tui.editor.cursorDown") {
            if self.history_index.is_some() && self.is_on_last_line() {
                self.navigate_history(1);
            } else if self.is_on_last_line() {
                // Already at bottom — jump to end of line (mirrors pi-mono behavior).
                self.move_cursor_line_end();
            } else {
                self.move_cursor_down();
            }
            return;
        }
        if kb.matches(&data, "tui.editor.cursorWordLeft") {
            self.move_word_left();
            return;
        }
        if kb.matches(&data, "tui.editor.cursorWordRight") {
            self.move_word_right();
            return;
        }

        // Jump mode — enter with Ctrl+] (forward) or Ctrl+Alt+] (backward).
        if kb.matches(&data, "tui.editor.jumpForward") {
            self.jump_mode = Some(JumpMode::Forward);
            return;
        }
        if kb.matches(&data, "tui.editor.jumpBackward") {
            self.jump_mode = Some(JumpMode::Backward);
            return;
        }
        // If in jump mode, consume the next printable character as the target.
        if let Some(mode) = self.jump_mode.take() {
            if let Some(ch) = data.chars().find(|c| !c.is_control()) {
                match mode {
                    JumpMode::Forward => self.jump_forward(ch),
                    JumpMode::Backward => self.jump_backward(ch),
                }
            }
            return;
        }

        // Kitty CSI-u printable character.
        if let Some(ch) = decode_kitty_printable(&data) {
            let s = ch.to_string();
            self.insert_text_nobreak(&s);
            return;
        }

        // Accept printable input (reject control chars).
        let has_control = data.chars().any(|c| {
            let code = c as u32;
            code < 32 || code == 0x7f || (0x80..=0x9f).contains(&code)
        });
        if !has_control {
            self.insert_text(&data);
        }
    }

    fn invalidate(&mut self) {}
}

impl EditorComponent for Editor {
    fn get_text(&self) -> String {
        self.render_text()
    }

    fn set_text(&mut self, text: &str) {
        // Push undo so the caller can undo programmatic setText calls.
        self.push_undo();
        let lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
        self.state.cursor_line = lines.len() - 1;
        self.state.cursor_col = lines[self.state.cursor_line].chars().count();
        self.state.lines = lines;
        self.history_index = None; // exit history browsing mode
        self.last_action = None;
        self.emit_change();
    }

    fn handle_input(&mut self, data: &str) {
        <Self as Component>::handle_input(self, data);
    }

    fn set_on_submit(&mut self, cb: SubmitCallback) {
        self.on_submit = Some(cb);
    }

    fn set_on_change(&mut self, cb: ChangeCallback) {
        self.on_change = Some(cb);
    }

    fn add_to_history(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        // Do not add consecutive duplicates.
        if self.history.last().map(|s| s.as_str()) == Some(text) {
            return;
        }
        // Limit history to 100 entries.
        if self.history.len() >= 100 {
            self.history.remove(0);
        }
        self.history.push(text.to_string());
    }

    fn insert_text_at_cursor(&mut self, text: &str) {
        // Normalize CRLF and CR line endings, then push undo + insert atomically.
        let cleaned = text.replace("\r\n", "\n").replace('\r', "\n");
        self.push_undo();
        self.insert_text_raw(&cleaned);
        self.last_action = None;
        self.emit_change();
    }

    fn get_expanded_text(&self) -> String {
        self.render_text()
    }

    fn set_autocomplete_provider(&mut self, provider: Box<dyn AutocompleteProvider>) {
        self.autocomplete_provider = Some(provider);
    }

    fn set_padding_x(&mut self, padding: u16) {
        self.padding_x = padding;
    }

    fn set_autocomplete_max_visible(&mut self, max_visible: usize) {
        self.autocomplete_max_visible = max_visible.clamp(3, 20);
    }
}

/// Compute the byte offset inside `line` corresponding to the given character-based column.
fn byte_offset_for_col(line: &str, col: usize) -> usize {
    let mut byte = 0;
    for (i, ch) in line.chars().enumerate() {
        if i == col {
            return byte;
        }
        byte += ch.len_utf8();
    }
    line.len()
}

/// Insert a CURSOR_MARKER and reverse-video cursor character at the given char column
/// of `chunk`. Returns the decorated string.
fn insert_cursor_marker(chunk: &str, cursor_col: usize) -> String {
    let chars: Vec<char> = chunk.chars().collect();
    let before: String = chars.iter().take(cursor_col).collect();
    let at = chars.get(cursor_col).copied().unwrap_or(' ');
    let after: String = chars.iter().skip(cursor_col + 1).collect();
    let at_str = at.to_string();
    format!("{before}{CURSOR_MARKER}\x1b[7m{at_str}\x1b[27m{after}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        Editor::new(
            EditorTheme::new(|s| s.to_string()),
            EditorOptions::default(),
        )
    }

    #[test]
    fn test_editor_set_text_get_text() {
        let mut e = editor();
        e.set_text("hello\nworld");
        assert_eq!(e.get_text(), "hello\nworld");
    }

    #[test]
    fn test_editor_type_chars() {
        let mut e = editor();
        e.insert_text("hi");
        assert_eq!(e.get_text(), "hi");
    }

    #[test]
    fn test_editor_newline() {
        let mut e = editor();
        e.insert_text("hello");
        e.handle_newline();
        e.insert_text("world");
        assert_eq!(e.get_text(), "hello\nworld");
    }

    #[test]
    fn test_editor_delete_backward() {
        let mut e = editor();
        e.insert_text("hello");
        e.delete_char_backward();
        assert_eq!(e.get_text(), "hell");
    }

    #[test]
    fn test_editor_cursor_up_down() {
        let mut e = editor();
        e.set_text("line1\nline2\nline3");
        // Cursor is at end of line3
        e.move_cursor_up();
        assert_eq!(e.state.cursor_line, 1);
        e.move_cursor_up();
        assert_eq!(e.state.cursor_line, 0);
        e.move_cursor_down();
        assert_eq!(e.state.cursor_line, 1);
    }

    #[test]
    fn test_editor_line_start_end() {
        let mut e = editor();
        e.insert_text("hello world");
        e.move_cursor_line_start();
        assert_eq!(e.state.cursor_col, 0);
        e.move_cursor_line_end();
        assert_eq!(e.state.cursor_col, 11);
    }

    #[test]
    fn test_editor_word_left_right() {
        let mut e = editor();
        e.insert_text("hello world foo");
        e.move_cursor_line_start();
        e.move_word_right();
        assert_eq!(e.state.cursor_col, 5);
        e.move_word_right();
        assert_eq!(e.state.cursor_col, 11);
    }

    #[test]
    fn test_editor_undo() {
        let mut e = editor();
        e.insert_text("foo");
        e.insert_text(" bar");
        let text_before_undo = e.get_text();
        assert_eq!(text_before_undo, "foo bar");
        e.undo();
        // After undo, should revert the most recent insertion chunk.
        assert_ne!(e.get_text(), text_before_undo);
    }

    #[test]
    fn test_editor_handle_paste() {
        let mut e = editor();
        e.handle_paste("hello\tworld\r\nnext");
        assert_eq!(e.get_text(), "hello    world\nnext");
    }

    // =========================================================================
    // Tests from editor.test.ts – History navigation
    // =========================================================================

    #[test]
    fn test_editor_history_empty_up_does_nothing() {
        let mut e = editor();
        // Up arrow when no history
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        assert_eq!(e.get_text(), "");
    }

    #[test]
    fn test_editor_history_up_shows_most_recent() {
        let mut e = editor();
        e.add_to_history("first prompt");
        e.add_to_history("second prompt");
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        assert_eq!(e.get_text(), "second prompt");
    }

    #[test]
    fn test_editor_history_cycles_on_repeated_up() {
        let mut e = editor();
        e.add_to_history("first");
        e.add_to_history("second");
        e.add_to_history("third");

        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        assert_eq!(e.get_text(), "third");
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        assert_eq!(e.get_text(), "second");
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        assert_eq!(e.get_text(), "first");
        // Stays at oldest
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        assert_eq!(e.get_text(), "first");
    }

    #[test]
    fn test_editor_history_down_clears_after_up() {
        let mut e = editor();
        e.add_to_history("prompt");
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        assert_eq!(e.get_text(), "prompt");
        <Editor as Component>::handle_input(&mut e, "\x1b[B");
        assert_eq!(e.get_text(), "");
    }

    #[test]
    fn test_editor_history_navigate_forward() {
        let mut e = editor();
        e.add_to_history("first");
        e.add_to_history("second");
        e.add_to_history("third");

        // Go to oldest
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        <Editor as Component>::handle_input(&mut e, "\x1b[A");

        // Navigate back
        <Editor as Component>::handle_input(&mut e, "\x1b[B");
        assert_eq!(e.get_text(), "second");
        <Editor as Component>::handle_input(&mut e, "\x1b[B");
        assert_eq!(e.get_text(), "third");
        <Editor as Component>::handle_input(&mut e, "\x1b[B");
        assert_eq!(e.get_text(), "");
    }

    #[test]
    fn test_editor_history_exit_on_type() {
        let mut e = editor();
        e.add_to_history("old prompt");
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        <Editor as Component>::handle_input(&mut e, "x");
        assert_eq!(e.get_text(), "old promptx");
    }

    #[test]
    fn test_editor_history_no_empty_strings() {
        let mut e = editor();
        e.add_to_history("");
        e.add_to_history("   ");
        e.add_to_history("valid");

        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        assert_eq!(e.get_text(), "valid");
        // No more entries
        <Editor as Component>::handle_input(&mut e, "\x1b[A");
        assert_eq!(e.get_text(), "valid");
    }

    // =========================================================================
    // Tests from editor.test.ts – Unicode text editing
    // =========================================================================

    #[test]
    fn test_editor_insert_unicode_chars() {
        let mut e = editor();
        for ch in ["H", "e", "l", "l", "o", " ", "ä", "ö", "ü", " ", "😀"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        assert_eq!(e.get_text(), "Hello äöü 😀");
    }

    #[test]
    fn test_editor_backspace_umlauts() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "ä");
        <Editor as Component>::handle_input(&mut e, "ö");
        <Editor as Component>::handle_input(&mut e, "ü");
        <Editor as Component>::handle_input(&mut e, "\x7f"); // Backspace
        assert_eq!(e.get_text(), "äö");
    }

    #[test]
    fn test_editor_backspace_emoji() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "😀");
        <Editor as Component>::handle_input(&mut e, "👍");
        <Editor as Component>::handle_input(&mut e, "\x7f"); // Backspace
        assert_eq!(e.get_text(), "😀");
    }

    #[test]
    fn test_editor_set_text_unicode_paste() {
        let mut e = editor();
        e.set_text("Hällö Wörld! 😀 äöüÄÖÜß");
        assert_eq!(e.get_text(), "Hällö Wörld! 😀 äöüÄÖÜß");
    }

    #[test]
    fn test_editor_ctrl_a_moves_to_start() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "a");
        <Editor as Component>::handle_input(&mut e, "b");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        <Editor as Component>::handle_input(&mut e, "x");
        assert_eq!(e.get_text(), "xab");
    }

    #[test]
    fn test_editor_ctrl_w_delete_word() {
        let mut e = editor();
        e.set_text("foo bar baz");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        assert_eq!(e.get_text(), "foo bar ");

        e.set_text("foo bar   ");
        <Editor as Component>::handle_input(&mut e, "\x17");
        assert_eq!(e.get_text(), "foo ");

        e.set_text("foo bar...");
        <Editor as Component>::handle_input(&mut e, "\x17");
        assert_eq!(e.get_text(), "foo bar");
    }

    #[test]
    fn test_editor_kitty_csi_u_unsupported_modifier_ignored() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "\x1b[99;9u"); // Super modifier - unsupported
        assert_eq!(e.get_text(), "");
    }

    // =========================================================================
    // Tests from editor.test.ts – Backslash+Enter newline workaround
    // =========================================================================

    #[test]
    fn test_editor_backslash_not_buffered() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "\\");
        assert_eq!(e.get_text(), "\\");
    }

    // =========================================================================
    // Tests from editor.test.ts – Kill ring
    // =========================================================================

    #[test]
    fn test_editor_ctrl_w_yank() {
        let mut e = editor();
        e.set_text("foo bar baz");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - deletes "baz"
        assert_eq!(e.get_text(), "foo bar ");

        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "bazfoo bar ");
    }

    #[test]
    fn test_editor_ctrl_k_saves_to_kill_ring() {
        let mut e = editor();
        e.set_text("hello world");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        <Editor as Component>::handle_input(&mut e, "\x0b"); // Ctrl+K
        assert_eq!(e.get_text(), "");
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "hello world");
    }

    #[test]
    fn test_editor_ctrl_y_does_nothing_when_ring_empty() {
        let mut e = editor();
        e.set_text("test");
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "test");
    }

    #[test]
    fn test_editor_consecutive_ctrl_w_accumulates() {
        let mut e = editor();
        e.set_text("one two three");
        <Editor as Component>::handle_input(&mut e, "\x17"); // deletes "three"
        <Editor as Component>::handle_input(&mut e, "\x17"); // deletes "two " (prepended)
        <Editor as Component>::handle_input(&mut e, "\x17"); // deletes "one " (prepended)
        assert_eq!(e.get_text(), "");

        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "one two three");
    }

    #[test]
    fn test_editor_alt_d_delete_word_forward() {
        let mut e = editor();
        e.set_text("hello world test");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A

        <Editor as Component>::handle_input(&mut e, "\x1bd"); // Alt+D - deletes "hello"
        assert_eq!(e.get_text(), " world test");

        <Editor as Component>::handle_input(&mut e, "\x1bd"); // Alt+D - deletes " world"
        assert_eq!(e.get_text(), " test");

        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "hello world test");
    }

    // =========================================================================
    // Tests from editor.test.ts – Undo
    // =========================================================================

    #[test]
    fn test_editor_undo_does_nothing_on_empty_stack() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "\x1f"); // Ctrl+- (undo)
        assert_eq!(e.get_text(), "");
    }

    #[test]
    fn test_editor_undo_coalesces_word_chars() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        assert_eq!(e.get_text(), "hello");

        <Editor as Component>::handle_input(&mut e, "\x1f"); // Ctrl+- (undo)
        // After undo, "hello" should be removed or partially removed
        let text = e.get_text();
        assert_ne!(text, "hello");
    }

    #[test]
    fn test_editor_bracketed_paste() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "\x1b[200~hello\tworld\r\nnext\x1b[201~");
        assert_eq!(e.get_text(), "hello    world\nnext");
    }

    #[test]
    fn test_editor_multiline_input() {
        let mut e = editor();
        e.set_text("line1\nline2\nline3");
        assert_eq!(e.get_text(), "line1\nline2\nline3");
        assert_eq!(e.state.cursor_line, 2);
    }

    #[test]
    fn test_editor_cursor_line_start_end_ctrl() {
        let mut e = editor();
        e.set_text("hello world");
        // Cursor at end after set_text
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A - line start
        assert_eq!(e.state.cursor_col, 0);
        <Editor as Component>::handle_input(&mut e, "\x05"); // Ctrl+E - line end
        assert_eq!(e.state.cursor_col, 11);
    }

    #[test]
    fn test_editor_delete_forward_ctrl_d() {
        let mut e = editor();
        e.set_text("hello");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        <Editor as Component>::handle_input(&mut e, "\x04"); // Ctrl+D - delete forward
        assert_eq!(e.get_text(), "ello");
    }

    #[test]
    fn test_editor_ctrl_u_delete_to_line_start() {
        let mut e = editor();
        e.set_text("hello world");
        // Cursor at end
        <Editor as Component>::handle_input(&mut e, "\x15"); // Ctrl+U
        assert_eq!(e.get_text(), "");
    }

    #[test]
    fn test_editor_get_lines_and_cursor() {
        let mut e = editor();
        e.set_text("a\nb");
        let lines = e.state.lines.clone();
        assert_eq!(lines, vec!["a", "b"]);
        assert_eq!(e.state.cursor_line, 1);
    }

    // =========================================================================
    // Tests from editor.test.ts – public state accessors
    // =========================================================================

    #[test]
    fn test_editor_get_cursor_initial() {
        let e = editor();
        assert_eq!(e.state.cursor_line, 0);
        assert_eq!(e.state.cursor_col, 0);
    }

    #[test]
    fn test_editor_cursor_advances_on_type() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "a");
        <Editor as Component>::handle_input(&mut e, "b");
        <Editor as Component>::handle_input(&mut e, "c");
        assert_eq!(e.state.cursor_col, 3);
    }

    #[test]
    fn test_editor_cursor_left_moves_back() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "a");
        <Editor as Component>::handle_input(&mut e, "b");
        <Editor as Component>::handle_input(&mut e, "c");
        <Editor as Component>::handle_input(&mut e, "\x1b[D"); // Left
        assert_eq!(e.state.cursor_col, 2);
    }

    #[test]
    fn test_editor_get_lines_defensive_copy() {
        let mut e = editor();
        e.set_text("a\nb");
        let lines_a = e.state.lines.clone();
        let lines_b = e.state.lines.clone();
        // Mutations of the copy should not affect state
        let mut mutable = lines_a.clone();
        mutable[0] = "mutated".to_string();
        assert_eq!(lines_b[0], "a");
        let _ = lines_a;
    }

    // =========================================================================
    // Tests from editor.test.ts – Unicode text editing (cursor movement)
    // =========================================================================

    #[test]
    fn test_editor_insert_at_correct_pos_after_cursor_move_over_umlauts() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "ä");
        <Editor as Component>::handle_input(&mut e, "ö");
        <Editor as Component>::handle_input(&mut e, "ü");
        <Editor as Component>::handle_input(&mut e, "\x1b[D"); // Left
        <Editor as Component>::handle_input(&mut e, "\x1b[D"); // Left
        <Editor as Component>::handle_input(&mut e, "x");
        assert_eq!(e.get_text(), "äxöü");
    }

    #[test]
    fn test_editor_move_cursor_over_emojis() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "😀");
        <Editor as Component>::handle_input(&mut e, "👍");
        <Editor as Component>::handle_input(&mut e, "🎉");
        <Editor as Component>::handle_input(&mut e, "\x1b[D"); // Left (over 🎉)
        <Editor as Component>::handle_input(&mut e, "\x1b[D"); // Left (over 👍)
        <Editor as Component>::handle_input(&mut e, "x");
        assert_eq!(e.get_text(), "😀x👍🎉");
    }

    #[test]
    fn test_editor_preserve_umlauts_across_line_breaks() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "ä");
        <Editor as Component>::handle_input(&mut e, "ö");
        <Editor as Component>::handle_input(&mut e, "ü");
        e.handle_newline();
        <Editor as Component>::handle_input(&mut e, "Ä");
        <Editor as Component>::handle_input(&mut e, "Ö");
        <Editor as Component>::handle_input(&mut e, "Ü");
        assert_eq!(e.get_text(), "äöü\nÄÖÜ");
    }

    #[test]
    fn test_editor_ctrl_w_delete_across_lines() {
        let mut e = editor();
        e.set_text("line one\nline two");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        assert_eq!(e.get_text(), "line one\nline ");
    }

    #[test]
    fn test_editor_ctrl_w_delete_empty_line_merge() {
        let mut e = editor();
        e.set_text("line one\n");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        assert_eq!(e.get_text(), "line one");
    }

    #[test]
    fn test_editor_ctrl_w_grapheme_emoji_word() {
        let mut e = editor();
        e.set_text("foo 😀😀 bar");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W deletes "bar"
        assert_eq!(e.get_text(), "foo 😀😀 ");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W deletes "😀😀 "
        assert_eq!(e.get_text(), "foo ");
    }

    #[test]
    fn test_editor_alt_backspace_delete_word() {
        let mut e = editor();
        e.set_text("foo bar");
        <Editor as Component>::handle_input(&mut e, "\x1b\x7f"); // Alt+Backspace
        assert_eq!(e.get_text(), "foo ");
    }

    #[test]
    fn test_editor_ctrl_left_right_word_navigation() {
        let mut e = editor();
        e.set_text("foo bar... baz");
        // cursor is at end after set_text
        <Editor as Component>::handle_input(&mut e, "\x1b[1;5D"); // Ctrl+Left
        assert_eq!(e.state.cursor_col, 11); // after '...'
        <Editor as Component>::handle_input(&mut e, "\x1b[1;5D"); // Ctrl+Left
        assert_eq!(e.state.cursor_col, 7); // after 'bar'
        <Editor as Component>::handle_input(&mut e, "\x1b[1;5D"); // Ctrl+Left
        assert_eq!(e.state.cursor_col, 4); // after 'foo '
        <Editor as Component>::handle_input(&mut e, "\x1b[1;5C"); // Ctrl+Right
        assert_eq!(e.state.cursor_col, 7); // end of 'bar'
        <Editor as Component>::handle_input(&mut e, "\x1b[1;5C"); // Ctrl+Right
        assert_eq!(e.state.cursor_col, 10); // after '...'
        <Editor as Component>::handle_input(&mut e, "\x1b[1;5C"); // Ctrl+Right
        assert_eq!(e.state.cursor_col, 14); // end of line
    }

    // =========================================================================
    // Tests from editor.test.ts – Kill ring (additional)
    // =========================================================================

    #[test]
    fn test_editor_ctrl_u_saves_to_kill_ring() {
        let mut e = editor();
        e.set_text("hello world");
        // Move to middle (after "hello ")
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..6 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        <Editor as Component>::handle_input(&mut e, "\x15"); // Ctrl+U
        assert_eq!(e.get_text(), "world");
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "hello world");
    }

    #[test]
    fn test_editor_alt_y_cycles_kill_ring() {
        let mut e = editor();
        e.set_text("first");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        e.set_text("second");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        e.set_text("third");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        assert_eq!(e.get_text(), "");

        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y - yanks "third"
        assert_eq!(e.get_text(), "third");
        <Editor as Component>::handle_input(&mut e, "\x1by"); // Alt+Y - "second"
        assert_eq!(e.get_text(), "second");
        <Editor as Component>::handle_input(&mut e, "\x1by"); // Alt+Y - "first"
        assert_eq!(e.get_text(), "first");
        <Editor as Component>::handle_input(&mut e, "\x1by"); // Alt+Y - "third"
        assert_eq!(e.get_text(), "third");
    }

    #[test]
    fn test_editor_alt_y_does_nothing_if_not_preceded_by_yank() {
        let mut e = editor();
        e.set_text("test");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        e.set_text("other");
        <Editor as Component>::handle_input(&mut e, "x");
        assert_eq!(e.get_text(), "otherx");
        <Editor as Component>::handle_input(&mut e, "\x1by"); // Alt+Y - no-op
        assert_eq!(e.get_text(), "otherx");
    }

    #[test]
    fn test_editor_alt_y_does_nothing_if_ring_has_one_entry() {
        let mut e = editor();
        e.set_text("only");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "only");
        <Editor as Component>::handle_input(&mut e, "\x1by"); // Alt+Y - no-op
        assert_eq!(e.get_text(), "only");
    }

    #[test]
    fn test_editor_ctrl_u_accumulates_multiline() {
        let mut e = editor();
        e.set_text("line1\nline2\nline3");
        // cursor at end of line3
        <Editor as Component>::handle_input(&mut e, "\x15"); // Ctrl+U deletes "line3"
        assert_eq!(e.get_text(), "line1\nline2\n");
        <Editor as Component>::handle_input(&mut e, "\x15"); // Ctrl+U deletes newline
        assert_eq!(e.get_text(), "line1\nline2");
        <Editor as Component>::handle_input(&mut e, "\x15"); // Ctrl+U deletes "line2"
        assert_eq!(e.get_text(), "line1\n");
        <Editor as Component>::handle_input(&mut e, "\x15"); // Ctrl+U deletes newline
        assert_eq!(e.get_text(), "line1");
        <Editor as Component>::handle_input(&mut e, "\x15"); // Ctrl+U deletes "line1"
        assert_eq!(e.get_text(), "");
        // All accumulated into one entry
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "line1\nline2\nline3");
    }

    #[test]
    fn test_editor_backward_deletions_prepend_forward_deletions_append() {
        let mut e = editor();
        e.set_text("prefix|suffix");
        // Position at '|' (index 6)
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..6 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right x6
        }
        <Editor as Component>::handle_input(&mut e, "\x0b"); // Ctrl+K - deletes "suffix" (forward, appended)
        <Editor as Component>::handle_input(&mut e, "\x0b"); // Ctrl+K - deletes "|" (forward, appended)
        assert_eq!(e.get_text(), "prefix");
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "prefix|suffix");
    }

    #[test]
    fn test_editor_non_delete_breaks_kill_accumulation() {
        let mut e = editor();
        e.set_text("foo bar baz");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - "baz"
        assert_eq!(e.get_text(), "foo bar ");
        <Editor as Component>::handle_input(&mut e, "x"); // breaks accumulation
        assert_eq!(e.get_text(), "foo bar x");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - "x" (separate)
        assert_eq!(e.get_text(), "foo bar ");
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y - most recent "x"
        assert_eq!(e.get_text(), "foo bar x");
        <Editor as Component>::handle_input(&mut e, "\x1by"); // Alt+Y - "baz"
        assert_eq!(e.get_text(), "foo bar baz");
    }

    #[test]
    fn test_editor_non_yank_actions_break_alt_y_chain() {
        let mut e = editor();
        e.set_text("first");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        e.set_text("second");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        e.set_text("");
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y - "second"
        assert_eq!(e.get_text(), "second");
        <Editor as Component>::handle_input(&mut e, "x"); // breaks yank chain
        assert_eq!(e.get_text(), "secondx");
        <Editor as Component>::handle_input(&mut e, "\x1by"); // Alt+Y - no-op
        assert_eq!(e.get_text(), "secondx");
    }

    #[test]
    fn test_editor_kill_ring_rotation_persists() {
        let mut e = editor();
        e.set_text("first");
        <Editor as Component>::handle_input(&mut e, "\x17");
        e.set_text("second");
        <Editor as Component>::handle_input(&mut e, "\x17");
        e.set_text("third");
        <Editor as Component>::handle_input(&mut e, "\x17");
        e.set_text("");
        // Ring: [first, second, third]
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y - "third"
        <Editor as Component>::handle_input(&mut e, "\x1by"); // Alt+Y - "second"
        assert_eq!(e.get_text(), "second");

        // After rotation, ring: [third, first, second]
        <Editor as Component>::handle_input(&mut e, "x");
        e.set_text("");
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y - "second"
        assert_eq!(e.get_text(), "second");
    }

    #[test]
    fn test_editor_consecutive_deletions_across_lines_coalesce() {
        let mut e = editor();
        e.set_text("1\n2\n3");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - "3"
        assert_eq!(e.get_text(), "1\n2\n");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - newline
        assert_eq!(e.get_text(), "1\n2");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - "2"
        assert_eq!(e.get_text(), "1\n");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - newline
        assert_eq!(e.get_text(), "1");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - "1"
        assert_eq!(e.get_text(), "");
        // All accumulated into one entry
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "1\n2\n3");
    }

    #[test]
    fn test_editor_ctrl_k_at_line_end_deletes_newline_and_coalesces() {
        let mut e = editor();
        // "ab\ncd" - move cursor to end of "ab" (first line)
        e.set_text("");
        <Editor as Component>::handle_input(&mut e, "a");
        <Editor as Component>::handle_input(&mut e, "b");
        e.handle_newline();
        <Editor as Component>::handle_input(&mut e, "c");
        <Editor as Component>::handle_input(&mut e, "d");
        // Move to end of first line
        e.move_cursor_up();
        e.move_cursor_line_end();
        // Now at end of "ab"
        <Editor as Component>::handle_input(&mut e, "\x0b"); // Ctrl+K - deletes newline
        assert_eq!(e.get_text(), "abcd");
        <Editor as Component>::handle_input(&mut e, "\x0b"); // Ctrl+K - deletes "cd"
        assert_eq!(e.get_text(), "ab");
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "ab\ncd");
    }

    #[test]
    fn test_editor_yank_in_middle_of_text() {
        let mut e = editor();
        e.set_text("word");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        e.set_text("hello world");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..6 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "hello wordworld");
    }

    #[test]
    fn test_editor_yank_pop_in_middle_of_text() {
        let mut e = editor();
        e.set_text("FIRST");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        e.set_text("SECOND");
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        // Ring: [FIRST, SECOND]
        e.set_text("hello world");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..6 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y - "SECOND"
        assert_eq!(e.get_text(), "hello SECONDworld");
        <Editor as Component>::handle_input(&mut e, "\x1by"); // Alt+Y - "FIRST"
        assert_eq!(e.get_text(), "hello FIRSTworld");
    }

    #[test]
    fn test_editor_alt_d_at_end_of_line_deletes_newline() {
        let mut e = editor();
        e.set_text("line1\nline2");
        e.move_cursor_up();
        e.move_cursor_line_end();
        <Editor as Component>::handle_input(&mut e, "\x1bd"); // Alt+D - deletes newline
        assert_eq!(e.get_text(), "line1line2");
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y
        assert_eq!(e.get_text(), "line1\nline2");
    }

    // =========================================================================
    // Tests from editor.test.ts – Undo (additional)
    // =========================================================================

    #[test]
    fn test_editor_undo_undoes_spaces_one_at_a_time() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        <Editor as Component>::handle_input(&mut e, " ");
        <Editor as Component>::handle_input(&mut e, " ");
        assert_eq!(e.get_text(), "hello  ");

        <Editor as Component>::handle_input(&mut e, "\x1f"); // Ctrl+- (undo) - removes second " "
        assert_eq!(e.get_text(), "hello ");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // removes first " "
        assert_eq!(e.get_text(), "hello");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // removes "hello"
        assert_eq!(e.get_text(), "");
    }

    #[test]
    fn test_editor_undo_undoes_newlines() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        e.handle_newline();
        for ch in ["w", "o", "r", "l", "d"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        assert_eq!(e.get_text(), "hello\nworld");

        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo "world"
        assert_eq!(e.get_text(), "hello\n");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo newline
        assert_eq!(e.get_text(), "hello");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo "hello"
        assert_eq!(e.get_text(), "");
    }

    #[test]
    fn test_editor_undo_undoes_backspace() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        <Editor as Component>::handle_input(&mut e, "\x7f"); // Backspace
        assert_eq!(e.get_text(), "hell");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello");
    }

    #[test]
    fn test_editor_undo_undoes_forward_delete() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        <Editor as Component>::handle_input(&mut e, "\x1b[3~"); // Delete key
        assert_eq!(e.get_text(), "hllo");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello");
    }

    #[test]
    fn test_editor_undo_ctrl_w() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W
        assert_eq!(e.get_text(), "hello ");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
    }

    #[test]
    fn test_editor_undo_ctrl_k() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..6 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        <Editor as Component>::handle_input(&mut e, "\x0b"); // Ctrl+K
        assert_eq!(e.get_text(), "hello ");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
        <Editor as Component>::handle_input(&mut e, "|");
        assert_eq!(e.get_text(), "hello |world");
    }

    #[test]
    fn test_editor_undo_ctrl_u() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..6 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        <Editor as Component>::handle_input(&mut e, "\x15"); // Ctrl+U
        assert_eq!(e.get_text(), "world");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
    }

    #[test]
    fn test_editor_undo_yank() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o", " "] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - delete "hello "
        <Editor as Component>::handle_input(&mut e, "\x19"); // Ctrl+Y - yank
        assert_eq!(e.get_text(), "hello ");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "");
    }

    #[test]
    fn test_editor_undo_single_line_paste_atomically() {
        let mut e = editor();
        e.set_text("hello world");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..5 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        <Editor as Component>::handle_input(&mut e, "\x1b[200~beep boop\x1b[201~");
        assert_eq!(e.get_text(), "hellobeep boop world");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
        <Editor as Component>::handle_input(&mut e, "|");
        assert_eq!(e.get_text(), "hello| world");
    }

    #[test]
    fn test_editor_undo_multi_line_paste_atomically() {
        let mut e = editor();
        e.set_text("hello world");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..5 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        <Editor as Component>::handle_input(&mut e, "\x1b[200~line1\nline2\nline3\x1b[201~");
        assert_eq!(e.get_text(), "helloline1\nline2\nline3 world");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
        <Editor as Component>::handle_input(&mut e, "|");
        assert_eq!(e.get_text(), "hello| world");
    }

    #[test]
    fn test_editor_insert_text_at_cursor_single_line() {
        let mut e = editor();
        e.set_text("hello world");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..5 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        e.insert_text_at_cursor("/tmp/image.png");
        assert_eq!(e.get_text(), "hello/tmp/image.png world");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
        <Editor as Component>::handle_input(&mut e, "|");
        assert_eq!(e.get_text(), "hello| world");
    }

    #[test]
    fn test_editor_insert_text_at_cursor_multiline() {
        let mut e = editor();
        e.set_text("hello world");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..5 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        e.insert_text_at_cursor("line1\nline2\nline3");
        assert_eq!(e.get_text(), "helloline1\nline2\nline3 world");
        // Cursor should be at end of inserted text
        assert_eq!(e.state.cursor_line, 2);
        assert_eq!(e.state.cursor_col, 5); // "line3".len()
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
    }

    #[test]
    fn test_editor_insert_text_at_cursor_normalizes_crlf() {
        let mut e = editor();
        e.set_text("");
        e.insert_text_at_cursor("a\r\nb\r\nc");
        assert_eq!(e.get_text(), "a\nb\nc");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "");

        e.insert_text_at_cursor("x\ry\rz");
        assert_eq!(e.get_text(), "x\ny\nz");
    }

    #[test]
    fn test_editor_undo_set_text_to_empty() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        assert_eq!(e.get_text(), "hello world");
        e.set_text("");
        assert_eq!(e.get_text(), "");
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
    }

    #[test]
    fn test_editor_undo_cursor_movement_starts_new_unit() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        assert_eq!(e.get_text(), "hello world");
        // Move cursor left 5 (to after "hello ")
        for _ in 0..5 {
            <Editor as Component>::handle_input(&mut e, "\x1b[D"); // Left
        }
        // Type "lol" in the middle
        for ch in ["l", "o", "l"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        assert_eq!(e.get_text(), "hello lolworld");
        // Undo should restore to "hello world"
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
        <Editor as Component>::handle_input(&mut e, "|");
        assert_eq!(e.get_text(), "hello |world");
    }

    #[test]
    fn test_editor_noop_delete_does_not_push_undo() {
        let mut e = editor();
        for ch in ["h", "e", "l", "l", "o"] {
            <Editor as Component>::handle_input(&mut e, ch);
        }
        <Editor as Component>::handle_input(&mut e, "\x17"); // Ctrl+W - deletes "hello"
        assert_eq!(e.get_text(), "");
        <Editor as Component>::handle_input(&mut e, "\x17"); // no-op
        <Editor as Component>::handle_input(&mut e, "\x17"); // no-op
        // Single undo should restore "hello"
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello");
    }

    // =========================================================================
    // Tests from editor.test.ts – Backslash+Enter newline workaround (additional)
    // =========================================================================

    #[test]
    fn test_editor_backslash_enter_converts_to_newline() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "\\");
        <Editor as Component>::handle_input(&mut e, "\r");
        assert_eq!(e.get_text(), "\n");
    }

    #[test]
    fn test_editor_backslash_followed_by_other_char() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "\\");
        <Editor as Component>::handle_input(&mut e, "x");
        assert_eq!(e.get_text(), "\\x");
    }

    #[test]
    fn test_editor_backslash_not_immediately_before_cursor_submits() {
        let mut e = editor();
        let submitted = std::sync::Arc::new(std::sync::Mutex::new(false));
        let s = submitted.clone();
        e.set_on_submit(Box::new(move |_| {
            *s.lock().unwrap() = true;
        }));
        <Editor as Component>::handle_input(&mut e, "\\");
        <Editor as Component>::handle_input(&mut e, "x");
        <Editor as Component>::handle_input(&mut e, "\r"); // Enter - should submit
        assert!(*submitted.lock().unwrap());
    }

    #[test]
    fn test_editor_multiple_backslashes_only_removes_last() {
        let mut e = editor();
        <Editor as Component>::handle_input(&mut e, "\\");
        <Editor as Component>::handle_input(&mut e, "\\");
        <Editor as Component>::handle_input(&mut e, "\\");
        assert_eq!(e.get_text(), "\\\\\\");
        <Editor as Component>::handle_input(&mut e, "\r");
        // Only last backslash removed, newline inserted
        assert_eq!(e.get_text(), "\\\\\n");
    }

    // =========================================================================
    // Tests from editor.test.ts – History (more non-arrow-dependent cases)
    // =========================================================================

    #[test]
    fn test_editor_add_to_history_ignores_empty_and_whitespace() {
        let mut e = editor();
        e.add_to_history("");
        e.add_to_history("   ");
        e.add_to_history("valid");
        // History should only contain "valid"
        assert_eq!(e.history.len(), 1);
        assert_eq!(e.history[0], "valid");
    }

    #[test]
    fn test_editor_add_to_history_allows_non_consecutive_duplicates() {
        let mut e = editor();
        e.add_to_history("first");
        e.add_to_history("second");
        e.add_to_history("first"); // not consecutive, should be added
        assert_eq!(e.history.len(), 3);
    }

    // =========================================================================
    // Tests from editor.test.ts – Word wrapping (wordWrapLine tests)
    // These test the wrap_text_with_ansi behavior that editor relies on.
    // =========================================================================

    #[test]
    fn test_editor_word_wrap_at_boundaries() {
        let mut e = editor();
        let width = 40u16;
        e.set_text("Hello world this is a test of word wrapping functionality");
        let lines = <Editor as Component>::render(&e, width);
        // Each content line should not exceed width
        for line in &lines {
            assert!(
                crate::utils::visible_width(line) <= width as usize,
                "Line exceeded width: {:?}",
                line
            );
        }
    }

    #[test]
    fn test_editor_render_empty_string() {
        let mut e = editor();
        let width = 40u16;
        e.set_text("");
        let lines = <Editor as Component>::render(&e, width);
        // Should have exactly 1 line (empty)
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_editor_render_width_limits_satisfied() {
        let mut e = editor();
        let width = 30u16;
        e.set_text("Check https://example.com/very/long/path/that/exceeds/width here");
        let lines = <Editor as Component>::render(&e, width);
        for line in &lines {
            assert!(
                crate::utils::visible_width(line) <= width as usize,
                "Line exceeded width: {:?}",
                line
            );
        }
    }

    #[test]
    fn test_editor_insert_text_at_cursor_pushes_undo() {
        let mut e = editor();
        e.set_text("hello world");
        <Editor as Component>::handle_input(&mut e, "\x01"); // Ctrl+A
        for _ in 0..5 {
            <Editor as Component>::handle_input(&mut e, "\x1b[C"); // Right
        }
        e.insert_text_at_cursor("/tmp/image.png");
        assert_eq!(e.get_text(), "hello/tmp/image.png world");
        // Single undo should restore pre-insert state
        <Editor as Component>::handle_input(&mut e, "\x1f"); // undo
        assert_eq!(e.get_text(), "hello world");
    }

    #[test]
    fn test_editor_does_not_trigger_autocomplete_during_paste() {
        // Test that bracketed paste doesn't call autocomplete
        let mut e = editor();
        let call_count = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let cc = call_count.clone();
        struct MockProvider {
            call_count: std::sync::Arc<std::sync::Mutex<u32>>,
        }
        impl crate::autocomplete::AutocompleteProvider for MockProvider {
            fn get_suggestions(
                &self,
                _lines: &[String],
                _cursor_line: usize,
                _cursor_col: usize,
            ) -> Option<crate::autocomplete::AutocompleteSuggestions> {
                *self.call_count.lock().unwrap() += 1;
                None
            }
            fn apply_completion(
                &self,
                lines: &[String],
                cursor_line: usize,
                cursor_col: usize,
                _item: &crate::autocomplete::AutocompleteItem,
                _prefix: &str,
            ) -> (Vec<String>, usize, usize) {
                (lines.to_vec(), cursor_line, cursor_col)
            }
        }
        e.set_autocomplete_provider(Box::new(MockProvider { call_count: cc }));
        <Editor as Component>::handle_input(
            &mut e,
            "\x1b[200~look at @node_modules/react/index.js please\x1b[201~",
        );
        assert_eq!(e.get_text(), "look at @node_modules/react/index.js please");
        assert_eq!(*call_count.lock().unwrap(), 0);
    }
}

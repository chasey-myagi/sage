/// Input component — single-line text input with horizontal scrolling.

use crate::keybindings::check_keybinding;
use crate::keys::decode_kitty_printable;
use crate::kill_ring::KillRing;
use crate::tui::{Component, Focusable, CURSOR_MARKER};
use crate::undo_stack::UndoStack;
use crate::utils::{is_punctuation_char, is_whitespace_char, slice_by_column, visible_width};

#[derive(Clone, Debug)]
struct InputState {
    value: String,
    cursor: usize,
}

/// Single-line text input with Emacs-style key bindings.
pub struct Input {
    value: String,
    cursor: usize,
    pub on_submit: Option<Box<dyn Fn(String) + Send>>,
    pub on_escape: Option<Box<dyn Fn() + Send>>,

    /// Focusable interface — set by TUI when focus changes.
    pub focused: bool,

    // Bracketed paste buffering
    paste_buffer: String,
    is_in_paste: bool,

    // Kill ring for Emacs-style kill/yank
    kill_ring: KillRing,
    last_action: Option<LastAction>,

    // Undo support
    undo_stack: UndoStack<InputState>,
}

#[derive(Clone, Debug, PartialEq)]
enum LastAction {
    Kill,
    Yank,
    TypeWord,
}

impl Input {
    pub fn new() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            on_submit: None,
            on_escape: None,
            focused: false,
            paste_buffer: String::new(),
            is_in_paste: false,
            kill_ring: KillRing::new(),
            last_action: None,
            undo_stack: UndoStack::new(),
        }
    }

    pub fn get_value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.cursor.min(self.value.len());
        // Reset kill/yank chain so a subsequent delete doesn't accumulate with
        // kills from before setValue was called.
        self.last_action = None;
    }

    // Helper: grapheme-aware segment of string
    fn graphemes_before_cursor(&self) -> Vec<String> {
        self.value[..self.cursor].chars().map(|c| c.to_string()).collect()
    }

    fn graphemes_after_cursor(&self) -> Vec<String> {
        self.value[self.cursor..].chars().map(|c| c.to_string()).collect()
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(&InputState {
            value: self.value.clone(),
            cursor: self.cursor,
        });
    }

    fn undo(&mut self) {
        if let Some(snapshot) = self.undo_stack.pop() {
            self.value = snapshot.value;
            self.cursor = snapshot.cursor;
            self.last_action = None;
        }
    }

    fn insert_character(&mut self, ch: &str) {
        let first_char_is_ws = ch.chars().next().is_some_and(is_whitespace_char);
        if first_char_is_ws || self.last_action != Some(LastAction::TypeWord) {
            self.push_undo();
        }
        self.last_action = Some(LastAction::TypeWord);

        self.value = format!("{}{ch}{}", &self.value[..self.cursor], &self.value[self.cursor..]);
        self.cursor += ch.len();
    }

    fn handle_backspace(&mut self) {
        self.last_action = None;
        if self.cursor > 0 {
            self.push_undo();
            let before = &self.value[..self.cursor];
            let grapheme_len = before.chars().last().map(|c| c.len_utf8()).unwrap_or(1);
            let new_cursor = self.cursor - grapheme_len;
            self.value = format!("{}{}", &self.value[..new_cursor], &self.value[self.cursor..]);
            self.cursor = new_cursor;
        }
    }

    fn handle_forward_delete(&mut self) {
        self.last_action = None;
        if self.cursor < self.value.len() {
            self.push_undo();
            let after = &self.value[self.cursor..];
            let grapheme_len = after.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            self.value = format!("{}{}", &self.value[..self.cursor], &self.value[self.cursor + grapheme_len..]);
        }
    }

    fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let deleted_text = self.value[..self.cursor].to_string();
        self.kill_ring.push(&deleted_text, true, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
        self.value = self.value[self.cursor..].to_string();
        self.cursor = 0;
    }

    fn delete_to_line_end(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.push_undo();
        let deleted_text = self.value[self.cursor..].to_string();
        self.kill_ring.push(&deleted_text, false, self.last_action == Some(LastAction::Kill));
        self.last_action = Some(LastAction::Kill);
        self.value = self.value[..self.cursor].to_string();
    }

    fn delete_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();

        let old_cursor = self.cursor;
        self.move_word_backwards();
        let delete_from = self.cursor;
        self.cursor = old_cursor;

        let deleted_text = self.value[delete_from..self.cursor].to_string();
        self.kill_ring.push(&deleted_text, true, was_kill);
        self.last_action = Some(LastAction::Kill);

        self.value = format!("{}{}", &self.value[..delete_from], &self.value[self.cursor..]);
        self.cursor = delete_from;
    }

    fn delete_word_forward(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let was_kill = self.last_action == Some(LastAction::Kill);
        self.push_undo();

        let old_cursor = self.cursor;
        self.move_word_forwards();
        let delete_to = self.cursor;
        self.cursor = old_cursor;

        let deleted_text = self.value[self.cursor..delete_to].to_string();
        self.kill_ring.push(&deleted_text, false, was_kill);
        self.last_action = Some(LastAction::Kill);

        self.value = format!("{}{}", &self.value[..self.cursor], &self.value[delete_to..]);
    }

    fn yank(&mut self) {
        let text = match self.kill_ring.peek() {
            Some(t) => t.to_string(),
            None => return,
        };
        self.push_undo();
        self.value = format!("{}{text}{}", &self.value[..self.cursor], &self.value[self.cursor..]);
        self.cursor += text.len();
        self.last_action = Some(LastAction::Yank);
    }

    fn yank_pop(&mut self) {
        if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
            return;
        }
        self.push_undo();

        let prev_text = self.kill_ring.peek().map(|t| t.to_string()).unwrap_or_default();
        let prev_len = prev_text.len();
        let new_cursor = self.cursor.saturating_sub(prev_len);
        self.value = format!("{}{}", &self.value[..new_cursor], &self.value[self.cursor..]);
        self.cursor = new_cursor;

        self.kill_ring.rotate();
        let text = self.kill_ring.peek().map(|t| t.to_string()).unwrap_or_default();
        self.value = format!("{}{text}{}", &self.value[..self.cursor], &self.value[self.cursor..]);
        self.cursor += text.len();
        self.last_action = Some(LastAction::Yank);
    }

    fn move_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.last_action = None;
        let text_before = &self.value[..self.cursor];
        let chars: Vec<char> = text_before.chars().collect();
        let mut pos = chars.len();

        // Skip trailing whitespace
        while pos > 0 && is_whitespace_char(chars[pos - 1]) {
            pos -= 1;
        }

        if pos > 0 {
            if is_punctuation_char(chars[pos - 1]) {
                while pos > 0 && is_punctuation_char(chars[pos - 1]) {
                    pos -= 1;
                }
            } else {
                while pos > 0
                    && !is_whitespace_char(chars[pos - 1])
                    && !is_punctuation_char(chars[pos - 1])
                {
                    pos -= 1;
                }
            }
        }

        // Compute byte offset
        self.cursor = chars[..pos].iter().map(|c| c.len_utf8()).sum();
    }

    fn move_word_forwards(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.last_action = None;
        let text_after = &self.value[self.cursor..];
        let chars: Vec<char> = text_after.chars().collect();
        let mut pos = 0;

        // Skip leading whitespace
        while pos < chars.len() && is_whitespace_char(chars[pos]) {
            pos += 1;
        }

        if pos < chars.len() {
            if is_punctuation_char(chars[pos]) {
                while pos < chars.len() && is_punctuation_char(chars[pos]) {
                    pos += 1;
                }
            } else {
                while pos < chars.len()
                    && !is_whitespace_char(chars[pos])
                    && !is_punctuation_char(chars[pos])
                {
                    pos += 1;
                }
            }
        }

        let byte_advance: usize = chars[..pos].iter().map(|c| c.len_utf8()).sum();
        self.cursor += byte_advance;
    }

    fn handle_paste(&mut self, pasted_text: &str) {
        self.last_action = None;
        self.push_undo();

        let clean_text = pasted_text
            .replace("\r\n", "")
            .replace('\r', "")
            .replace('\n', "")
            .replace('\t', "    ");

        self.value = format!("{}{clean_text}{}", &self.value[..self.cursor], &self.value[self.cursor..]);
        self.cursor += clean_text.len();
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Focusable for Input {
    fn focused(&self) -> bool {
        self.focused
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }
}

impl Component for Input {
    fn handle_input(&mut self, data: &str) {
        // Handle bracketed paste
        if data.contains("\x1b[200~") {
            self.is_in_paste = true;
            self.paste_buffer = String::new();
            let data = data.replace("\x1b[200~", "");
            if self.is_in_paste {
                self.paste_buffer.push_str(&data);
                let end_idx = self.paste_buffer.find("\x1b[201~");
                if let Some(end) = end_idx {
                    let paste_content = self.paste_buffer[..end].to_string();
                    self.handle_paste(&paste_content);
                    self.is_in_paste = false;
                    let remaining = self.paste_buffer[end + 6..].to_string();
                    self.paste_buffer = String::new();
                    if !remaining.is_empty() {
                        self.handle_input(&remaining);
                    }
                }
                return;
            }
        }

        if self.is_in_paste {
            self.paste_buffer.push_str(data);
            let end_idx = self.paste_buffer.find("\x1b[201~");
            if let Some(end) = end_idx {
                let paste_content = self.paste_buffer[..end].to_string();
                self.handle_paste(&paste_content);
                self.is_in_paste = false;
                let remaining = self.paste_buffer[end + 6..].to_string();
                self.paste_buffer = String::new();
                if !remaining.is_empty() {
                    self.handle_input(&remaining);
                }
            }
            return;
        }

        // Escape / Cancel
        if check_keybinding(data, "tui.select.cancel") {
            if let Some(cb) = &self.on_escape {
                cb();
            }
            return;
        }

        // Undo
        if check_keybinding(data, "tui.editor.undo") {
            self.undo();
            return;
        }

        // Submit
        if check_keybinding(data, "tui.input.submit") || data == "\n" {
            let value = self.value.clone();
            if let Some(cb) = &self.on_submit {
                cb(value);
            }
            return;
        }

        // Deletion
        if check_keybinding(data, "tui.editor.deleteCharBackward") {
            self.handle_backspace();
            return;
        }
        if check_keybinding(data, "tui.editor.deleteCharForward") {
            self.handle_forward_delete();
            return;
        }
        if check_keybinding(data, "tui.editor.deleteWordBackward") {
            self.delete_word_backwards();
            return;
        }
        if check_keybinding(data, "tui.editor.deleteWordForward") {
            self.delete_word_forward();
            return;
        }
        if check_keybinding(data, "tui.editor.deleteToLineStart") {
            self.delete_to_line_start();
            return;
        }
        if check_keybinding(data, "tui.editor.deleteToLineEnd") {
            self.delete_to_line_end();
            return;
        }

        // Kill ring
        if check_keybinding(data, "tui.editor.yank") {
            self.yank();
            return;
        }
        if check_keybinding(data, "tui.editor.yankPop") {
            self.yank_pop();
            return;
        }

        // Cursor movement
        if check_keybinding(data, "tui.editor.cursorLeft") {
            self.last_action = None;
            if self.cursor > 0 {
                let before = &self.value[..self.cursor];
                let grapheme_len = before.chars().last().map(|c| c.len_utf8()).unwrap_or(1);
                self.cursor -= grapheme_len;
            }
            return;
        }
        if check_keybinding(data, "tui.editor.cursorRight") {
            self.last_action = None;
            if self.cursor < self.value.len() {
                let after = &self.value[self.cursor..];
                let grapheme_len = after.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                self.cursor += grapheme_len;
            }
            return;
        }
        if check_keybinding(data, "tui.editor.cursorLineStart") {
            self.last_action = None;
            self.cursor = 0;
            return;
        }
        if check_keybinding(data, "tui.editor.cursorLineEnd") {
            self.last_action = None;
            self.cursor = self.value.len();
            return;
        }
        if check_keybinding(data, "tui.editor.cursorWordLeft") {
            self.move_word_backwards();
            return;
        }
        if check_keybinding(data, "tui.editor.cursorWordRight") {
            self.move_word_forwards();
            return;
        }

        // Kitty CSI-u printable
        if let Some(kitty_char) = decode_kitty_printable(data) {
            self.insert_character(&kitty_char.to_string());
            return;
        }

        // Regular characters — reject control characters
        let has_control_chars = data.chars().any(|c| {
            let code = c as u32;
            code < 32 || code == 0x7f || (code >= 0x80 && code <= 0x9f)
        });
        if !has_control_chars {
            self.insert_character(data);
        }
    }

    fn render(&self, width: u16) -> Vec<String> {
        let width = width as usize;
        let prompt = "> ";
        let available_width = width.saturating_sub(prompt.len());

        if available_width == 0 {
            return vec![prompt.to_string()];
        }

        let total_width = visible_width(&self.value);
        let (visible_text, cursor_display) = if total_width < available_width {
            (self.value.clone(), self.cursor)
        } else {
            // Need horizontal scrolling
            let scroll_width = if self.cursor == self.value.len() {
                available_width.saturating_sub(1)
            } else {
                available_width
            };
            let cursor_col = visible_width(&self.value[..self.cursor]);

            let start_col = if scroll_width > 0 {
                let half_width = scroll_width / 2;
                if cursor_col < half_width {
                    0
                } else if cursor_col > total_width.saturating_sub(half_width) {
                    total_width.saturating_sub(scroll_width)
                } else {
                    cursor_col.saturating_sub(half_width)
                }
            } else {
                0
            };

            let vt = if scroll_width > 0 {
                slice_by_column(&self.value, start_col, scroll_width, true)
            } else {
                String::new()
            };
            let before_cursor_slice = slice_by_column(&self.value, start_col, cursor_col.saturating_sub(start_col), true);
            let cd = before_cursor_slice.len();
            (vt, cd)
        };

        // Build line with fake cursor
        let after_cursor_chars: Vec<char> = visible_text[cursor_display..].chars().collect();
        let at_cursor_char = after_cursor_chars.first().map(|c| c.to_string()).unwrap_or(" ".to_string());
        let at_cursor_len = at_cursor_char.len();

        let before_cursor = &visible_text[..cursor_display];
        let after_cursor = if cursor_display + at_cursor_len <= visible_text.len() {
            &visible_text[cursor_display + at_cursor_len..]
        } else {
            ""
        };

        let marker = if self.focused { CURSOR_MARKER } else { "" };
        let cursor_char = format!("\x1b[7m{at_cursor_char}\x1b[27m");
        let text_with_cursor = format!("{before_cursor}{marker}{cursor_char}{after_cursor}");

        let visual_length = visible_width(&text_with_cursor);
        let padding = " ".repeat(available_width.saturating_sub(visual_length));
        let line = format!("{prompt}{text_with_cursor}{padding}");

        vec![line]
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_empty_render() {
        let input = Input::new();
        let lines = input.render(80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("> "));
    }

    #[test]
    fn test_input_type_chars() {
        let mut input = Input::new();
        input.handle_input("h");
        input.handle_input("i");
        assert_eq!(input.get_value(), "hi");
    }

    #[test]
    fn test_input_backspace() {
        let mut input = Input::new();
        input.set_value("hello");
        input.cursor = 5;
        input.handle_input("\x7f"); // Backspace
        assert_eq!(input.get_value(), "hell");
    }

    #[test]
    fn test_input_cursor_movement() {
        let mut input = Input::new();
        input.set_value("hello");
        input.cursor = 5;
        input.handle_input("\x1b[D"); // Left
        assert_eq!(input.cursor, 4);
        input.handle_input("\x1b[C"); // Right
        assert_eq!(input.cursor, 5);
    }

    #[test]
    fn test_input_undo() {
        let mut input = Input::new();
        input.handle_input("a");
        input.handle_input("b");
        input.handle_input("\x1f"); // Ctrl+-
        assert_eq!(input.get_value(), "");
    }

    #[test]
    fn test_input_delete_to_line_end() {
        let mut input = Input::new();
        input.set_value("hello world");
        input.cursor = 5;
        input.handle_input("\x0b"); // Ctrl+K
        assert_eq!(input.get_value(), "hello");
    }

    #[test]
    fn test_input_home_end() {
        let mut input = Input::new();
        input.set_value("hello");
        input.cursor = 3;
        input.handle_input("\x1b[H"); // Home
        assert_eq!(input.cursor, 0);
        input.handle_input("\x1b[F"); // End
        assert_eq!(input.cursor, 5);
    }

    // =========================================================================
    // Tests from input.test.ts
    // =========================================================================

    #[test]
    fn test_input_submit_includes_backslash() {
        let mut input = Input::new();
        let submitted = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let s = submitted.clone();
        input.on_submit = Some(Box::new(move |v| {
            *s.lock().unwrap() = Some(v);
        }));

        for ch in ["h", "e", "l", "l", "o", "\\"] {
            input.handle_input(ch);
        }
        input.handle_input("\r"); // Enter

        let result = submitted.lock().unwrap().clone();
        assert_eq!(result, Some("hello\\".to_string()));
    }

    #[test]
    fn test_input_backslash_as_regular_char() {
        let mut input = Input::new();
        input.handle_input("\\");
        input.handle_input("x");
        assert_eq!(input.get_value(), "\\x");
    }

    // =========================================================================
    // Tests from input.test.ts – Kill ring
    // =========================================================================

    #[test]
    fn test_input_ctrl_w_saves_to_kill_ring_and_yank() {
        let mut input = Input::new();
        input.set_value("foo bar baz");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W - deletes "baz"
        assert_eq!(input.get_value(), "foo bar ");

        input.handle_input("\x01"); // Ctrl+A
        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "bazfoo bar ");
    }

    #[test]
    fn test_input_ctrl_u_saves_to_kill_ring() {
        let mut input = Input::new();
        input.set_value("hello world");
        input.handle_input("\x01"); // Ctrl+A
        for _ in 0..6 {
            input.handle_input("\x1b[C");
        }
        input.handle_input("\x15"); // Ctrl+U - deletes "hello "
        assert_eq!(input.get_value(), "world");
        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn test_input_ctrl_k_saves_to_kill_ring() {
        let mut input = Input::new();
        input.set_value("hello world");
        input.handle_input("\x01"); // Ctrl+A
        input.handle_input("\x0b"); // Ctrl+K
        assert_eq!(input.get_value(), "");
        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn test_input_ctrl_y_does_nothing_when_ring_empty() {
        let mut input = Input::new();
        input.set_value("test");
        input.cursor = input.value.len();
        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "test");
    }

    #[test]
    fn test_input_consecutive_ctrl_w_accumulates() {
        let mut input = Input::new();
        input.set_value("one two three");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // deletes "three"
        input.handle_input("\x17"); // deletes "two "
        input.handle_input("\x17"); // deletes "one "
        assert_eq!(input.get_value(), "");
        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "one two three");
    }

    #[test]
    fn test_input_non_delete_breaks_kill_accumulation() {
        let mut input = Input::new();
        input.set_value("foo bar baz");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // deletes "baz"
        assert_eq!(input.get_value(), "foo bar ");

        input.handle_input("x"); // breaks accumulation
        assert_eq!(input.get_value(), "foo bar x");

        input.handle_input("\x17"); // deletes "x" (separate entry)
        assert_eq!(input.get_value(), "foo bar ");

        input.handle_input("\x19"); // most recent is "x"
        assert_eq!(input.get_value(), "foo bar x");

        input.handle_input("\x1by"); // cycle to "baz"
        assert_eq!(input.get_value(), "foo bar baz");
    }

    #[test]
    fn test_input_alt_d_delete_word_forward() {
        let mut input = Input::new();
        input.set_value("hello world test");
        input.handle_input("\x01"); // Ctrl+A

        input.handle_input("\x1bd"); // Alt+D - deletes "hello"
        assert_eq!(input.get_value(), " world test");

        input.handle_input("\x1bd"); // Alt+D - deletes " world"
        assert_eq!(input.get_value(), " test");

        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "hello world test");
    }

    #[test]
    fn test_input_yank_in_middle_of_text() {
        let mut input = Input::new();
        input.set_value("word");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W - deletes "word"
        input.set_value("hello world");
        input.handle_input("\x01"); // Ctrl+A
        for _ in 0..6 {
            input.handle_input("\x1b[C");
        }
        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "hello wordworld");
    }

    // =========================================================================
    // Tests from input.test.ts – Undo
    // =========================================================================

    #[test]
    fn test_input_undo_empty_stack_does_nothing() {
        let mut input = Input::new();
        input.handle_input("\x1f"); // Ctrl+- (undo)
        assert_eq!(input.get_value(), "");
    }

    #[test]
    fn test_input_undo_coalesces_word_chars() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o"] {
            input.handle_input(ch);
        }
        // space breaks coalescing: it calls push_undo before inserting the space.
        // After space, last_action is still TypeWord, so "world" doesn't push undo.
        // This means " world" and "hello" each form one undo unit.
        input.handle_input(" ");
        for ch in ["w", "o", "r", "l", "d"] {
            input.handle_input(ch);
        }
        assert_eq!(input.get_value(), "hello world");

        // First undo: restores state before the space was typed = "hello"
        // (The space push_undo captured state "hello", and "world" didn't push)
        input.handle_input("\x1f"); // Ctrl+- (undo) - removes " world" (back to "hello")
        assert_eq!(input.get_value(), "hello");

        // Second undo: restores state before "hello" = ""
        input.handle_input("\x1f"); // Ctrl+- (undo) - removes "hello"
        assert_eq!(input.get_value(), "");
    }

    #[test]
    fn test_input_undo_backspace() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o"] {
            input.handle_input(ch);
        }
        input.handle_input("\x7f"); // Backspace
        assert_eq!(input.get_value(), "hell");

        input.handle_input("\x1f"); // Ctrl+- (undo)
        assert_eq!(input.get_value(), "hello");
    }

    #[test]
    fn test_input_undo_ctrl_w() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            input.handle_input(ch);
        }
        assert_eq!(input.get_value(), "hello world");
        input.handle_input("\x17"); // Ctrl+W
        assert_eq!(input.get_value(), "hello ");

        input.handle_input("\x1f"); // Ctrl+- (undo)
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn test_input_undo_ctrl_k() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            input.handle_input(ch);
        }
        input.handle_input("\x01"); // Ctrl+A
        for _ in 0..6 {
            input.handle_input("\x1b[C");
        }
        input.handle_input("\x0b"); // Ctrl+K
        assert_eq!(input.get_value(), "hello ");
        input.handle_input("\x1b[45;5u");
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn test_input_undo_paste_atomically() {
        let mut input = Input::new();
        input.set_value("hello world");
        input.handle_input("\x01"); // Ctrl+A
        for _ in 0..5 {
            input.handle_input("\x1b[C");
        }
        input.handle_input("\x1b[200~beep boop\x1b[201~");
        assert_eq!(input.get_value(), "hellobeep boop world");

        input.handle_input("\x1b[45;5u");
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn test_input_undo_alt_d() {
        let mut input = Input::new();
        input.set_value("hello world");
        input.handle_input("\x01"); // Ctrl+A
        input.handle_input("\x1bd"); // Alt+D
        assert_eq!(input.get_value(), " world");
        input.handle_input("\x1b[45;5u");
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn test_input_cursor_movement_starts_new_undo_unit() {
        let mut input = Input::new();
        for ch in ["a", "b", "c"] {
            input.handle_input(ch);
        }
        input.handle_input("\x01"); // Ctrl+A - movement breaks coalescing
        input.handle_input("\x05"); // Ctrl+E
        for ch in ["d", "e"] {
            input.handle_input(ch);
        }
        assert_eq!(input.get_value(), "abcde");

        input.handle_input("\x1b[45;5u"); // removes "de"
        assert_eq!(input.get_value(), "abc");

        input.handle_input("\x1b[45;5u"); // removes "abc"
        assert_eq!(input.get_value(), "");
    }

    // =========================================================================
    // Additional tests from input.test.ts – Kill ring
    // =========================================================================

    #[test]
    fn test_input_alt_y_cycles_kill_ring() {
        let mut input = Input::new();
        input.set_value("first");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W
        input.set_value("second");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W
        input.set_value("third");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W
        assert_eq!(input.get_value(), "");

        input.handle_input("\x19"); // Ctrl+Y - "third"
        assert_eq!(input.get_value(), "third");
        input.handle_input("\x1by"); // Alt+Y - "second"
        assert_eq!(input.get_value(), "second");
        input.handle_input("\x1by"); // Alt+Y - "first"
        assert_eq!(input.get_value(), "first");
        input.handle_input("\x1by"); // Alt+Y - wraps to "third"
        assert_eq!(input.get_value(), "third");
    }

    #[test]
    fn test_input_alt_y_does_nothing_if_not_preceded_by_yank() {
        let mut input = Input::new();
        input.set_value("test");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W
        input.set_value("other");
        input.cursor = input.value.len();
        input.handle_input("x"); // break yank chain
        assert_eq!(input.get_value(), "otherx");
        input.handle_input("\x1by"); // Alt+Y - no-op
        assert_eq!(input.get_value(), "otherx");
    }

    #[test]
    fn test_input_alt_y_does_nothing_if_ring_has_one_entry() {
        let mut input = Input::new();
        input.set_value("only");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W
        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "only");
        input.handle_input("\x1by"); // Alt+Y - no-op
        assert_eq!(input.get_value(), "only");
    }

    #[test]
    fn test_input_kill_ring_rotation_persists() {
        let mut input = Input::new();
        input.set_value("first");
        input.cursor = input.value.len();
        input.handle_input("\x17");
        input.set_value("second");
        input.cursor = input.value.len();
        input.handle_input("\x17");
        input.set_value("third");
        input.cursor = input.value.len();
        input.handle_input("\x17");
        input.set_value("");

        input.handle_input("\x19"); // Ctrl+Y - "third"
        input.handle_input("\x1by"); // Alt+Y - "second"
        assert_eq!(input.get_value(), "second");

        // Break chain and start fresh
        input.handle_input("x");
        input.set_value("");

        // New yank should get "second" (now at ring end after rotation)
        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "second");
    }

    #[test]
    fn test_input_backward_delete_prepends_forward_appends() {
        let mut input = Input::new();
        input.set_value("prefix|suffix");
        // Position at '|' (index 6)
        input.handle_input("\x01"); // Ctrl+A
        for _ in 0..6 {
            input.handle_input("\x1b[C"); // Right x6
        }
        input.handle_input("\x0b"); // Ctrl+K - deletes "|suffix" (forward)
        assert_eq!(input.get_value(), "prefix");
        input.handle_input("\x19"); // Ctrl+Y
        assert_eq!(input.get_value(), "prefix|suffix");
    }

    #[test]
    fn test_input_non_yank_breaks_alt_y_chain() {
        let mut input = Input::new();
        input.set_value("first");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W
        input.set_value("second");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W
        input.set_value("");

        input.handle_input("\x19"); // Ctrl+Y - "second"
        assert_eq!(input.get_value(), "second");
        input.handle_input("x"); // breaks yank chain
        assert_eq!(input.get_value(), "secondx");
        input.handle_input("\x1by"); // Alt+Y - no-op
        assert_eq!(input.get_value(), "secondx");
    }

    #[test]
    fn test_input_yank_pop_in_middle_of_text() {
        let mut input = Input::new();
        input.set_value("FIRST");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W
        input.set_value("SECOND");
        input.cursor = input.value.len();
        input.handle_input("\x17"); // Ctrl+W

        input.set_value("hello world");
        input.handle_input("\x01"); // Ctrl+A
        for _ in 0..6 {
            input.handle_input("\x1b[C"); // Right
        }
        input.handle_input("\x19"); // Ctrl+Y - "SECOND"
        assert_eq!(input.get_value(), "hello SECONDworld");
        input.handle_input("\x1by"); // Alt+Y - "FIRST"
        assert_eq!(input.get_value(), "hello FIRSTworld");
    }

    // =========================================================================
    // Additional tests from input.test.ts – render
    // =========================================================================

    #[test]
    fn test_input_render_does_not_overflow_with_cjk() {
        let width = 93u16;
        let cases = [
            "가나다라마바사아자차카타파하 한글 텍스트가 터미널 너비를 초과하면 크래시가 발생합니다 이것은 재현용 테스트입니다",
            "これはテスト文章です。日本語のテキストが正しく表示されるかどうかを確認するためのサンプルテキストです。あいうえお",
            "这是一段测试文本，用于验证中文字符在终端中的显示宽度是否被正确计算，如果不正确就会导致用户界面崩溃的问题",
        ];
        for text in &cases {
            let mut input = Input::new();
            input.set_value(*text);
            input.focused = true;
            let lines = input.render(width);
            let line = &lines[0];
            assert!(
                crate::utils::visible_width(line) <= width as usize,
                "rendered line overflowed for CJK text, width={}",
                crate::utils::visible_width(line)
            );
        }
    }

    #[test]
    fn test_input_render_cursor_visible_with_wide_text_scrolling() {
        let mut input = Input::new();
        let width = 20u16;
        let text = "가나다라마바사아자차카타파하";
        input.set_value(text);
        input.focused = true;
        input.handle_input("\x01"); // Ctrl+A - go to start
        for _ in 0..5 {
            input.handle_input("\x1b[C"); // Right
        }
        let lines = input.render(width);
        let line = &lines[0];
        assert!(crate::utils::visible_width(line) <= width as usize);
    }

    // =========================================================================
    // Additional tests from input.test.ts – Undo
    // =========================================================================

    #[test]
    fn test_input_undo_undoes_spaces_one_at_a_time() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o"] {
            input.handle_input(ch);
        }
        input.handle_input(" ");
        input.handle_input(" ");
        assert_eq!(input.get_value(), "hello  ");

        input.handle_input("\x1b[45;5u"); // removes second " "
        assert_eq!(input.get_value(), "hello ");
        input.handle_input("\x1b[45;5u"); // removes first " "
        assert_eq!(input.get_value(), "hello");
        input.handle_input("\x1b[45;5u"); // removes "hello"
        assert_eq!(input.get_value(), "");
    }

    #[test]
    fn test_input_undo_forward_delete() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o"] {
            input.handle_input(ch);
        }
        input.handle_input("\x01"); // Ctrl+A
        input.handle_input("\x1b[C"); // Right
        input.handle_input("\x1b[3~"); // Delete key
        assert_eq!(input.get_value(), "hllo");
        input.handle_input("\x1b[45;5u"); // undo
        assert_eq!(input.get_value(), "hello");
    }

    #[test]
    fn test_input_undo_ctrl_u() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"] {
            input.handle_input(ch);
        }
        input.handle_input("\x01"); // Ctrl+A
        for _ in 0..6 {
            input.handle_input("\x1b[C");
        }
        input.handle_input("\x15"); // Ctrl+U
        assert_eq!(input.get_value(), "world");
        input.handle_input("\x1b[45;5u"); // undo
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn test_input_undo_yank() {
        let mut input = Input::new();
        for ch in ["h", "e", "l", "l", "o", " "] {
            input.handle_input(ch);
        }
        input.handle_input("\x17"); // Ctrl+W - delete "hello "
        input.handle_input("\x19"); // Ctrl+Y - yank
        assert_eq!(input.get_value(), "hello ");
        input.handle_input("\x1b[45;5u"); // undo
        assert_eq!(input.get_value(), "");
    }
}

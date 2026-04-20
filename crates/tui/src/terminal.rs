/// Terminal trait and ProcessTerminal implementation.
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::keys::set_kitty_protocol_active;
use crate::stdin_buffer::{StdinBuffer, StdinBufferOptions, StdinEvent};

/// Minimal terminal interface for TUI.
pub trait Terminal: Send {
    /// Start the terminal with input and resize handlers.
    fn start(
        &mut self,
        on_input: Box<dyn Fn(String) + Send + 'static>,
        on_resize: Box<dyn Fn() + Send + 'static>,
    );

    /// Stop the terminal and restore state.
    fn stop(&mut self);

    /// Write output to terminal.
    fn write(&self, data: &str);

    /// Get terminal width in columns.
    fn columns(&self) -> u16;

    /// Get terminal height in rows.
    fn rows(&self) -> u16;

    /// Whether Kitty keyboard protocol is active.
    fn kitty_protocol_active(&self) -> bool;

    /// Move cursor up (negative) or down (positive) by N lines.
    fn move_by(&self, lines: i32);

    /// Hide the cursor.
    fn hide_cursor(&self);

    /// Show the cursor.
    fn show_cursor(&self);

    /// Clear current line.
    fn clear_line(&self);

    /// Clear from cursor to end of screen.
    fn clear_from_cursor(&self);

    /// Clear entire screen and move cursor to (0,0).
    fn clear_screen(&self);

    /// Set terminal window title.
    fn set_title(&self, title: &str);

    /// Drain pending input for up to `max_ms` milliseconds, idling after
    /// `idle_ms` of silence.
    ///
    /// Call this just before `stop()` to prevent buffered Kitty key-release
    /// events from leaking to the parent shell over slow SSH connections.
    /// Mirrors pi-mono's `Terminal.drainInput()`.
    fn drain_input(&self, max_ms: u64, idle_ms: u64) {
        // Default no-op implementation; CrosstermTerminal overrides this.
        let _ = (max_ms, idle_ms);
    }
}

/// Simple mock terminal for testing.
pub struct MockTerminal {
    pub output: Arc<Mutex<Vec<String>>>,
    pub cols: u16,
    pub rows: u16,
}

impl MockTerminal {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            output: Arc::new(Mutex::new(Vec::new())),
            cols,
            rows,
        }
    }

    pub fn get_output(&self) -> Vec<String> {
        self.output.lock().unwrap().clone()
    }
}

impl Terminal for MockTerminal {
    fn start(
        &mut self,
        _on_input: Box<dyn Fn(String) + Send + 'static>,
        _on_resize: Box<dyn Fn() + Send + 'static>,
    ) {
    }

    fn stop(&mut self) {}

    fn write(&self, data: &str) {
        self.output.lock().unwrap().push(data.to_string());
    }

    fn columns(&self) -> u16 {
        self.cols
    }

    fn rows(&self) -> u16 {
        self.rows
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

/// Real terminal using stdout/stdin via crossterm.
pub struct CrosstermTerminal {
    kitty_active: Arc<AtomicBool>,
    modify_other_keys_active: bool,
    input_thread: Option<std::thread::JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
}

impl CrosstermTerminal {
    pub fn new() -> Self {
        Self {
            kitty_active: Arc::new(AtomicBool::new(false)),
            modify_other_keys_active: false,
            input_thread: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for CrosstermTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal for CrosstermTerminal {
    fn start(
        &mut self,
        on_input: Box<dyn Fn(String) + Send + 'static>,
        on_resize: Box<dyn Fn() + Send + 'static>,
    ) {
        use crossterm::{
            event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
            terminal,
        };

        // Enable raw mode
        let _ = terminal::enable_raw_mode();

        // Enable bracketed paste
        self.write("\x1b[?2004h");

        // Query Kitty protocol
        self.write("\x1b[?u");

        let stop_flag = self.stop_flag.clone();
        let kitty_active = self.kitty_active.clone();

        // Spawn input reading thread
        let handle = std::thread::spawn(move || {
            let mut stdin_buf = StdinBuffer::new(StdinBufferOptions::default());
            let mut kitty_detected = false;

            // Set a fallback timer: if no kitty response, enable modifyOtherKeys
            let start = std::time::Instant::now();
            let kitty_timeout = std::time::Duration::from_millis(150);

            loop {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }

                if !kitty_detected
                    && start.elapsed() >= kitty_timeout
                    && !kitty_active.load(Ordering::SeqCst)
                {
                    // Enable xterm modifyOtherKeys as fallback
                    let _ = std::io::stdout().write_all(b"\x1b[>4;2m");
                    let _ = std::io::stdout().flush();
                    kitty_detected = true; // prevent re-sending
                }

                // Poll for events with short timeout
                match event::poll(std::time::Duration::from_millis(10)) {
                    Ok(true) => {
                        // Read raw bytes from stdin
                        // crossterm's event system handles parsing, but we want raw bytes
                        // For now, translate crossterm events back to sequences
                        match event::read() {
                            Ok(Event::Key(key_event)) => {
                                let seq = key_event_to_sequence(&key_event);
                                if !seq.is_empty() {
                                    // Check for Kitty protocol response
                                    if !kitty_active.load(Ordering::SeqCst) {
                                        let kitty_re =
                                            regex::Regex::new(r"^\x1b\[\?(\d+)u$").unwrap();
                                        if kitty_re.is_match(&seq) {
                                            kitty_active.store(true, Ordering::SeqCst);
                                            set_kitty_protocol_active(true);
                                            // Enable Kitty keyboard protocol flags 1+2+4
                                            let _ = std::io::stdout().write_all(b"\x1b[>7u");
                                            let _ = std::io::stdout().flush();
                                            kitty_detected = true;
                                            continue;
                                        }
                                    }
                                    let events = stdin_buf.process(&seq);
                                    for event in events {
                                        match event {
                                            StdinEvent::Data(s) => on_input(s),
                                            StdinEvent::Paste(s) => {
                                                on_input(format!("\x1b[200~{s}\x1b[201~"))
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(Event::Resize(_, _)) => on_resize(),
                            _ => {}
                        }
                    }
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        });

        self.input_thread = Some(handle);
    }

    fn stop(&mut self) {
        // Drain any pending Kitty key-release events before disabling the
        // protocol — prevents them from leaking to the parent shell over SSH.
        if self.kitty_active.load(Ordering::SeqCst) {
            self.drain_input(1000, 50);
        }

        // Disable bracketed paste
        self.write("\x1b[?2004l");

        // Disable Kitty protocol
        if self.kitty_active.load(Ordering::SeqCst) {
            self.write("\x1b[<u");
            self.kitty_active.store(false, Ordering::SeqCst);
            set_kitty_protocol_active(false);
        }

        // Disable modifyOtherKeys
        if self.modify_other_keys_active {
            self.write("\x1b[>4;0m");
            self.modify_other_keys_active = false;
        }

        // Signal stop
        self.stop_flag.store(true, Ordering::SeqCst);

        // Wait for thread
        if let Some(handle) = self.input_thread.take() {
            let _ = handle.join();
        }

        let _ = crossterm::terminal::disable_raw_mode();
    }

    fn drain_input(&self, max_ms: u64, idle_ms: u64) {
        use crossterm::event::{poll, read};
        use std::time::{Duration, Instant};

        let deadline = Instant::now() + Duration::from_millis(max_ms);
        let idle_dur = Duration::from_millis(idle_ms);

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let timeout = remaining.min(idle_dur);
            match poll(timeout) {
                Ok(true) => {
                    let _ = read(); // consume and discard
                }
                _ => break, // timeout or error → done
            }
        }
    }

    fn write(&self, data: &str) {
        let _ = std::io::stdout().write_all(data.as_bytes());
        let _ = std::io::stdout().flush();
    }

    fn columns(&self) -> u16 {
        crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80)
    }

    fn rows(&self) -> u16 {
        crossterm::terminal::size().map(|(_, h)| h).unwrap_or(24)
    }

    fn kitty_protocol_active(&self) -> bool {
        self.kitty_active.load(Ordering::SeqCst)
    }

    fn move_by(&self, lines: i32) {
        if lines > 0 {
            self.write(&format!("\x1b[{lines}B"));
        } else if lines < 0 {
            self.write(&format!("\x1b[{}A", -lines));
        }
    }

    fn hide_cursor(&self) {
        self.write("\x1b[?25l");
    }

    fn show_cursor(&self) {
        self.write("\x1b[?25h");
    }

    fn clear_line(&self) {
        self.write("\x1b[K");
    }

    fn clear_from_cursor(&self) {
        self.write("\x1b[J");
    }

    fn clear_screen(&self) {
        self.write("\x1b[2J\x1b[H");
    }

    fn set_title(&self, title: &str) {
        self.write(&format!("\x1b]0;{title}\x07"));
    }
}

fn key_event_to_sequence(event: &crossterm::event::KeyEvent) -> String {
    use crossterm::event::{KeyCode, KeyModifiers};

    let mods = event.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let shift = mods.contains(KeyModifiers::SHIFT);
    let alt = mods.contains(KeyModifiers::ALT);

    match event.code {
        KeyCode::Char(c) => {
            if ctrl {
                if c.is_ascii_alphabetic() {
                    let code = (c.to_ascii_lowercase() as u8 - b'a' + 1) as char;
                    return code.to_string();
                }
            }
            if alt {
                return format!("\x1b{c}");
            }
            if shift {
                return c.to_uppercase().to_string();
            }
            c.to_string()
        }
        KeyCode::Enter => "\r".to_string(),
        KeyCode::Tab => {
            if shift {
                "\x1b[Z".to_string()
            } else {
                "\t".to_string()
            }
        }
        KeyCode::Backspace => "\x7f".to_string(),
        KeyCode::Delete => "\x1b[3~".to_string(),
        KeyCode::Esc => "\x1b".to_string(),
        KeyCode::Up => "\x1b[A".to_string(),
        KeyCode::Down => "\x1b[B".to_string(),
        KeyCode::Right => "\x1b[C".to_string(),
        KeyCode::Left => "\x1b[D".to_string(),
        KeyCode::Home => "\x1b[H".to_string(),
        KeyCode::End => "\x1b[F".to_string(),
        KeyCode::PageUp => "\x1b[5~".to_string(),
        KeyCode::PageDown => "\x1b[6~".to_string(),
        KeyCode::Insert => "\x1b[2~".to_string(),
        KeyCode::F(n) => match n {
            1 => "\x1bOP".to_string(),
            2 => "\x1bOQ".to_string(),
            3 => "\x1bOR".to_string(),
            4 => "\x1bOS".to_string(),
            5 => "\x1b[15~".to_string(),
            6 => "\x1b[17~".to_string(),
            7 => "\x1b[18~".to_string(),
            8 => "\x1b[19~".to_string(),
            9 => "\x1b[20~".to_string(),
            10 => "\x1b[21~".to_string(),
            11 => "\x1b[23~".to_string(),
            12 => "\x1b[24~".to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_terminal_write() {
        let term = MockTerminal::new(80, 24);
        term.write("hello");
        let output = term.get_output();
        assert_eq!(output, vec!["hello"]);
    }

    #[test]
    fn test_mock_terminal_dimensions() {
        let term = MockTerminal::new(120, 40);
        assert_eq!(term.columns(), 120);
        assert_eq!(term.rows(), 40);
    }

    #[test]
    fn test_mock_terminal_kitty() {
        let term = MockTerminal::new(80, 24);
        assert!(!term.kitty_protocol_active());
    }
}

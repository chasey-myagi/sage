/// Loader component — animated spinner with message.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::tui::Component;
use crate::components::text::Text;

const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Shared state for the loader, accessible from the timer thread.
struct LoaderState {
    current_frame: usize,
    message: String,
    stopped: bool,
    /// Render-invalidation trigger function (called when frame changes).
    request_render: Option<Box<dyn Fn() + Send>>,
}

type SpinnerColorFn = Box<dyn Fn(&str) -> String + Send + Sync>;
type MessageColorFn = Box<dyn Fn(&str) -> String + Send + Sync>;

pub struct Loader {
    text: Text,
    state: Arc<Mutex<LoaderState>>,
    spinner_color_fn: SpinnerColorFn,
    message_color_fn: MessageColorFn,
}

impl Loader {
    pub fn new<S, M>(
        spinner_color_fn: S,
        message_color_fn: M,
        message: impl Into<String>,
        request_render: Option<Box<dyn Fn() + Send>>,
    ) -> Self
    where
        S: Fn(&str) -> String + Send + Sync + 'static,
        M: Fn(&str) -> String + Send + Sync + 'static,
    {
        let msg = message.into();
        let state = Arc::new(Mutex::new(LoaderState {
            current_frame: 0,
            message: msg.clone(),
            stopped: false,
            request_render,
        }));

        let spinner_fn: SpinnerColorFn = Box::new(spinner_color_fn);
        let message_fn: MessageColorFn = Box::new(message_color_fn);

        // Initial display text
        let initial_text = format!(
            "{} {}",
            spinner_fn(FRAMES[0]),
            message_fn(&msg)
        );

        let mut loader = Self {
            text: Text::new(initial_text, 1, 0),
            state: state.clone(),
            spinner_color_fn: spinner_fn,
            message_color_fn: message_fn,
        };

        // Spawn timer thread
        let state_clone = state;
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(80));
                let mut lock = state_clone.lock().unwrap();
                if lock.stopped {
                    break;
                }
                lock.current_frame = (lock.current_frame + 1) % FRAMES.len();
                if let Some(cb) = &lock.request_render {
                    cb();
                }
            }
        });

        loader
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        let msg = message.into();
        {
            let mut lock = self.state.lock().unwrap();
            lock.message = msg.clone();
        }
        self.update_display();
    }

    pub fn stop(&mut self) {
        let mut lock = self.state.lock().unwrap();
        lock.stopped = true;
    }

    fn update_display(&mut self) {
        let (frame, message) = {
            let lock = self.state.lock().unwrap();
            (lock.current_frame, lock.message.clone())
        };
        let text = format!(
            "{} {}",
            (self.spinner_color_fn)(FRAMES[frame]),
            (self.message_color_fn)(&message)
        );
        self.text.set_text(text);
    }
}

impl Component for Loader {
    fn render(&self, width: u16) -> Vec<String> {
        // Update display text from current state
        let (frame, message) = {
            let lock = self.state.lock().unwrap();
            (lock.current_frame, lock.message.clone())
        };
        let text = format!(
            "{} {}",
            (self.spinner_color_fn)(FRAMES[frame]),
            (self.message_color_fn)(&message)
        );

        // Prepend an empty line as in the TS version: ["", ...super.render(width)]
        let mut lines = vec![String::new()];
        // Create a temporary text component with the current display
        let tmp = Text::new(text, 1, 0);
        lines.extend(tmp.render(width));
        lines
    }

    fn invalidate(&mut self) {
        self.text.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loader_renders() {
        let loader = Loader::new(
            |s: &str| s.to_string(),
            |s: &str| s.to_string(),
            "Loading...",
            None,
        );
        let lines = loader.render(80);
        // Should have at least 1 empty line + content
        assert!(!lines.is_empty());
        assert_eq!(lines[0], "");
    }
}

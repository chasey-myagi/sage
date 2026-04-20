/// CancellableLoader — Loader that can be cancelled with Escape.

use tokio_util::sync::CancellationToken;

use crate::components::loader::Loader;
use crate::keybindings::check_keybinding;
use crate::tui::Component;

pub struct CancellableLoader {
    loader: Loader,
    cancel_token: CancellationToken,
    on_abort: Option<Box<dyn Fn() + Send>>,
}

impl CancellableLoader {
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
        let loader = Loader::new(spinner_color_fn, message_color_fn, message, request_render);
        Self { loader, cancel_token: CancellationToken::new(), on_abort: None }
    }

    /// Set callback for when user presses Escape.
    pub fn set_on_abort<F: Fn() + Send + 'static>(&mut self, cb: F) {
        self.on_abort = Some(Box::new(cb));
    }

    /// Returns a clone of the cancellation token for use with `tokio::select!`.
    pub fn token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    pub fn stop(&mut self) {
        self.loader.stop();
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.loader.set_message(message);
    }

    pub fn handle_input(&mut self, data: &str) {
        if check_keybinding(data, "tui.select.cancel") {
            self.cancel_token.cancel();
            if let Some(cb) = &self.on_abort {
                cb();
            }
        }
    }
}

impl Component for CancellableLoader {
    fn render(&self, width: u16) -> Vec<String> {
        self.loader.render(width)
    }

    fn handle_input(&mut self, data: &str) {
        self.handle_input(data);
    }

    fn invalidate(&mut self) {
        self.loader.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancellable_not_aborted_initially() {
        let cl = CancellableLoader::new(
            |s: &str| s.to_string(),
            |s: &str| s.to_string(),
            "Working...",
            None,
        );
        assert!(!cl.is_cancelled());
    }

    #[test]
    fn test_cancellable_aborts_on_escape() {
        let mut cl = CancellableLoader::new(
            |s: &str| s.to_string(),
            |s: &str| s.to_string(),
            "Working...",
            None,
        );
        cl.handle_input("\x1b"); // Escape
        assert!(cl.is_cancelled());
    }
}

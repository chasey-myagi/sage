//! OAuth provider selector component.
//!
//! Translated from `components/oauth-selector.ts`.
//!
//! Displays a list of OAuth providers for login / logout.

// ============================================================================
// Types
// ============================================================================

/// A registered OAuth provider entry.
#[derive(Debug, Clone)]
pub struct OAuthProviderInfo {
    /// Provider identifier (e.g., `"anthropic"`, `"google"`).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Whether the user is currently logged in.
    pub is_logged_in: bool,
}

/// Mode of the OAuth selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthSelectorMode {
    Login,
    Logout,
}

// ============================================================================
// OAuthSelectorComponent
// ============================================================================

/// Component that renders an OAuth provider selector.
pub struct OAuthSelectorComponent {
    pub mode: OAuthSelectorMode,
    pub providers: Vec<OAuthProviderInfo>,
    pub selected_index: usize,
    on_select: Option<Box<dyn Fn(String) + Send>>,
    on_cancel: Option<Box<dyn Fn() + Send>>,
}

impl OAuthSelectorComponent {
    /// Create a new OAuth selector.
    pub fn new(mode: OAuthSelectorMode, providers: Vec<OAuthProviderInfo>) -> Self {
        Self {
            mode,
            providers,
            selected_index: 0,
            on_select: None,
            on_cancel: None,
        }
    }

    pub fn set_on_select<F: Fn(String) + Send + 'static>(&mut self, f: F) {
        self.on_select = Some(Box::new(f));
    }

    pub fn set_on_cancel<F: Fn() + Send + 'static>(&mut self, f: F) {
        self.on_cancel = Some(Box::new(f));
    }

    /// Move selection up.
    pub fn select_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Move selection down.
    pub fn select_down(&mut self) {
        if !self.providers.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.providers.len() - 1);
        }
    }

    /// Confirm the current selection.
    pub fn confirm(&self) {
        if let Some(provider) = self.providers.get(self.selected_index) {
            if let Some(cb) = &self.on_select {
                cb(provider.id.clone());
            }
        }
    }

    /// Cancel without selecting.
    pub fn cancel(&self) {
        if let Some(cb) = &self.on_cancel {
            cb();
        }
    }

    /// Title string for the current mode.
    pub fn title(&self) -> &'static str {
        match self.mode {
            OAuthSelectorMode::Login => "Select provider to login:",
            OAuthSelectorMode::Logout => "Select provider to logout:",
        }
    }

    /// No-providers message for the current mode.
    pub fn empty_message(&self) -> &'static str {
        match self.mode {
            OAuthSelectorMode::Login => "No OAuth providers available",
            OAuthSelectorMode::Logout => "No OAuth providers logged in. Use /login first.",
        }
    }

    /// Get selected provider, if any.
    pub fn selected_provider(&self) -> Option<&OAuthProviderInfo> {
        self.providers.get(self.selected_index)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_providers() -> Vec<OAuthProviderInfo> {
        vec![
            OAuthProviderInfo {
                id: "anthropic".into(),
                name: "Anthropic".into(),
                is_logged_in: true,
            },
            OAuthProviderInfo {
                id: "google".into(),
                name: "Google".into(),
                is_logged_in: false,
            },
        ]
    }

    #[test]
    fn login_title() {
        let sel = OAuthSelectorComponent::new(OAuthSelectorMode::Login, vec![]);
        assert_eq!(sel.title(), "Select provider to login:");
    }

    #[test]
    fn logout_title() {
        let sel = OAuthSelectorComponent::new(OAuthSelectorMode::Logout, vec![]);
        assert_eq!(sel.title(), "Select provider to logout:");
    }

    #[test]
    fn initial_selection() {
        let sel = OAuthSelectorComponent::new(OAuthSelectorMode::Login, make_providers());
        assert_eq!(
            sel.selected_provider().map(|p| p.id.as_str()),
            Some("anthropic")
        );
    }

    #[test]
    fn navigation() {
        let mut sel = OAuthSelectorComponent::new(OAuthSelectorMode::Login, make_providers());
        sel.select_down();
        assert_eq!(sel.selected_index, 1);
        sel.select_down(); // clamps
        assert_eq!(sel.selected_index, 1);
        sel.select_up();
        assert_eq!(sel.selected_index, 0);
    }

    #[test]
    fn confirm_calls_callback() {
        let mut sel = OAuthSelectorComponent::new(OAuthSelectorMode::Login, make_providers());
        sel.select_down();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let cap2 = captured.clone();
        sel.set_on_select(move |id| *cap2.lock().unwrap() = id);
        sel.confirm();
        assert_eq!(*captured.lock().unwrap(), "google");
    }

    #[test]
    fn empty_message_login() {
        let sel = OAuthSelectorComponent::new(OAuthSelectorMode::Login, vec![]);
        assert!(sel.empty_message().contains("No OAuth providers available"));
    }

    #[test]
    fn empty_message_logout() {
        let sel = OAuthSelectorComponent::new(OAuthSelectorMode::Logout, vec![]);
        assert!(sel.empty_message().contains("Use /login first"));
    }
}

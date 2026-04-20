//! Thinking level selector component.
//!
//! Translated from `components/thinking-selector.ts`.
//!
//! Displays a list of thinking levels for selection.

// ============================================================================
// ThinkingLevel
// ============================================================================

/// Available thinking depth levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

impl ThinkingLevel {
    /// All levels in order.
    pub const ALL: &'static [ThinkingLevel] = &[
        ThinkingLevel::Off,
        ThinkingLevel::Minimal,
        ThinkingLevel::Low,
        ThinkingLevel::Medium,
        ThinkingLevel::High,
        ThinkingLevel::Xhigh,
    ];

    /// Display name (matches TypeScript `value`/`label`).
    pub fn as_str(self) -> &'static str {
        match self {
            ThinkingLevel::Off => "off",
            ThinkingLevel::Minimal => "minimal",
            ThinkingLevel::Low => "low",
            ThinkingLevel::Medium => "medium",
            ThinkingLevel::High => "high",
            ThinkingLevel::Xhigh => "xhigh",
        }
    }

    /// Human-readable description.
    pub fn description(self) -> &'static str {
        match self {
            ThinkingLevel::Off => "No reasoning",
            ThinkingLevel::Minimal => "Very brief reasoning (~1k tokens)",
            ThinkingLevel::Low => "Light reasoning (~2k tokens)",
            ThinkingLevel::Medium => "Moderate reasoning (~8k tokens)",
            ThinkingLevel::High => "Deep reasoning (~16k tokens)",
            ThinkingLevel::Xhigh => "Maximum reasoning (~32k tokens)",
        }
    }

    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "off" => Some(ThinkingLevel::Off),
            "minimal" => Some(ThinkingLevel::Minimal),
            "low" => Some(ThinkingLevel::Low),
            "medium" => Some(ThinkingLevel::Medium),
            "high" => Some(ThinkingLevel::High),
            "xhigh" => Some(ThinkingLevel::Xhigh),
            _ => None,
        }
    }
}

impl std::fmt::Display for ThinkingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// ThinkingSelectorComponent
// ============================================================================

/// Component that renders a thinking level selector.
pub struct ThinkingSelectorComponent {
    pub current_level: ThinkingLevel,
    pub available_levels: Vec<ThinkingLevel>,
    pub selected_index: usize,
    on_select: Option<Box<dyn Fn(ThinkingLevel) + Send>>,
    on_cancel: Option<Box<dyn Fn() + Send>>,
}

impl ThinkingSelectorComponent {
    /// Create a new thinking selector.
    pub fn new(current_level: ThinkingLevel, available_levels: Vec<ThinkingLevel>) -> Self {
        let selected_index = available_levels
            .iter()
            .position(|&l| l == current_level)
            .unwrap_or(0);
        Self {
            current_level,
            available_levels,
            selected_index,
            on_select: None,
            on_cancel: None,
        }
    }

    pub fn set_on_select<F: Fn(ThinkingLevel) + Send + 'static>(&mut self, f: F) {
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
        if !self.available_levels.is_empty() {
            self.selected_index =
                (self.selected_index + 1).min(self.available_levels.len() - 1);
        }
    }

    /// Confirm the current selection.
    pub fn confirm(&self) {
        if let Some(level) = self.available_levels.get(self.selected_index) {
            if let Some(cb) = &self.on_select {
                cb(*level);
            }
        }
    }

    /// Cancel without selecting.
    pub fn cancel(&self) {
        if let Some(cb) = &self.on_cancel {
            cb();
        }
    }

    /// Get the currently highlighted level.
    pub fn selected_level(&self) -> Option<ThinkingLevel> {
        self.available_levels.get(self.selected_index).copied()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_levels_have_descriptions() {
        for level in ThinkingLevel::ALL {
            assert!(!level.description().is_empty());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for level in ThinkingLevel::ALL {
            let s = level.as_str();
            assert_eq!(ThinkingLevel::from_str(s), Some(*level));
        }
    }

    #[test]
    fn from_str_invalid() {
        assert!(ThinkingLevel::from_str("superduper").is_none());
    }

    #[test]
    fn initial_selection_matches_current() {
        let available = vec![ThinkingLevel::Low, ThinkingLevel::Medium, ThinkingLevel::High];
        let sel = ThinkingSelectorComponent::new(ThinkingLevel::Medium, available);
        assert_eq!(sel.selected_index, 1);
        assert_eq!(sel.selected_level(), Some(ThinkingLevel::Medium));
    }

    #[test]
    fn navigation() {
        let available = ThinkingLevel::ALL.to_vec();
        let mut sel = ThinkingSelectorComponent::new(ThinkingLevel::Off, available);
        assert_eq!(sel.selected_index, 0);
        sel.select_down();
        assert_eq!(sel.selected_level(), Some(ThinkingLevel::Minimal));
        sel.select_up();
        assert_eq!(sel.selected_level(), Some(ThinkingLevel::Off));
    }

    #[test]
    fn confirm_calls_callback() {
        let available = vec![ThinkingLevel::Low, ThinkingLevel::High];
        let mut sel = ThinkingSelectorComponent::new(ThinkingLevel::Low, available);
        sel.select_down();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let cap2 = captured.clone();
        sel.set_on_select(move |l| *cap2.lock().unwrap() = Some(l));
        sel.confirm();
        assert_eq!(*captured.lock().unwrap(), Some(ThinkingLevel::High));
    }

    #[test]
    fn display_format() {
        assert_eq!(format!("{}", ThinkingLevel::Xhigh), "xhigh");
    }
}

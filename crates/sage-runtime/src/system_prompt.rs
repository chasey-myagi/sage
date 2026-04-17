// SystemPromptBuilder — structured system prompt assembly.
//
// Allows upper-layer projects to compose system prompts from named sections
// without hand-rolling string concatenation. Aligned with pi-mono's
// buildSystemPrompt() pattern but kept simple: sections → flat String,
// no provider-specific cache_control plumbing at this layer.

/// A single section of a system prompt.
#[derive(Debug, Clone)]
pub struct PromptSection {
    pub name: &'static str,
    pub content: String,
    /// Whether this section should be marked for prompt caching when
    /// the provider supports structured system prompts (future use).
    pub cacheable: bool,
}

/// A structured system prompt composed of named sections.
///
/// For providers that only accept a flat `String`, call `.to_string()`.
#[derive(Debug, Clone, Default)]
pub struct SystemPrompt {
    sections: Vec<PromptSection>,
}

impl SystemPrompt {
    pub fn builder() -> SystemPromptBuilder {
        SystemPromptBuilder::new()
    }

    pub fn sections(&self) -> &[PromptSection] {
        &self.sections
    }

    pub fn is_empty(&self) -> bool {
        self.sections.iter().all(|s| s.content.is_empty())
    }
}

impl std::fmt::Display for SystemPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let joined = self
            .sections
            .iter()
            .filter(|s| !s.content.is_empty())
            .map(|s| s.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        f.write_str(&joined)
    }
}

impl From<SystemPrompt> for String {
    fn from(sp: SystemPrompt) -> Self {
        sp.to_string()
    }
}

impl From<&str> for SystemPrompt {
    fn from(s: &str) -> Self {
        SystemPrompt {
            sections: vec![PromptSection {
                name: "prompt",
                content: s.to_string(),
                cacheable: false,
            }],
        }
    }
}

impl From<String> for SystemPrompt {
    fn from(s: String) -> Self {
        SystemPrompt {
            sections: vec![PromptSection {
                name: "prompt",
                content: s,
                cacheable: false,
            }],
        }
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for [`SystemPrompt`].
///
/// # Example
/// ```rust
/// # use sage_runtime::SystemPrompt;
/// let prompt = SystemPrompt::builder()
///     .cacheable_section("base", "You are a helpful assistant.")
///     .section("context", "Project context goes here.")
///     .build();
/// assert!(!prompt.is_empty());
/// ```
#[derive(Debug, Default)]
pub struct SystemPromptBuilder {
    sections: Vec<PromptSection>,
}

impl SystemPromptBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a section (not marked for caching).
    pub fn section(mut self, name: &'static str, content: impl Into<String>) -> Self {
        self.sections.push(PromptSection {
            name,
            content: content.into(),
            cacheable: false,
        });
        self
    }

    /// Append a section marked as a cache breakpoint (for future provider support).
    pub fn cacheable_section(mut self, name: &'static str, content: impl Into<String>) -> Self {
        self.sections.push(PromptSection {
            name,
            content: content.into(),
            cacheable: true,
        });
        self
    }

    pub fn build(self) -> SystemPrompt {
        SystemPrompt {
            sections: self.sections,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_builder_produces_empty_string() {
        let sp = SystemPrompt::builder().build();
        assert!(sp.is_empty());
        assert_eq!(sp.to_string(), "");
    }

    #[test]
    fn single_section() {
        let sp = SystemPrompt::builder()
            .section("base", "You are a helpful assistant.")
            .build();
        assert_eq!(sp.to_string(), "You are a helpful assistant.");
    }

    #[test]
    fn multiple_sections_joined_with_double_newline() {
        let sp = SystemPrompt::builder()
            .section("base", "You are a helpful assistant.")
            .section("context", "Project: Sage")
            .build();
        assert_eq!(
            sp.to_string(),
            "You are a helpful assistant.\n\nProject: Sage"
        );
    }

    #[test]
    fn empty_sections_skipped() {
        let sp = SystemPrompt::builder()
            .section("base", "You are a helpful assistant.")
            .section("optional", "")
            .section("tail", "End.")
            .build();
        assert_eq!(sp.to_string(), "You are a helpful assistant.\n\nEnd.");
    }

    #[test]
    fn cacheable_section_sets_flag() {
        let sp = SystemPrompt::builder()
            .cacheable_section("base", "System.")
            .section("extra", "Extra.")
            .build();
        assert!(sp.sections()[0].cacheable);
        assert!(!sp.sections()[1].cacheable);
    }

    #[test]
    fn from_str_conversion() {
        let sp: SystemPrompt = "You are a helpful assistant.".into();
        assert_eq!(sp.to_string(), "You are a helpful assistant.");
    }

    #[test]
    fn into_string_conversion() {
        let sp = SystemPrompt::builder()
            .section("base", "hello")
            .build();
        let s: String = sp.into();
        assert_eq!(s, "hello");
    }
}

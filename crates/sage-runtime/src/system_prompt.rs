// SystemPromptBuilder — structured system prompt assembly.
//
// Allows upper-layer projects to compose system prompts from named sections
// without hand-rolling string concatenation. Aligned with pi-mono's
// buildSystemPrompt() pattern but kept simple: sections → flat String,
// no provider-specific cache_control plumbing at this layer.

/// Platform-level "how a Sage agent works" methodology.
///
/// Every Sage agent shares this section — it defines the discover /
/// execute / sediment / evolve skeleton that makes a Sage agent what it
/// is. Per-agent configs contribute only the domain `goal` (one-line
/// "what this agent does for me"); the methodology is not a per-agent
/// choice.
///
/// Baked into the binary via `include_str!` so every driver (CLI chat,
/// daemon, channel adapters) picks up the same text without file-system
/// lookups at runtime.
pub const SAGE_CORE_PROMPT: &str = include_str!("core_prompt.md");

/// Compose the default Sage system prompt from a per-agent goal.
///
/// Output layout:
///   <SAGE_CORE_PROMPT>
///
///   ## Your goal
///
///   <goal>
///
/// Drivers may append memory / skill-index / etc. sections using the
/// [`SystemPromptBuilder`] API.
pub fn compose_sage_prompt(goal: &str) -> String {
    let goal = goal.trim();
    if goal.is_empty() {
        return SAGE_CORE_PROMPT.trim_end().to_string();
    }
    format!(
        "{}\n\n## Your goal\n\n{}",
        SAGE_CORE_PROMPT.trim_end(),
        goal
    )
}

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
    fn sage_core_prompt_is_non_empty_and_mentions_pillars() {
        // The methodology is load-bearing — any regression that guts it
        // should fail fast at test time rather than silently ship an
        // empty skeleton.
        assert!(!SAGE_CORE_PROMPT.is_empty());
        for pillar in ["发现信息", "执行任务", "沉淀信息", "根据用户历史优化"] {
            assert!(
                SAGE_CORE_PROMPT.contains(pillar),
                "core prompt must keep the '{pillar}' pillar"
            );
        }
    }

    #[test]
    fn compose_sage_prompt_includes_core_and_goal() {
        let out = compose_sage_prompt("替我完成飞书上的日常操作");
        assert!(out.contains("发现信息"), "core skeleton must be present");
        assert!(out.contains("## Your goal"), "goal section header present");
        assert!(out.contains("替我完成飞书上的日常操作"), "goal text appears");
    }

    #[test]
    fn compose_sage_prompt_empty_goal_returns_core_only() {
        // Defensive — if a caller forgets to set goal the core skeleton
        // still ships, rather than emitting a stray "## Your goal" header
        // with no body.
        let out = compose_sage_prompt("   ");
        assert!(!out.contains("## Your goal"));
        assert!(out.contains("发现信息"));
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

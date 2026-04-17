// context.rs — Pure functions for building agent context (system prompt).
//
// Separated from serve.rs so they can be unit-tested without file I/O.
// serve.rs loads files from disk, then calls these functions to compose the final prompt.

// ── Memory injection ──────────────────────────────────────────────────────────

/// Prepend memory sections to a base system prompt.
///
/// Each section is a `(label, content)` pair where `label` is the file name
/// (e.g. `"AGENT.md"`) and `content` is the file's text. Sections with
/// empty or whitespace-only content are silently skipped. The remaining
/// sections are placed before the base prompt, separated by a `---` divider.
///
/// Returns the base prompt unchanged when `sections` is empty.
pub fn prepend_memory_sections(base: &str, sections: &[(&str, &str)]) -> String {
    let non_empty: Vec<(&str, &str)> = sections
        .iter()
        .filter(|(_, content)| !content.trim().is_empty())
        .copied()
        .collect();

    if non_empty.is_empty() {
        return base.to_string();
    }

    let mut parts: Vec<String> = non_empty
        .iter()
        .map(|(label, content)| {
            if label.is_empty() {
                content.trim().to_string()
            } else {
                format!("### {label}\n\n{}", content.trim())
            }
        })
        .collect();

    if !base.is_empty() {
        parts.push(base.to_string());
    }

    parts.join("\n\n---\n\n")
}

// ── Skill injection ───────────────────────────────────────────────────────────
//
// Task #88: removed. Skills are no longer auto-injected into the system
// prompt. Agents navigate `workspace/skills/INDEX.md` + read SKILL.md
// bodies on demand via the file tool. If a future design wants to surface
// a skill index in the prompt, add a new helper here — don't resurrect
// the always-on body-injection path.

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── prepend_memory_sections ──────────────────────────────────────────────

    #[test]
    fn no_sections_returns_base_prompt_unchanged() {
        let base = "You are a helpful assistant.";
        let result = prepend_memory_sections(base, &[]);
        assert_eq!(result, base);
    }

    #[test]
    fn single_section_prepended_before_base_prompt() {
        let base = "You are a helpful assistant.";
        let sections = [("AGENT.md", "# FeishuAgent\nYou handle Feishu messages.")];
        let result = prepend_memory_sections(base, &sections);
        // Memory content must appear before the base prompt
        let memory_pos = result.find("FeishuAgent").expect("memory content missing");
        let base_pos = result.find("helpful assistant").expect("base prompt missing");
        assert!(memory_pos < base_pos, "memory must come before base prompt");
    }

    #[test]
    fn single_section_includes_label() {
        let base = "base";
        let sections = [("AGENT.md", "agent content")];
        let result = prepend_memory_sections(base, &sections);
        assert!(result.contains("AGENT.md"), "section label must appear in output");
    }

    #[test]
    fn base_prompt_preserved_after_sections() {
        let base = "UNIQUE_BASE_CONTENT_XYZ";
        let sections = [("AGENT.md", "memory content")];
        let result = prepend_memory_sections(base, &sections);
        assert!(result.contains(base), "base prompt must be preserved in output");
    }

    #[test]
    fn empty_content_section_is_skipped() {
        let base = "base prompt";
        let sections = [("AGENT.md", ""), ("MEMORY.md", "real content")];
        let result = prepend_memory_sections(base, &sections);
        // Empty file should not contribute label or content
        assert!(!result.contains("AGENT.md"), "empty section label should be omitted");
        assert!(result.contains("MEMORY.md"), "non-empty section must appear");
    }

    #[test]
    fn whitespace_only_content_section_is_skipped() {
        let base = "base";
        let sections = [("AGENT.md", "   \n\t  \n")];
        let result = prepend_memory_sections(base, &sections);
        assert_eq!(result, base, "whitespace-only content should leave prompt unchanged");
    }

    #[test]
    fn multiple_sections_all_appear_in_result() {
        let base = "base";
        let sections = [
            ("AGENT.md", "agent instructions"),
            ("MEMORY.md", "memory entries"),
        ];
        let result = prepend_memory_sections(base, &sections);
        assert!(result.contains("agent instructions"), "first section content missing");
        assert!(result.contains("memory entries"), "second section content missing");
    }

    #[test]
    fn sections_appear_in_original_order() {
        let base = "base";
        let sections = [
            ("FIRST.md", "alpha content"),
            ("SECOND.md", "beta content"),
        ];
        let result = prepend_memory_sections(base, &sections);
        let alpha_pos = result.find("alpha content").unwrap();
        let beta_pos = result.find("beta content").unwrap();
        assert!(alpha_pos < beta_pos, "sections must preserve input order");
    }

    #[test]
    fn separator_between_memory_and_base_prompt() {
        let base = "UNIQUE_BASE_SENTINEL";
        let sections = [("AGENT.md", "UNIQUE_MEM_SENTINEL")];
        let result = prepend_memory_sections(base, &sections);
        let mem_end = result.find("UNIQUE_MEM_SENTINEL").unwrap() + "UNIQUE_MEM_SENTINEL".len();
        let base_start = result.find("UNIQUE_BASE_SENTINEL").unwrap();
        // The characters between mem content and base prompt must include a separator (not just whitespace)
        let between = &result[mem_end..base_start];
        assert!(
            between.contains("---") || between.contains("===") || !between.trim().is_empty(),
            "must have separator between memory content and base prompt; got: {:?}",
            between
        );
        // Concrete check: at least a blank line or a --- divider between them
        assert!(between.contains('\n'), "separator must include a newline");
    }

    #[test]
    fn all_empty_sections_returns_base_unchanged() {
        let base = "base prompt";
        let sections = [("A.md", ""), ("B.md", "  "), ("C.md", "\n")];
        let result = prepend_memory_sections(base, &sections);
        assert_eq!(result, base);
    }

    #[test]
    fn empty_base_with_sections_produces_just_section_content() {
        let base = "";
        let sections = [("AGENT.md", "section content")];
        let result = prepend_memory_sections(base, &sections);
        assert!(result.contains("section content"), "section content must appear");
        // No leading separator artifact — result should not start with --- if base is empty
        // (implementation detail: just don't panic or produce garbage)
    }

    #[test]
    fn multiple_nonempty_sections_have_separators_between_them() {
        let base = "base";
        let sections = [("A.md", "content-a"), ("B.md", "content-b")];
        let result = prepend_memory_sections(base, &sections);
        let pos_a = result.find("content-a").unwrap();
        let pos_b = result.find("content-b").unwrap();
        // There must be at least a newline between the two sections
        let between = &result[pos_a + "content-a".len()..pos_b];
        assert!(between.contains('\n'), "sections must be separated by newlines");
    }

    #[test]
    fn section_content_containing_dashes_not_confused_with_separator() {
        let base = "base prompt";
        let sections = [("NOTES.md", "item one\n---\nitem two")];
        let result = prepend_memory_sections(base, &sections);
        // Both items should appear, and base should still be present
        assert!(result.contains("item one"), "content before --- must appear");
        assert!(result.contains("item two"), "content after --- must appear");
        assert!(result.contains(base), "base prompt must still be present");
    }

    // ── append_skill_sections: removed in task #88 ────────────────────────
    // The auto-injection path is gone. Memory/prepend tests above are the
    // only contract `build_system_prompt` still honours.
}

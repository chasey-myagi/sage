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

/// A loaded skill file.
pub struct SkillEntry<'a> {
    pub name: &'a str,
    pub content: &'a str,
}

/// Append skill definitions to a base system prompt.
///
/// Each entry provides a skill name (derived from the filename without extension)
/// and its markdown content. Skills with empty/whitespace-only content are skipped.
/// When skills are present, they are appended after the base prompt under a
/// `## Available Skills` section header.
///
/// Returns the base prompt unchanged when `skills` is empty (after filtering).
pub fn append_skill_sections<'a>(base: &str, skills: &[SkillEntry<'a>]) -> String {
    let non_empty: Vec<&SkillEntry<'a>> = skills
        .iter()
        .filter(|s| !s.content.trim().is_empty())
        .collect();

    if non_empty.is_empty() {
        return base.to_string();
    }

    let skill_blocks: Vec<String> = non_empty
        .iter()
        .map(|s| format!("### {}\n\n{}", s.name, s.content.trim()))
        .collect();

    let skills_section = format!("## Available Skills\n\n{}", skill_blocks.join("\n\n---\n\n"));

    if base.is_empty() {
        skills_section
    } else {
        format!("{base}\n\n---\n\n{skills_section}")
    }
}

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

    // ── append_skill_sections ────────────────────────────────────────────────

    #[test]
    fn no_skills_returns_base_prompt_unchanged() {
        let base = "You are a helpful assistant.";
        let result = append_skill_sections(base, &[]);
        assert_eq!(result, base);
    }

    #[test]
    fn single_skill_appended_after_base_prompt() {
        let base = "You are a helpful assistant.";
        let skills = [SkillEntry { name: "calendar", content: "Use this to check schedules." }];
        let result = append_skill_sections(base, &skills);
        let base_pos = result.find("helpful assistant").unwrap();
        let skill_pos = result.find("calendar").unwrap();
        assert!(skill_pos > base_pos, "skill must come after base prompt");
    }

    #[test]
    fn skill_section_has_available_skills_header() {
        let base = "base";
        let skills = [SkillEntry { name: "my-skill", content: "skill content" }];
        let result = append_skill_sections(base, &skills);
        // Must have the standard "Available Skills" section header
        assert!(
            result.contains("Available Skills"),
            "skill section must contain 'Available Skills' header"
        );
    }

    #[test]
    fn skill_name_appears_in_output() {
        let base = "base";
        let skills = [SkillEntry { name: "feishu-reply", content: "reply to feishu" }];
        let result = append_skill_sections(base, &skills);
        assert!(result.contains("feishu-reply"), "skill name must appear in output");
    }

    #[test]
    fn skill_content_appears_in_output() {
        let base = "base";
        let skills = [SkillEntry { name: "skill", content: "UNIQUE_SKILL_CONTENT_ABC" }];
        let result = append_skill_sections(base, &skills);
        assert!(result.contains("UNIQUE_SKILL_CONTENT_ABC"));
    }

    #[test]
    fn empty_skill_content_is_skipped() {
        let base = "base";
        let skills = [
            SkillEntry { name: "empty-skill", content: "" },
            SkillEntry { name: "real-skill", content: "real content" },
        ];
        let result = append_skill_sections(base, &skills);
        assert!(!result.contains("empty-skill"), "empty skill must be omitted");
        assert!(result.contains("real-skill"), "non-empty skill must appear");
    }

    #[test]
    fn whitespace_only_skill_content_is_skipped() {
        let base = "base";
        let skills = [SkillEntry { name: "blank", content: "\n  \t  \n" }];
        let result = append_skill_sections(base, &skills);
        assert_eq!(result, base, "whitespace-only skill should leave prompt unchanged");
    }

    #[test]
    fn multiple_skills_all_appear_in_result() {
        let base = "base";
        let skills = [
            SkillEntry { name: "skill-a", content: "content-a" },
            SkillEntry { name: "skill-b", content: "content-b" },
        ];
        let result = append_skill_sections(base, &skills);
        assert!(result.contains("skill-a") && result.contains("content-a"));
        assert!(result.contains("skill-b") && result.contains("content-b"));
    }

    #[test]
    fn base_prompt_preserved_with_skills() {
        let base = "UNIQUE_PRESERVED_BASE";
        let skills = [SkillEntry { name: "s", content: "c" }];
        let result = append_skill_sections(base, &skills);
        assert!(result.contains(base));
    }

    #[test]
    fn multiple_skills_preserve_input_order() {
        let base = "base";
        let skills = [
            SkillEntry { name: "alpha-skill", content: "alpha content" },
            SkillEntry { name: "beta-skill", content: "beta content" },
        ];
        let result = append_skill_sections(base, &skills);
        let alpha_pos = result.find("alpha-skill").unwrap();
        let beta_pos = result.find("beta-skill").unwrap();
        assert!(alpha_pos < beta_pos, "skills must preserve input order");
    }

    #[test]
    fn available_skills_header_appears_exactly_once_with_multiple_skills() {
        let base = "base";
        let skills = [
            SkillEntry { name: "skill-a", content: "content a" },
            SkillEntry { name: "skill-b", content: "content b" },
            SkillEntry { name: "skill-c", content: "content c" },
        ];
        let result = append_skill_sections(base, &skills);
        let count = result.matches("Available Skills").count();
        assert_eq!(count, 1, "'Available Skills' header must appear exactly once, found {count}");
    }

    #[test]
    fn all_empty_skills_returns_base_unchanged() {
        let base = "base prompt";
        let skills = [
            SkillEntry { name: "a", content: "" },
            SkillEntry { name: "b", content: "  \n  " },
        ];
        let result = append_skill_sections(base, &skills);
        assert_eq!(result, base, "all empty skills must leave base prompt unchanged");
    }

    #[test]
    fn empty_base_with_skills_produces_skills_content() {
        let base = "";
        let skills = [SkillEntry { name: "my-skill", content: "useful skill" }];
        let result = append_skill_sections(base, &skills);
        assert!(result.contains("my-skill"), "skill name must appear even with empty base");
        assert!(result.contains("useful skill"), "skill content must appear");
    }
}

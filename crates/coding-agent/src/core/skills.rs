//! Skills discovery and loading.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/skills.ts`.
//!
//! Skills are Markdown files (with YAML frontmatter) discovered from the
//! agent dir and current project. Each skill lives in its own directory as
//! `SKILL.md`, or as a plain `.md` file in a skill root directory.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::utils::frontmatter::parse_frontmatter_as;

// ============================================================================
// Constants
// ============================================================================

const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;

const IGNORE_FILE_NAMES: &[&str] = &[".gitignore", ".ignore", ".fdignore"];

// ============================================================================
// Types
// ============================================================================

/// Frontmatter extracted from a skill file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "disable-model-invocation")]
    pub disable_model_invocation: bool,
}

/// A loaded skill.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub file_path: PathBuf,
    pub base_dir: PathBuf,
    /// Source identifier: `"user"`, `"project"`, `"path"`, or a package name.
    pub source: String,
    pub disable_model_invocation: bool,
    /// Raw content of the skill file (after frontmatter is stripped).
    pub content: String,
}

/// A diagnostic message about a skill load attempt.
#[derive(Debug, Clone)]
pub struct SkillDiagnostic {
    pub file_path: PathBuf,
    pub message: String,
}

/// Result of loading skills from a directory.
#[derive(Debug, Default)]
pub struct LoadSkillsResult {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

// ============================================================================
// Frontmatter parsing
// ============================================================================

/// Parse the YAML frontmatter of a skill file into a [`SkillFrontmatter`],
/// returning the typed frontmatter and the body. Delegates to the shared
/// `utils::frontmatter` parser so multi-line descriptions and other valid
/// YAML constructs are handled correctly.
fn parse_frontmatter(content: &str) -> (SkillFrontmatter, String) {
    // Strip BOM if present so frontmatter detection still works on UTF-8 BOM
    // files saved by editors on Windows.
    let content = content.trim_start_matches('\u{FEFF}');
    parse_frontmatter_as::<SkillFrontmatter>(content)
}

// ============================================================================
// Validation
// ============================================================================

fn validate_name(name: &str, parent_dir_name: &str) -> Vec<String> {
    let mut errors = Vec::new();
    if name != parent_dir_name {
        errors.push(format!(
            "name \"{}\" does not match parent directory \"{}\"",
            name, parent_dir_name
        ));
    }
    if name.len() > MAX_NAME_LENGTH {
        errors.push(format!(
            "name exceeds {} characters ({})",
            MAX_NAME_LENGTH,
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        errors.push(
            "name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)"
                .to_string(),
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        errors.push("name must not start or end with a hyphen".to_string());
    }
    if name.contains("--") {
        errors.push("name must not contain consecutive hyphens".to_string());
    }
    errors
}

fn validate_description(description: Option<&str>) -> Vec<String> {
    let mut errors = Vec::new();
    match description {
        None | Some("") => errors.push("description is required".to_string()),
        Some(d) if d.trim().is_empty() => errors.push("description is required".to_string()),
        Some(d) if d.len() > MAX_DESCRIPTION_LENGTH => errors.push(format!(
            "description exceeds {} characters ({})",
            MAX_DESCRIPTION_LENGTH,
            d.len()
        )),
        _ => {}
    }
    errors
}

// ============================================================================
// XML escaping
// ============================================================================

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ============================================================================
// Prompt formatting
// ============================================================================

/// Format skills for inclusion in a system prompt (XML format).
///
/// Skills with `disable_model_invocation = true` are excluded — they can only
/// be invoked via explicit `/skill:name` commands.
///
/// Translated from pi-mono `packages/coding-agent/src/core/skills.ts`
/// `formatSkillsForPrompt`.
pub fn format_skills_for_prompt(skills: &[Skill]) -> String {
    let visible: Vec<&Skill> = skills
        .iter()
        .filter(|s| !s.disable_model_invocation)
        .collect();

    if visible.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = vec![
        String::new(),
        String::new(),
        "The following skills provide specialized instructions for specific tasks.".to_string(),
        "Use the read tool to load a skill's file when the task matches its description.".to_string(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".to_string(),
        String::new(),
        "<available_skills>".to_string(),
    ];

    for skill in &visible {
        lines.push("  <skill>".to_string());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&skill.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&skill.file_path.to_string_lossy())
        ));
        lines.push("  </skill>".to_string());
    }

    lines.push("</available_skills>".to_string());

    lines.join("\n")
}

// ============================================================================
// Loading
// ============================================================================

/// Try to read a `SKILL.md` or plain `.md` file and produce a Skill.
pub fn load_skill_from_file(
    file_path: &Path,
    source: &str,
) -> (Option<Skill>, Vec<SkillDiagnostic>) {
    let mut diagnostics = Vec::new();

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            diagnostics.push(SkillDiagnostic {
                file_path: file_path.to_path_buf(),
                message: format!("Failed to read: {e}"),
            });
            return (None, diagnostics);
        }
    };

    let (fm, body) = parse_frontmatter(&content);

    // Parent directory name is used as the skill name fallback
    let parent_name = file_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let is_skill_md = file_path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md");

    // For SKILL.md: name must match parent dir; for plain .md: use file stem
    let effective_name = fm.name.clone().unwrap_or_else(|| {
        if is_skill_md {
            parent_name.clone()
        } else {
            file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        }
    });

    // Validate name only for SKILL.md files
    if is_skill_md {
        let name_errors = validate_name(&effective_name, &parent_name);
        for e in name_errors {
            diagnostics.push(SkillDiagnostic {
                file_path: file_path.to_path_buf(),
                message: e,
            });
        }
    }

    // Validate description
    let desc_errors = validate_description(fm.description.as_deref());
    for e in desc_errors {
        diagnostics.push(SkillDiagnostic {
            file_path: file_path.to_path_buf(),
            message: e,
        });
    }

    let description = fm.description.clone().unwrap_or_default();
    let base_dir = file_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let skill = Skill {
        name: effective_name,
        description,
        file_path: file_path.to_path_buf(),
        base_dir,
        source: source.to_string(),
        disable_model_invocation: fm.disable_model_invocation,
        content: body,
    };

    (Some(skill), diagnostics)
}

/// Options for loading skills from a directory.
pub struct LoadSkillsFromDirOptions {
    pub dir: PathBuf,
    pub source: String,
}

/// Load skills from a directory.
///
/// Discovery rules:
/// - If a directory contains `SKILL.md`, treat it as a skill root and do not recurse further.
/// - Otherwise load direct `.md` children in the root.
/// - Recurse into subdirectories to find `SKILL.md`.
pub fn load_skills_from_dir(options: LoadSkillsFromDirOptions) -> LoadSkillsResult {
    load_skills_from_dir_internal(&options.dir, &options.source, true)
}

fn should_ignore(path: &Path, ignore_patterns: &[String]) -> bool {
    let path_str = path.to_string_lossy();
    ignore_patterns.iter().any(|p| {
        // Simple prefix-match ignore (full gitignore semantics would need the `ignore` crate)
        path_str.contains(p.as_str())
    })
}

fn collect_ignore_patterns(dir: &Path) -> Vec<String> {
    let mut patterns = Vec::new();
    for filename in IGNORE_FILE_NAMES {
        let ignore_path = dir.join(filename);
        if let Ok(content) = std::fs::read_to_string(&ignore_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                patterns.push(trimmed.to_string());
            }
        }
    }
    patterns
}

fn load_skills_from_dir_internal(
    dir: &Path,
    source: &str,
    include_root_files: bool,
) -> LoadSkillsResult {
    let mut result = LoadSkillsResult::default();

    if !dir.exists() {
        return result;
    }

    let ignore_patterns = collect_ignore_patterns(dir);

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return result,
    };

    let entries: Vec<_> = entries.flatten().collect();

    // First check if SKILL.md exists at this level
    for entry in &entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str != "SKILL.md" {
            continue;
        }
        let full_path = entry.path();
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            || entry.file_type().map(|t| t.is_symlink()).unwrap_or(false)
                && std::fs::metadata(&full_path)
                    .map(|m| m.is_file())
                    .unwrap_or(false);
        if !is_file {
            continue;
        }
        if should_ignore(&full_path, &ignore_patterns) {
            continue;
        }
        let (skill, diags) = load_skill_from_file(&full_path, source);
        if let Some(s) = skill {
            result.skills.push(s);
        }
        result.diagnostics.extend(diags);
        return result;
    }

    // No SKILL.md — look at children
    for entry in &entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }

        let full_path = entry.path();
        if should_ignore(&full_path, &ignore_patterns) {
            continue;
        }

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        let (is_dir, is_file) = if file_type.is_symlink() {
            let meta = match std::fs::metadata(&full_path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            (meta.is_dir(), meta.is_file())
        } else {
            (file_type.is_dir(), file_type.is_file())
        };

        if is_dir {
            let sub = load_skills_from_dir_internal(&full_path, source, false);
            result.skills.extend(sub.skills);
            result.diagnostics.extend(sub.diagnostics);
            continue;
        }

        if is_file && include_root_files && name_str.ends_with(".md") {
            let (skill, diags) = load_skill_from_file(&full_path, source);
            if let Some(s) = skill {
                result.skills.push(s);
            }
            result.diagnostics.extend(diags);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, name: &str, content: &str) -> PathBuf {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let path = skill_dir.join("SKILL.md");
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_skill_from_valid_file() {
        let tmp = TempDir::new().unwrap();
        let path = write_skill(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: A test skill\n---\n\nSkill content here.",
        );
        let (skill, diags) = load_skill_from_file(&path, "user");
        assert!(skill.is_some());
        let s = skill.unwrap();
        assert_eq!(s.name, "my-skill");
        assert_eq!(s.description, "A test skill");
        assert!(s.content.contains("Skill content here"));
        assert!(diags.is_empty());
    }

    #[test]
    fn load_skill_missing_description_produces_diagnostic() {
        let tmp = TempDir::new().unwrap();
        let path = write_skill(tmp.path(), "no-desc", "---\nname: no-desc\n---\nContent.");
        let (skill, diags) = load_skill_from_file(&path, "user");
        assert!(skill.is_some()); // skill still loaded
        assert!(!diags.is_empty());
        assert!(diags.iter().any(|d| d.message.contains("description")));
    }

    #[test]
    fn load_skill_name_mismatch_produces_diagnostic() {
        let tmp = TempDir::new().unwrap();
        let path = write_skill(
            tmp.path(),
            "my-skill",
            "---\nname: wrong-name\ndescription: desc\n---\nContent.",
        );
        let (skill, diags) = load_skill_from_file(&path, "user");
        assert!(skill.is_some());
        assert!(diags.iter().any(|d| d.message.contains("does not match")));
    }

    #[test]
    fn load_skills_from_dir_finds_skill_md() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "hello-skill",
            "---\nname: hello-skill\ndescription: Hello world skill\n---\nDo something.",
        );
        let result = load_skills_from_dir(LoadSkillsFromDirOptions {
            dir: tmp.path().to_path_buf(),
            source: "user".to_string(),
        });
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "hello-skill");
    }

    #[test]
    fn load_skills_from_dir_nonexistent() {
        let result = load_skills_from_dir(LoadSkillsFromDirOptions {
            dir: PathBuf::from("/nonexistent/path"),
            source: "user".to_string(),
        });
        assert!(result.skills.is_empty());
    }

    #[test]
    fn load_skills_from_dir_multiple_skills() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "skill-a",
            "---\nname: skill-a\ndescription: Skill A\n---\nA.",
        );
        write_skill(
            tmp.path(),
            "skill-b",
            "---\nname: skill-b\ndescription: Skill B\n---\nB.",
        );
        let result = load_skills_from_dir(LoadSkillsFromDirOptions {
            dir: tmp.path().to_path_buf(),
            source: "user".to_string(),
        });
        assert_eq!(result.skills.len(), 2);
    }

    #[test]
    fn validate_name_valid() {
        let errors = validate_name("my-skill", "my-skill");
        assert!(errors.is_empty());
    }

    #[test]
    fn validate_name_consecutive_hyphens() {
        let errors = validate_name("my--skill", "my--skill");
        assert!(errors.iter().any(|e| e.contains("consecutive")));
    }

    #[test]
    fn validate_name_starts_with_hyphen() {
        let errors = validate_name("-skill", "-skill");
        assert!(errors.iter().any(|e| e.contains("start or end")));
    }

    #[test]
    fn validate_name_uppercase() {
        let errors = validate_name("MySkill", "MySkill");
        assert!(errors.iter().any(|e| e.contains("invalid characters")));
    }

    #[test]
    fn validate_description_empty() {
        let errors = validate_description(Some(""));
        assert!(!errors.is_empty());
    }

    #[test]
    fn validate_description_missing() {
        let errors = validate_description(None);
        assert!(!errors.is_empty());
    }

    #[test]
    fn validate_description_valid() {
        let errors = validate_description(Some("A valid description"));
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_frontmatter_no_fm() {
        let (fm, body) = parse_frontmatter("Just body text.");
        assert!(fm.name.is_none());
        assert!(fm.description.is_none());
        assert_eq!(body, "Just body text.");
    }

    #[test]
    fn parse_frontmatter_with_fm() {
        let input = "---\nname: test-skill\ndescription: Test description\n---\nBody text.";
        let (fm, body) = parse_frontmatter(input);
        assert_eq!(fm.name.as_deref(), Some("test-skill"));
        assert_eq!(fm.description.as_deref(), Some("Test description"));
        assert_eq!(body, "Body text.");
    }

    #[test]
    fn parse_frontmatter_disable_model_invocation() {
        let input = "---\nname: x\ndescription: d\ndisable-model-invocation: true\n---\nBody.";
        let (fm, _) = parse_frontmatter(input);
        assert!(fm.disable_model_invocation);
    }

    // -----------------------------------------------------------------------
    // format_skills_for_prompt
    // -----------------------------------------------------------------------

    fn make_skill(name: &str, description: &str, file_path: &str, disable: bool) -> Skill {
        Skill {
            name: name.to_string(),
            description: description.to_string(),
            file_path: PathBuf::from(file_path),
            base_dir: PathBuf::from(file_path)
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf(),
            source: "test".to_string(),
            disable_model_invocation: disable,
            content: String::new(),
        }
    }

    #[test]
    fn format_skills_for_prompt_empty_returns_empty_string() {
        let result = format_skills_for_prompt(&[]);
        assert_eq!(result, "");
    }

    #[test]
    fn format_skills_for_prompt_xml_structure() {
        let skills = vec![make_skill(
            "test-skill",
            "A test skill.",
            "/path/to/skill/SKILL.md",
            false,
        )];
        let result = format_skills_for_prompt(&skills);
        assert!(result.contains("<available_skills>"));
        assert!(result.contains("</available_skills>"));
        assert!(result.contains("<skill>"));
        assert!(result.contains("<name>test-skill</name>"));
        assert!(result.contains("<description>A test skill.</description>"));
        assert!(result.contains("<location>/path/to/skill/SKILL.md</location>"));
    }

    #[test]
    fn format_skills_for_prompt_intro_text_before_xml() {
        let skills = vec![make_skill(
            "test-skill",
            "A test skill.",
            "/path/to/skill/SKILL.md",
            false,
        )];
        let result = format_skills_for_prompt(&skills);
        let xml_start = result.find("<available_skills>").unwrap();
        let intro = &result[..xml_start];
        assert!(intro.contains("The following skills provide specialized instructions"));
        assert!(intro.contains("Use the read tool to load a skill's file"));
    }

    #[test]
    fn format_skills_for_prompt_escapes_xml_special_chars() {
        let skills = vec![make_skill(
            "test-skill",
            "A skill with <special> & \"characters\".",
            "/path/to/skill/SKILL.md",
            false,
        )];
        let result = format_skills_for_prompt(&skills);
        assert!(result.contains("&lt;special&gt;"));
        assert!(result.contains("&amp;"));
        assert!(result.contains("&quot;characters&quot;"));
    }

    #[test]
    fn format_skills_for_prompt_multiple_skills() {
        let skills = vec![
            make_skill("skill-one", "First skill.", "/path/one/SKILL.md", false),
            make_skill("skill-two", "Second skill.", "/path/two/SKILL.md", false),
        ];
        let result = format_skills_for_prompt(&skills);
        assert!(result.contains("<name>skill-one</name>"));
        assert!(result.contains("<name>skill-two</name>"));
        assert_eq!(result.matches("<skill>").count(), 2);
    }

    #[test]
    fn format_skills_for_prompt_excludes_disabled_skills() {
        let skills = vec![
            make_skill(
                "visible-skill",
                "A visible skill.",
                "/path/visible/SKILL.md",
                false,
            ),
            make_skill(
                "hidden-skill",
                "A hidden skill.",
                "/path/hidden/SKILL.md",
                true,
            ),
        ];
        let result = format_skills_for_prompt(&skills);
        assert!(result.contains("<name>visible-skill</name>"));
        assert!(!result.contains("<name>hidden-skill</name>"));
        assert_eq!(result.matches("<skill>").count(), 1);
    }

    #[test]
    fn format_skills_for_prompt_all_disabled_returns_empty_string() {
        let skills = vec![make_skill(
            "hidden-skill",
            "A hidden skill.",
            "/path/hidden/SKILL.md",
            true,
        )];
        let result = format_skills_for_prompt(&skills);
        assert_eq!(result, "");
    }
}

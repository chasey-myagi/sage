//! System prompt construction and project context loading.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/system-prompt.ts`.

use chrono::Utc;

// ============================================================================
// Options
// ============================================================================

#[derive(Debug, Default, Clone)]
pub struct BuildSystemPromptOptions {
    /// Custom system prompt (replaces default).
    pub custom_prompt: Option<String>,
    /// Tools to include in prompt. Default: `["read", "bash", "edit", "write"]`.
    pub selected_tools: Option<Vec<String>>,
    /// Optional one-line tool snippets keyed by tool name.
    pub tool_snippets: std::collections::HashMap<String, String>,
    /// Additional guideline bullets appended to the default system prompt.
    pub prompt_guidelines: Vec<String>,
    /// Text to append to system prompt.
    pub append_system_prompt: Option<String>,
    /// Working directory. Default: `cwd()`.
    pub cwd: Option<String>,
    /// Pre-loaded context files (path + content).
    pub context_files: Vec<ContextFile>,
    /// Pre-loaded skills.
    pub skills: Vec<Skill>,

    /// Path to README (used in default prompt).
    pub readme_path: Option<String>,
    /// Path to docs directory.
    pub docs_path: Option<String>,
    /// Path to examples directory.
    pub examples_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContextFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
}

// ============================================================================
// Build
// ============================================================================

/// Build the system prompt.
///
/// Mirrors `buildSystemPrompt()` from TypeScript.
pub fn build_system_prompt(options: BuildSystemPromptOptions) -> String {
    let cwd_raw = options
        .cwd
        .unwrap_or_else(|| std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default());
    let prompt_cwd = cwd_raw.replace('\\', "/");
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let append_section = options
        .append_system_prompt
        .as_deref()
        .map(|s| format!("\n\n{s}"))
        .unwrap_or_default();

    let tools = options
        .selected_tools
        .unwrap_or_else(|| vec!["read".to_string(), "bash".to_string(), "edit".to_string(), "write".to_string()]);

    // Skills formatting
    let skills_text = if !options.skills.is_empty() && tools.contains(&"read".to_string()) {
        format_skills_for_prompt(&options.skills)
    } else {
        String::new()
    };

    // Context files
    let context_section = if !options.context_files.is_empty() {
        let mut s = "\n\n# Project Context\n\nProject-specific instructions and guidelines:\n\n".to_string();
        for cf in &options.context_files {
            s.push_str(&format!("## {}\n\n{}\n\n", cf.path, cf.content));
        }
        s
    } else {
        String::new()
    };

    // ---------- Custom prompt branch ----------
    if let Some(custom) = options.custom_prompt {
        let mut prompt = custom;
        prompt.push_str(&append_section);
        prompt.push_str(&context_section);
        prompt.push_str(&skills_text);
        prompt.push_str(&format!("\nCurrent date: {date}"));
        prompt.push_str(&format!("\nCurrent working directory: {prompt_cwd}"));
        return prompt;
    }

    // ---------- Default prompt ----------

    // Build tools list
    let visible_tools: Vec<&str> = tools
        .iter()
        .filter_map(|name| {
            if options.tool_snippets.contains_key(name.as_str()) {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect();

    let tools_list = if visible_tools.is_empty() {
        "(none)".to_string()
    } else {
        visible_tools
            .iter()
            .map(|name| format!("- {name}: {}", options.tool_snippets[*name]))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Build guidelines
    let mut guidelines_list: Vec<String> = Vec::new();
    let mut seen_guidelines = std::collections::HashSet::new();

    let mut add_guideline = |g: String| {
        if seen_guidelines.insert(g.clone()) {
            guidelines_list.push(g);
        }
    };

    let has_bash = tools.contains(&"bash".to_string());
    let has_grep = tools.contains(&"grep".to_string());
    let has_find = tools.contains(&"find".to_string());
    let has_ls = tools.contains(&"ls".to_string());

    if has_bash && !has_grep && !has_find && !has_ls {
        add_guideline("Use bash for file operations like ls, rg, find".to_string());
    } else if has_bash && (has_grep || has_find || has_ls) {
        add_guideline(
            "Prefer grep/find/ls tools over bash for file exploration (faster, respects .gitignore)"
                .to_string(),
        );
    }

    for g in &options.prompt_guidelines {
        let normalized = g.trim().to_string();
        if !normalized.is_empty() {
            add_guideline(normalized);
        }
    }

    add_guideline("Be concise in your responses".to_string());
    add_guideline("Show file paths clearly when working with files".to_string());

    let guidelines = guidelines_list
        .iter()
        .map(|g| format!("- {g}"))
        .collect::<Vec<_>>()
        .join("\n");

    let readme_path = options.readme_path.as_deref().unwrap_or("README.md");
    let docs_path = options.docs_path.as_deref().unwrap_or("docs/");
    let examples_path = options.examples_path.as_deref().unwrap_or("examples/");

    let mut prompt = format!(
        "You are an expert coding assistant operating inside pi, a coding agent harness. \
You help users by reading files, executing commands, editing code, and writing new files.\n\
\n\
Available tools:\n\
{tools_list}\n\
\n\
In addition to the tools above, you may have access to other custom tools depending on the project.\n\
\n\
Guidelines:\n\
{guidelines}\n\
\n\
Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):\n\
- Main documentation: {readme_path}\n\
- Additional docs: {docs_path}\n\
- Examples: {examples_path} (extensions, custom tools, SDK)"
    );

    prompt.push_str(&append_section);
    prompt.push_str(&context_section);
    prompt.push_str(&skills_text);
    prompt.push_str(&format!("\nCurrent date: {date}"));
    prompt.push_str(&format!("\nCurrent working directory: {prompt_cwd}"));

    prompt
}

fn format_skills_for_prompt(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = "\n\n# Available Skills\n\n".to_string();
    for skill in skills {
        s.push_str(&format!("## {}\n{}\n\n", skill.name, skill.description));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tools_shows_none() {
        let prompt = build_system_prompt(BuildSystemPromptOptions {
            selected_tools: Some(vec![]),
            context_files: vec![],
            skills: vec![],
            ..Default::default()
        });
        assert!(prompt.contains("Available tools:\n(none)"));
    }

    #[test]
    fn shows_file_paths_guideline_with_no_tools() {
        let prompt = build_system_prompt(BuildSystemPromptOptions {
            selected_tools: Some(vec![]),
            context_files: vec![],
            skills: vec![],
            ..Default::default()
        });
        assert!(prompt.contains("Show file paths clearly"));
    }

    #[test]
    fn default_tools_with_snippets() {
        let mut snippets = std::collections::HashMap::new();
        snippets.insert("read".to_string(), "Read file contents".to_string());
        snippets.insert("bash".to_string(), "Execute bash commands".to_string());
        snippets.insert("edit".to_string(), "Make surgical edits".to_string());
        snippets.insert("write".to_string(), "Create or overwrite files".to_string());

        let prompt = build_system_prompt(BuildSystemPromptOptions {
            tool_snippets: snippets,
            context_files: vec![],
            skills: vec![],
            ..Default::default()
        });

        assert!(prompt.contains("- read:"));
        assert!(prompt.contains("- bash:"));
        assert!(prompt.contains("- edit:"));
        assert!(prompt.contains("- write:"));
    }

    #[test]
    fn custom_prompt_passes_through() {
        let prompt = build_system_prompt(BuildSystemPromptOptions {
            custom_prompt: Some("My custom prompt".to_string()),
            context_files: vec![],
            skills: vec![],
            ..Default::default()
        });
        assert!(prompt.starts_with("My custom prompt"));
    }
}

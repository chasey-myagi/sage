//! Prompt template loading and argument substitution.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/prompt-templates.ts`.

use std::fs;
use std::path::{Path, PathBuf};

use super::source_info::{SourceInfo, create_synthetic_source_info};

// ============================================================================
// Types
// ============================================================================

/// A prompt template loaded from a Markdown file.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source_info: SourceInfo,
    /// Absolute path to the template file.
    pub file_path: PathBuf,
}

// ============================================================================
// Argument parsing
// ============================================================================

/// Parse command arguments respecting quoted strings (bash-style).
///
/// Mirrors `parseCommandArgs()` from TypeScript.
pub fn parse_command_args(args_string: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;

    for ch in args_string.chars() {
        if let Some(q) = in_quote {
            if ch == q {
                in_quote = None;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
        } else if ch == ' ' || ch == '\t' {
            if !current.is_empty() {
                args.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

// ============================================================================
// Argument substitution
// ============================================================================

/// Substitute argument placeholders in template content.
///
/// Supports:
/// - `$1`, `$2`, … for positional args
/// - `$@` and `$ARGUMENTS` for all args
/// - `${@:N}` for args from Nth onwards
/// - `${@:N:L}` for L args starting from Nth
///
/// Mirrors `substituteArgs()` from TypeScript.
pub fn substitute_args(content: &str, args: &[String]) -> String {
    // Step 1: Replace $1, $2, … (positional) — BEFORE wildcards
    let mut result = {
        let mut s = content.to_string();
        // Collect replacements first to avoid index confusion
        let mut out = String::new();
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                i += 1;
                let mut num_str = String::new();
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    num_str.push(bytes[i] as char);
                    i += 1;
                }
                let idx: usize = num_str.parse().unwrap_or(0);
                if idx > 0 {
                    out.push_str(args.get(idx - 1).map(|s| s.as_str()).unwrap_or(""));
                } else {
                    // $0 → empty string (mirrors pi-mono: args[-1] = undefined = "")
                    out.push_str("");
                }
            } else {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
        out
    };

    // Step 2: Replace ${@:start} or ${@:start:length}
    let re_slice = regex::Regex::new(r"\$\{@:(\d+)(?::(\d+))?\}").unwrap();
    result = re_slice
        .replace_all(&result, |caps: &regex::Captures| {
            let start_raw: usize = caps[1].parse().unwrap_or(1);
            let start = if start_raw == 0 { 0 } else { start_raw - 1 };
            if let Some(len_str) = caps.get(2) {
                let length: usize = len_str.as_str().parse().unwrap_or(0);
                args.iter()
                    .skip(start)
                    .take(length)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                args.iter()
                    .skip(start)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        })
        .into_owned();

    // Step 3: Pre-compute all args joined
    let all_args = args.join(" ");

    // Step 4: Replace $ARGUMENTS
    result = result.replace("$ARGUMENTS", &all_args);

    // Step 5: Replace $@
    result = result.replace("$@", &all_args);

    result
}

// ============================================================================
// Template loading
// ============================================================================

fn parse_frontmatter(raw: &str) -> (std::collections::HashMap<String, String>, String) {
    let mut front = std::collections::HashMap::new();
    if raw.starts_with("---") {
        let end = match raw[3..].find("---") {
            Some(i) => i + 3,
            None => return (front, raw.to_string()),
        };
        let fm_section = &raw[3..end];
        let body = raw[end + 3..].trim_start_matches('\n').to_string();
        for line in fm_section.lines() {
            if let Some(colon) = line.find(':') {
                let key = line[..colon].trim().to_string();
                let value = line[colon + 1..].trim().to_string();
                front.insert(key, value);
            }
        }
        return (front, body);
    }
    (front, raw.to_string())
}

fn load_template_from_file(file_path: &Path, source_info: SourceInfo) -> Option<PromptTemplate> {
    let raw = fs::read_to_string(file_path).ok()?;
    let (frontmatter, body) = parse_frontmatter(&raw);

    let name = file_path.file_stem()?.to_string_lossy().to_string();

    let description = if let Some(d) = frontmatter.get("description") {
        d.clone()
    } else {
        body.lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| {
                let s = l.trim();
                if s.len() > 60 {
                    format!("{}...", &s[..60])
                } else {
                    s.to_string()
                }
            })
            .unwrap_or_default()
    };

    Some(PromptTemplate {
        name,
        description,
        content: body,
        source_info,
        file_path: file_path.to_path_buf(),
    })
}

fn load_templates_from_dir(
    dir: &Path,
    get_source_info: &impl Fn(&Path) -> SourceInfo,
) -> Vec<PromptTemplate> {
    let mut templates = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return templates,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().map_or(false, |e| e == "md") {
            if let Some(t) = load_template_from_file(&path, get_source_info(&path)) {
                templates.push(t);
            }
        }
    }

    templates
}

// ============================================================================
// Public load options
// ============================================================================

#[derive(Debug, Default)]
pub struct LoadPromptTemplatesOptions {
    pub cwd: Option<PathBuf>,
    pub agent_dir: Option<PathBuf>,
    pub prompt_paths: Vec<String>,
    pub include_defaults: Option<bool>,
}

/// Load all prompt templates from default + explicit locations.
///
/// Mirrors `loadPromptTemplates()` from TypeScript.
pub fn load_prompt_templates(options: LoadPromptTemplatesOptions) -> Vec<PromptTemplate> {
    let cwd = options
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let include_defaults = options.include_defaults.unwrap_or(true);
    let mut templates = Vec::new();

    let get_source_info = |path: &Path| -> SourceInfo {
        create_synthetic_source_info(
            path.to_string_lossy().as_ref(),
            "local",
            None,
            None,
            path.parent().map(|p| p.to_string_lossy().to_string()),
        )
    };

    if include_defaults {
        if let Some(ref agent_dir) = options.agent_dir {
            let global_prompts = agent_dir.join("prompts");
            templates.extend(load_templates_from_dir(&global_prompts, &get_source_info));
        }

        let project_prompts = cwd.join(".pi").join("prompts");
        templates.extend(load_templates_from_dir(&project_prompts, &get_source_info));
    }

    for raw_path in &options.prompt_paths {
        let resolved = normalize_path(raw_path, &cwd);
        if !resolved.exists() {
            continue;
        }
        if resolved.is_dir() {
            templates.extend(load_templates_from_dir(&resolved, &get_source_info));
        } else if resolved.extension().map_or(false, |e| e == "md") {
            if let Some(t) = load_template_from_file(&resolved, get_source_info(&resolved)) {
                templates.push(t);
            }
        }
    }

    templates
}

fn normalize_path(p: &str, cwd: &Path) -> PathBuf {
    let trimmed = p.trim();
    let expanded = if trimmed == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"))
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(rest)
    } else {
        PathBuf::from(trimmed)
    };

    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

// ============================================================================
// Template expansion
// ============================================================================

/// Expand a prompt template if the text starts with `/template_name`.
/// Returns the expanded content or the original text.
///
/// Mirrors `expandPromptTemplate()` from TypeScript.
pub fn expand_prompt_template(text: &str, templates: &[PromptTemplate]) -> String {
    if !text.starts_with('/') {
        return text.to_string();
    }

    let (template_name, args_string) = if let Some(space) = text.find(' ') {
        (&text[1..space], &text[space + 1..])
    } else {
        (&text[1..], "")
    };

    if let Some(template) = templates.iter().find(|t| t.name == template_name) {
        let args = parse_command_args(args_string);
        substitute_args(&template.content, &args)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // substituteArgs — basic
    // =========================================================================

    #[test]
    fn substitute_args_positional() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(substitute_args("$1 $2", &args), "a b");
    }

    #[test]
    fn substitute_args_all() {
        let args = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(substitute_args("$ARGUMENTS", &args), "a b c");
        assert_eq!(substitute_args("$@", &args), "a b c");
    }

    #[test]
    fn substitute_args_no_recursive_substitution() {
        let args = vec!["$1".to_string(), "$ARGUMENTS".to_string()];
        assert_eq!(substitute_args("$ARGUMENTS", &args), "$1 $ARGUMENTS");
    }

    #[test]
    fn substitute_args_out_of_range() {
        let args = vec!["a".to_string(), "b".to_string()];
        // $3 and beyond → empty string
        assert_eq!(substitute_args("$1 $2 $3", &args), "a b ");
    }

    #[test]
    fn substitute_args_dollar_at_equals_arguments() {
        let args = vec!["foo".to_string(), "bar".to_string(), "baz".to_string()];
        assert_eq!(
            substitute_args("Test: $@", &args),
            substitute_args("Test: $ARGUMENTS", &args)
        );
    }

    #[test]
    fn substitute_args_no_recursive_dollar_at() {
        let args = vec!["$100".to_string(), "$1".to_string()];
        assert_eq!(substitute_args("$@", &args), "$100 $1");
        assert_eq!(substitute_args("$ARGUMENTS", &args), "$100 $1");
    }

    #[test]
    fn substitute_args_mixed_positional_and_all() {
        let args = vec!["prefix".to_string(), "a".to_string(), "b".to_string()];
        assert_eq!(
            substitute_args("$1: $ARGUMENTS", &args),
            "prefix: prefix a b"
        );
        assert_eq!(substitute_args("$1: $@", &args), "prefix: prefix a b");
    }

    #[test]
    fn substitute_args_empty_args_arguments() {
        assert_eq!(substitute_args("Test: $ARGUMENTS", &[]), "Test: ");
    }

    #[test]
    fn substitute_args_empty_args_dollar_at() {
        assert_eq!(substitute_args("Test: $@", &[]), "Test: ");
    }

    #[test]
    fn substitute_args_empty_args_positional() {
        assert_eq!(substitute_args("Test: $1", &[]), "Test: ");
    }

    #[test]
    fn substitute_args_multiple_occurrences_arguments() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            substitute_args("$ARGUMENTS and $ARGUMENTS", &args),
            "a b and a b"
        );
    }

    #[test]
    fn substitute_args_multiple_occurrences_dollar_at() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(substitute_args("$@ and $@", &args), "a b and a b");
    }

    #[test]
    fn substitute_args_mixed_at_and_arguments() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(substitute_args("$@ and $ARGUMENTS", &args), "a b and a b");
    }

    #[test]
    fn substitute_args_special_chars_in_args() {
        let args = vec!["arg100".to_string(), "@user".to_string()];
        assert_eq!(
            substitute_args("$1 $2: $ARGUMENTS", &args),
            "arg100 @user: arg100 @user"
        );
    }

    #[test]
    fn substitute_args_out_of_range_multi() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(substitute_args("$1 $2 $3 $4 $5", &args), "a b   ");
    }

    #[test]
    fn substitute_args_unicode() {
        let args = vec!["日本語".to_string(), "🎉".to_string(), "café".to_string()];
        assert_eq!(substitute_args("$ARGUMENTS", &args), "日本語 🎉 café");
    }

    #[test]
    fn substitute_args_newlines_and_tabs() {
        let args = vec!["line1\nline2".to_string(), "tab\tthere".to_string()];
        assert_eq!(substitute_args("$1 $2", &args), "line1\nline2 tab\tthere");
    }

    #[test]
    fn substitute_args_consecutive_dollar_patterns() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(substitute_args("$1$2", &args), "ab");
    }

    #[test]
    fn substitute_args_quoted_args_with_spaces() {
        let args = vec!["first arg".to_string(), "second arg".to_string()];
        assert_eq!(substitute_args("$ARGUMENTS", &args), "first arg second arg");
    }

    #[test]
    fn substitute_args_single_arg_arguments() {
        let args = vec!["only".to_string()];
        assert_eq!(substitute_args("Test: $ARGUMENTS", &args), "Test: only");
    }

    #[test]
    fn substitute_args_single_arg_dollar_at() {
        let args = vec!["only".to_string()];
        assert_eq!(substitute_args("Test: $@", &args), "Test: only");
    }

    #[test]
    fn substitute_args_dollar_zero_is_empty() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(substitute_args("$0", &args), "");
    }

    #[test]
    fn substitute_args_decimal_number_only_integer_matches() {
        let args = vec!["a".to_string()];
        assert_eq!(substitute_args("$1.5", &args), "a.5");
    }

    #[test]
    fn substitute_args_prefix_before_arguments() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(substitute_args("pre$ARGUMENTS", &args), "prea b");
    }

    #[test]
    fn substitute_args_prefix_before_dollar_at() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(substitute_args("pre$@", &args), "prea b");
    }

    #[test]
    fn substitute_args_empty_arg_in_middle() {
        let args = vec!["a".to_string(), "".to_string(), "c".to_string()];
        assert_eq!(substitute_args("$ARGUMENTS", &args), "a  c");
    }

    #[test]
    fn substitute_args_leading_trailing_spaces_in_args() {
        let args = vec!["  leading  ".to_string(), "trailing  ".to_string()];
        assert_eq!(
            substitute_args("$ARGUMENTS", &args),
            "  leading   trailing  "
        );
    }

    #[test]
    fn substitute_args_arg_containing_pattern_word() {
        let args = vec!["ARGUMENTS".to_string()];
        assert_eq!(
            substitute_args("Prefix $ARGUMENTS suffix", &args),
            "Prefix ARGUMENTS suffix"
        );
    }

    #[test]
    fn substitute_args_non_matching_patterns_preserved() {
        let args = vec!["a".to_string()];
        // $A, $$, $ (bare), $ARGS → all preserved
        assert_eq!(substitute_args("$A $$ $ $ARGS", &args), "$A $$ $ $ARGS");
    }

    #[test]
    fn substitute_args_case_sensitive() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            substitute_args("$arguments $Arguments $ARGUMENTS", &args),
            "$arguments $Arguments a b"
        );
    }

    #[test]
    fn substitute_args_both_syntaxes_same_result() {
        let args = vec!["x".to_string(), "y".to_string(), "z".to_string()];
        let r1 = substitute_args("$@ and $ARGUMENTS", &args);
        let r2 = substitute_args("$ARGUMENTS and $@", &args);
        assert_eq!(r1, r2);
        assert_eq!(r1, "x y z and x y z");
    }

    #[test]
    fn substitute_args_very_long_arg_list() {
        let args: Vec<String> = (0..100).map(|i| format!("arg{}", i)).collect();
        let expected = args.join(" ");
        assert_eq!(substitute_args("$ARGUMENTS", &args), expected);
    }

    #[test]
    fn substitute_args_single_digit_numbered() {
        let args = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(substitute_args("$1 $2 $3", &args), "a b c");
    }

    #[test]
    fn substitute_args_multi_digit_numbered() {
        let args: Vec<String> = (0..15).map(|i| format!("val{}", i)).collect();
        assert_eq!(substitute_args("$10 $12 $15", &args), "val9 val11 val14");
    }

    #[test]
    fn substitute_args_backslash_before_dollar_treated_literally() {
        // No escape mechanism: backslash is literal, $1 gets removed (no arg)
        assert_eq!(substitute_args("Price: \\$100", &[]), "Price: \\");
    }

    #[test]
    fn substitute_args_mixed_numbered_and_wildcard() {
        let args = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];
        assert_eq!(
            substitute_args("$1: $@ ($ARGUMENTS)", &args),
            "first: first second third (first second third)"
        );
    }

    #[test]
    fn substitute_args_no_placeholders() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(substitute_args("Just plain text", &args), "Just plain text");
    }

    #[test]
    fn substitute_args_only_placeholders() {
        let args = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(substitute_args("$1 $2 $@", &args), "a b a b c");
    }

    // =========================================================================
    // substituteArgs — array slicing (Bash-style ${@:N} / ${@:N:L})
    // =========================================================================

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn slice_from_index() {
        assert_eq!(
            substitute_args("${@:2}", &s(&["a", "b", "c", "d"])),
            "b c d"
        );
        assert_eq!(substitute_args("${@:1}", &s(&["a", "b", "c"])), "a b c");
        assert_eq!(substitute_args("${@:3}", &s(&["a", "b", "c", "d"])), "c d");
    }

    #[test]
    fn slice_with_length() {
        assert_eq!(
            substitute_args("${@:2:2}", &s(&["a", "b", "c", "d"])),
            "b c"
        );
        assert_eq!(substitute_args("${@:1:1}", &s(&["a", "b", "c"])), "a");
        assert_eq!(substitute_args("${@:3:1}", &s(&["a", "b", "c", "d"])), "c");
        assert_eq!(
            substitute_args("${@:2:3}", &s(&["a", "b", "c", "d", "e"])),
            "b c d"
        );
    }

    #[test]
    fn slice_out_of_range() {
        assert_eq!(substitute_args("${@:99}", &s(&["a", "b"])), "");
        assert_eq!(substitute_args("${@:5}", &s(&["a", "b"])), "");
        assert_eq!(substitute_args("${@:10:5}", &s(&["a", "b"])), "");
    }

    #[test]
    fn slice_zero_length() {
        assert_eq!(substitute_args("${@:2:0}", &s(&["a", "b", "c"])), "");
        assert_eq!(substitute_args("${@:1:0}", &s(&["a", "b"])), "");
    }

    #[test]
    fn slice_length_exceeds_array() {
        assert_eq!(substitute_args("${@:2:99}", &s(&["a", "b", "c"])), "b c");
        assert_eq!(substitute_args("${@:1:10}", &s(&["a", "b"])), "a b");
    }

    #[test]
    fn slice_processed_before_simple_dollar_at() {
        assert_eq!(
            substitute_args("${@:2} vs $@", &s(&["a", "b", "c"])),
            "b c vs a b c"
        );
        assert_eq!(
            substitute_args("First: ${@:1:1}, All: $@", &s(&["x", "y", "z"])),
            "First: x, All: x y z"
        );
    }

    #[test]
    fn slice_no_recursive_substitution() {
        assert_eq!(
            substitute_args("${@:1}", &s(&["${@:2}", "test"])),
            "${@:2} test"
        );
        assert_eq!(
            substitute_args("${@:2}", &s(&["a", "${@:3}", "c"])),
            "${@:3} c"
        );
    }

    #[test]
    fn slice_mixed_with_positional() {
        assert_eq!(
            substitute_args("$1: ${@:2}", &s(&["cmd", "arg1", "arg2"])),
            "cmd: arg1 arg2"
        );
        assert_eq!(
            substitute_args("$1 $2 ${@:3}", &s(&["a", "b", "c", "d"])),
            "a b c d"
        );
    }

    #[test]
    fn slice_zero_start_is_all_args() {
        assert_eq!(substitute_args("${@:0}", &s(&["a", "b", "c"])), "a b c");
    }

    #[test]
    fn slice_empty_args_array() {
        assert_eq!(substitute_args("${@:2}", &[]), "");
        assert_eq!(substitute_args("${@:1}", &[]), "");
    }

    #[test]
    fn slice_single_arg_array() {
        assert_eq!(substitute_args("${@:1}", &s(&["only"])), "only");
        assert_eq!(substitute_args("${@:2}", &s(&["only"])), "");
    }

    #[test]
    fn slice_in_middle_of_text() {
        assert_eq!(
            substitute_args("Process ${@:2} with $1", &s(&["tool", "file1", "file2"])),
            "Process file1 file2 with tool"
        );
    }

    #[test]
    fn slice_multiple_in_one_template() {
        assert_eq!(
            substitute_args("${@:1:1} and ${@:2}", &s(&["a", "b", "c"])),
            "a and b c"
        );
        assert_eq!(
            substitute_args("${@:1:2} vs ${@:3:2}", &s(&["a", "b", "c", "d", "e"])),
            "a b vs c d"
        );
    }

    #[test]
    fn slice_quoted_args() {
        assert_eq!(
            substitute_args("${@:2}", &s(&["cmd", "first arg", "second arg"])),
            "first arg second arg"
        );
    }

    #[test]
    fn slice_special_chars_in_args() {
        assert_eq!(
            substitute_args("${@:2}", &s(&["cmd", "$100", "@user", "#tag"])),
            "$100 @user #tag"
        );
    }

    #[test]
    fn slice_unicode_in_args() {
        assert_eq!(
            substitute_args("${@:1}", &s(&["日本語", "🎉", "café"])),
            "日本語 🎉 café"
        );
    }

    #[test]
    fn slice_combined_positional_slice_wildcard() {
        let args = s(&["eslint", "file1.ts", "file2.ts", "file3.ts"]);
        let template = "Run $1 on ${@:2:2}, then process $@";
        assert_eq!(
            substitute_args(template, &args),
            "Run eslint on file1.ts file2.ts, then process eslint file1.ts file2.ts file3.ts"
        );
    }

    #[test]
    fn slice_no_spacing_around_slice() {
        assert_eq!(
            substitute_args("prefix${@:2}suffix", &s(&["a", "b", "c"])),
            "prefixb csuffix"
        );
    }

    #[test]
    fn slice_large_length_graceful() {
        let args: Vec<String> = (1..=10).map(|i| format!("arg{}", i)).collect();
        assert_eq!(
            substitute_args("${@:5:100}", &args),
            "arg5 arg6 arg7 arg8 arg9 arg10"
        );
    }

    // =========================================================================
    // parseCommandArgs
    // =========================================================================

    #[test]
    fn parse_command_args_simple() {
        assert_eq!(parse_command_args("a b c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_command_args_quoted() {
        let args = parse_command_args(r#"hello "world foo" bar"#);
        assert_eq!(args, vec!["hello", "world foo", "bar"]);
    }

    #[test]
    fn parse_command_args_single_quoted() {
        assert_eq!(
            parse_command_args("'first arg' second"),
            vec!["first arg", "second"]
        );
    }

    #[test]
    fn parse_command_args_mixed_quotes() {
        assert_eq!(
            parse_command_args(r#""double" 'single' "double again""#),
            vec!["double", "single", "double again"]
        );
    }

    #[test]
    fn parse_command_args_empty_string() {
        assert_eq!(parse_command_args(""), Vec::<String>::new());
    }

    #[test]
    fn parse_command_args_extra_spaces() {
        assert_eq!(parse_command_args("a  b   c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_command_args_tabs_as_separators() {
        assert_eq!(parse_command_args("a\tb\tc"), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_command_args_quoted_empty_string_skipped() {
        // Empty quotes are skipped; space-only quoted string is kept
        assert_eq!(parse_command_args(r#""" " ""#), vec![" "]);
    }

    #[test]
    fn parse_command_args_special_chars() {
        assert_eq!(
            parse_command_args("$100 @user #tag"),
            vec!["$100", "@user", "#tag"]
        );
    }

    #[test]
    fn parse_command_args_unicode() {
        assert_eq!(
            parse_command_args("日本語 🎉 café"),
            vec!["日本語", "🎉", "café"]
        );
    }

    #[test]
    fn parse_command_args_newlines_in_quotes() {
        assert_eq!(
            parse_command_args("\"line1\nline2\" second"),
            vec!["line1\nline2", "second"]
        );
    }

    #[test]
    fn parse_command_args_trailing_spaces() {
        assert_eq!(parse_command_args("a b c   "), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_command_args_leading_spaces() {
        assert_eq!(parse_command_args("   a b c"), vec!["a", "b", "c"]);
    }

    // =========================================================================
    // Integration: parseCommandArgs + substituteArgs
    // =========================================================================

    #[test]
    fn integration_parse_and_substitute() {
        let input = r#"Button "onClick handler" "disabled support""#;
        let args = parse_command_args(input);
        let template = "Create component $1 with features: $ARGUMENTS";
        let result = substitute_args(template, &args);
        assert_eq!(
            result,
            "Create component Button with features: Button onClick handler disabled support"
        );
    }

    #[test]
    fn integration_readme_example() {
        let input = r#"Button "onClick handler" "disabled support""#;
        let args = parse_command_args(input);
        let template = "Create a React component named $1 with features: $ARGUMENTS";
        let result = substitute_args(template, &args);
        assert_eq!(
            result,
            "Create a React component named Button with features: Button onClick handler disabled support"
        );
    }

    #[test]
    fn integration_dollar_at_equals_arguments() {
        let args = parse_command_args("feature1 feature2 feature3");
        let r1 = substitute_args("Implement: $@", &args);
        let r2 = substitute_args("Implement: $ARGUMENTS", &args);
        assert_eq!(r1, r2);
    }

    // =========================================================================
    // expand_prompt_template
    // =========================================================================

    #[test]
    fn expand_prompt_template_not_a_template() {
        let templates = vec![];
        assert_eq!(expand_prompt_template("hello", &templates), "hello");
    }

    #[test]
    fn expand_prompt_template_no_match() {
        let templates = vec![];
        assert_eq!(
            expand_prompt_template("/unknown-template foo", &templates),
            "/unknown-template foo"
        );
    }

    #[test]
    fn expand_prompt_template_found() {
        use crate::core::source_info::create_synthetic_source_info;
        use std::path::PathBuf;

        let template = PromptTemplate {
            name: "greet".to_string(),
            description: "Greets".to_string(),
            content: "Hello, $1!".to_string(),
            source_info: create_synthetic_source_info("/fake/greet.md", "test", None, None, None),
            file_path: PathBuf::from("/fake/greet.md"),
        };
        let result = expand_prompt_template("/greet World", &[template]);
        assert_eq!(result, "Hello, World!");
    }
}

// EditTool — precise text replacement with fuzzy matching.

use std::sync::Arc;

use crate::types::Content;

use super::backend::ToolBackend;

/// Match types returned by fuzzy_find.
#[derive(Debug)]
pub enum FuzzyMatch {
    Exact(usize),
    Normalized(usize),
    Fuzzy(usize),
}

/// Errors from apply_edit.
#[derive(Debug)]
pub enum EditError {
    NotFound,
    MultipleMatches(usize),
}

/// Result of a successful edit.
#[derive(Debug)]
pub struct EditResult {
    pub new_content: String,
    pub match_type: FuzzyMatch,
    pub diff: String,
}

/// Normalize smart quotes, dashes, and trailing whitespace.
pub fn fuzzy_normalize(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\u{2018}' | '\u{2019}' => result.push('\''),
            '\u{201C}' | '\u{201D}' => result.push('"'),
            '\u{2014}' => result.push_str("--"),
            '\u{2013}' => result.push('-'),
            _ => result.push(ch),
        }
    }
    result
        .split('\n')
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Find needle in content with escalating match strategies.
pub fn fuzzy_find(content: &str, needle: &str) -> Option<FuzzyMatch> {
    if needle.is_empty() {
        return None;
    }

    // 1. Exact match
    if let Some(pos) = content.find(needle) {
        return Some(FuzzyMatch::Exact(pos));
    }

    // 2. Normalized (CRLF → LF)
    let content_norm = content.replace("\r\n", "\n");
    let needle_norm = needle.replace("\r\n", "\n");
    if let Some(pos) = content_norm.find(&needle_norm) {
        return Some(FuzzyMatch::Normalized(pos));
    }

    // 3. Fuzzy (smart quotes, dashes, trailing whitespace)
    let content_fuzzy = fuzzy_normalize(&content_norm);
    let needle_fuzzy = fuzzy_normalize(&needle_norm);
    if let Some(pos) = content_fuzzy.find(&needle_fuzzy) {
        return Some(FuzzyMatch::Fuzzy(pos));
    }

    None
}

/// Apply a text replacement, returning error if not found or ambiguous.
pub fn apply_edit(content: &str, old_text: &str, new_text: &str) -> Result<EditResult, EditError> {
    if old_text.is_empty() {
        return Err(EditError::NotFound);
    }

    // Exact match
    let exact_count = content.matches(old_text).count();
    if exact_count == 1 {
        let pos = content.find(old_text).unwrap();
        let new_content = format!(
            "{}{}{}",
            &content[..pos],
            new_text,
            &content[pos + old_text.len()..]
        );
        let diff = generate_diff(content, &new_content, "file");
        return Ok(EditResult {
            new_content,
            match_type: FuzzyMatch::Exact(pos),
            diff,
        });
    }
    if exact_count > 1 {
        return Err(EditError::MultipleMatches(exact_count));
    }

    // Normalized (CRLF)
    let content_norm = content.replace("\r\n", "\n");
    let old_norm = old_text.replace("\r\n", "\n");
    let norm_count = content_norm.matches(&old_norm).count();
    if norm_count == 1 {
        let pos = content_norm.find(&old_norm).unwrap();
        let new_content = format!(
            "{}{}{}",
            &content_norm[..pos],
            new_text,
            &content_norm[pos + old_norm.len()..]
        );
        let diff = generate_diff(content, &new_content, "file");
        return Ok(EditResult {
            new_content,
            match_type: FuzzyMatch::Normalized(pos),
            diff,
        });
    }
    if norm_count > 1 {
        return Err(EditError::MultipleMatches(norm_count));
    }

    // Fuzzy (smart quotes, dashes, trailing whitespace)
    let content_fuzzy = fuzzy_normalize(&content_norm);
    let old_fuzzy = fuzzy_normalize(&old_norm);
    let fuzzy_count = content_fuzzy.matches(&old_fuzzy).count();
    if fuzzy_count == 1 {
        let pos = content_fuzzy.find(&old_fuzzy).unwrap();
        let new_content = format!(
            "{}{}{}",
            &content_fuzzy[..pos],
            new_text,
            &content_fuzzy[pos + old_fuzzy.len()..]
        );
        let diff = generate_diff(content, &new_content, "file");
        return Ok(EditResult {
            new_content,
            match_type: FuzzyMatch::Fuzzy(pos),
            diff,
        });
    }
    if fuzzy_count > 1 {
        return Err(EditError::MultipleMatches(fuzzy_count));
    }

    Err(EditError::NotFound)
}

/// Generate a unified diff between old and new content.
pub fn generate_diff(old: &str, new: &str, filename: &str) -> String {
    if old == new {
        return String::new();
    }

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut result = format!("--- a/{}\n+++ b/{}\n", filename, filename);

    // Find common prefix
    let common_prefix = old_lines
        .iter()
        .zip(new_lines.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Find common suffix
    let common_suffix = old_lines
        .iter()
        .rev()
        .zip(new_lines.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(old_lines.len().saturating_sub(common_prefix))
        .min(new_lines.len().saturating_sub(common_prefix));

    let old_changed = &old_lines[common_prefix..old_lines.len() - common_suffix];
    let new_changed = &new_lines[common_prefix..new_lines.len() - common_suffix];

    result.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        common_prefix + 1,
        old_changed.len(),
        common_prefix + 1,
        new_changed.len()
    ));

    for line in old_changed {
        result.push_str(&format!("-{}\n", line));
    }
    for line in new_changed {
        result.push_str(&format!("+{}\n", line));
    }

    result
}

fn error_output(msg: &str) -> super::ToolOutput {
    super::ToolOutput {
        content: vec![Content::Text {
            text: msg.to_string(),
        }],
        is_error: true,
    }
}

pub struct EditTool(pub Arc<dyn ToolBackend>);

#[async_trait::async_trait]
impl super::AgentTool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Replace a precise text range in a LOCAL workspace file (fuzzy \
         match supported). Use this for surgical edits to SKILL.md etc. \
         Does NOT edit remote documents; for those use `bash <cli>`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "old_text": { "type": "string" },
                "new_text": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" }
            },
            "required": ["file_path", "old_text", "new_text"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> super::ToolOutput {
        let file_path = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_string(),
            Some(_) => return error_output("file_path is empty"),
            None => return error_output("missing required parameter: file_path"),
        };

        let old_text = match args
            .get("old_text")
            .or_else(|| args.get("old_string"))
            .and_then(|v| v.as_str())
        {
            Some(s) => s.to_string(),
            None => return error_output("missing required parameter: old_text"),
        };

        let new_text = match args
            .get("new_text")
            .or_else(|| args.get("new_string"))
            .and_then(|v| v.as_str())
        {
            Some(s) => s.to_string(),
            None => return error_output("missing required parameter: new_text"),
        };

        let content = match self.0.read_file(&file_path).await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => return error_output(&format!("Failed to read {}: {}", file_path, e)),
        };

        match apply_edit(&content, &old_text, &new_text) {
            Ok(result) => {
                if let Err(e) = self
                    .0
                    .write_file(&file_path, result.new_content.as_bytes())
                    .await
                {
                    return error_output(&format!("Failed to write {}: {}", file_path, e));
                }
                super::ToolOutput {
                    content: vec![Content::Text { text: result.diff }],
                    is_error: false,
                }
            }
            Err(EditError::NotFound) => error_output("old_text not found in file"),
            Err(EditError::MultipleMatches(n)) => {
                error_output(&format!("old_text found {} times — must be unique", n))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::AgentTool;
    use crate::tools::backend::LocalBackend;
    use serde_json::json;

    fn edit_tool() -> EditTool {
        EditTool(LocalBackend::new())
    }

    // ===============================================================
    // fuzzy_normalize
    // ===============================================================

    #[test]
    fn test_normalize_plain_ascii_unchanged() {
        let input = "hello world";
        assert_eq!(fuzzy_normalize(input), input);
    }

    #[test]
    fn test_normalize_smart_single_quotes() {
        // \u{2018} = ' , \u{2019} = '
        let input = "it\u{2018}s a test\u{2019}";
        let result = fuzzy_normalize(&input);
        assert_eq!(result, "it's a test'");
    }

    #[test]
    fn test_normalize_smart_double_quotes() {
        // \u{201C} = " , \u{201D} = "
        let input = "\u{201C}hello\u{201D}";
        let result = fuzzy_normalize(&input);
        assert_eq!(result, "\"hello\"");
    }

    #[test]
    fn test_normalize_em_dash() {
        // \u{2014} = — (em dash)
        let input = "foo \u{2014} bar";
        let result = fuzzy_normalize(&input);
        assert_eq!(result, "foo -- bar");
    }

    #[test]
    fn test_normalize_en_dash() {
        // \u{2013} = – (en dash)
        let input = "1\u{2013}10";
        let result = fuzzy_normalize(&input);
        assert_eq!(result, "1-10");
    }

    #[test]
    fn test_normalize_trailing_whitespace_removed() {
        let input = "hello   \nworld  \n";
        let result = fuzzy_normalize(&input);
        // Each line should have trailing whitespace stripped
        for line in result.lines() {
            assert_eq!(line, line.trim_end());
        }
    }

    #[test]
    fn test_normalize_preserves_leading_whitespace() {
        let input = "  indented\n    more indented";
        let result = fuzzy_normalize(&input);
        assert!(result.starts_with("  indented"));
        assert!(result.contains("    more indented"));
    }

    #[test]
    fn test_normalize_empty_string() {
        assert_eq!(fuzzy_normalize(""), "");
    }

    // ===============================================================
    // fuzzy_find
    // ===============================================================

    #[test]
    fn test_find_exact_match() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        let needle = "    println!(\"hello\");";
        let result = fuzzy_find(content, needle);
        assert!(result.is_some());
        match result.unwrap() {
            FuzzyMatch::Exact(pos) => {
                assert_eq!(&content[pos..pos + needle.len()], needle);
            }
            other => panic!("expected Exact, got {:?}", other),
        }
    }

    #[test]
    fn test_find_exact_multiline() {
        let content = "a\nb\nc\nd\n";
        let needle = "b\nc";
        let result = fuzzy_find(content, needle);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), FuzzyMatch::Exact(_)));
    }

    #[test]
    fn test_find_normalized_crlf() {
        // Content has \r\n but needle has \n
        let content = "line1\r\nline2\r\nline3\r\n";
        let needle = "line1\nline2";
        let result = fuzzy_find(content, needle);
        assert!(result.is_some());
        match result.unwrap() {
            FuzzyMatch::Normalized(_) => {} // expected
            FuzzyMatch::Exact(_) => {}      // also acceptable if impl normalizes content first
            other => panic!("expected Normalized or Exact, got {:?}", other),
        }
    }

    #[test]
    fn test_find_fuzzy_smart_quotes() {
        let content = "let msg = \"it's fine\";";
        let needle = "let msg = \u{201C}it\u{2019}s fine\u{201D};";
        let result = fuzzy_find(content, needle);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), FuzzyMatch::Fuzzy(_)));
    }

    #[test]
    fn test_find_fuzzy_trailing_whitespace() {
        let content = "hello   \nworld";
        let needle = "hello\nworld";
        let result = fuzzy_find(content, needle);
        assert!(result.is_some());
        // Should match via fuzzy normalization (trailing whitespace ignored)
        assert!(matches!(
            result.unwrap(),
            FuzzyMatch::Fuzzy(_) | FuzzyMatch::Normalized(_)
        ));
    }

    #[test]
    fn test_find_not_found() {
        let content = "fn main() {}";
        let needle = "fn nonexistent() {}";
        let result = fuzzy_find(content, needle);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_empty_needle() {
        let content = "hello world";
        let result = fuzzy_find(content, "");
        // Empty needle should return None (nothing to search for) or Exact(0)
        assert!(
            result.is_none() || matches!(result, Some(FuzzyMatch::Exact(0))),
            "empty needle should return None or Exact(0), got: {:?}",
            result
        );
    }

    #[test]
    fn test_find_needle_longer_than_content() {
        let content = "short";
        let needle = "this is a much longer string than content";
        let result = fuzzy_find(content, needle);
        assert!(result.is_none());
    }

    // ===============================================================
    // apply_edit
    // ===============================================================

    #[test]
    fn test_apply_exact_replacement() {
        let content = "hello world";
        let result = apply_edit(content, "world", "rust").unwrap();
        assert_eq!(result.new_content, "hello rust");
        assert!(matches!(result.match_type, FuzzyMatch::Exact(_)));
    }

    #[test]
    fn test_apply_multiline_replacement() {
        let content = "fn main() {\n    println!(\"old\");\n}\n";
        let old = "    println!(\"old\");";
        let new = "    println!(\"new\");";
        let result = apply_edit(content, old, new).unwrap();
        assert!(result.new_content.contains("println!(\"new\")"));
        assert!(!result.new_content.contains("println!(\"old\")"));
    }

    #[test]
    fn test_apply_not_found_returns_error() {
        let content = "hello world";
        let result = apply_edit(content, "nonexistent", "replacement");
        assert!(result.is_err());
        match result.unwrap_err() {
            EditError::NotFound => {}
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_apply_multiple_matches_returns_error() {
        let content = "aaa bbb aaa ccc aaa";
        let result = apply_edit(content, "aaa", "zzz");
        assert!(result.is_err());
        match result.unwrap_err() {
            EditError::MultipleMatches(count) => {
                assert_eq!(count, 3);
            }
            other => panic!("expected MultipleMatches, got {:?}", other),
        }
    }

    #[test]
    fn test_apply_empty_old_text() {
        // Empty old_text is invalid — implementation should return an error
        let content = "hello";
        let result = apply_edit(content, "", "new");
        // Must not panic; should return an error since empty search is meaningless
        assert!(
            result.is_err(),
            "empty old_text should return an error, got: {:?}",
            result
        );
    }

    #[test]
    fn test_apply_empty_new_text_is_deletion() {
        let content = "hello cruel world";
        let result = apply_edit(content, " cruel", "").unwrap();
        assert_eq!(result.new_content, "hello world");
    }

    #[test]
    fn test_apply_result_has_diff() {
        let content = "old line";
        let result = apply_edit(content, "old", "new").unwrap();
        assert!(!result.diff.is_empty());
    }

    #[test]
    fn test_apply_fuzzy_match_still_works() {
        // Content has smart quotes; old_text uses straight quotes
        let content = "let x = \u{201C}hello\u{201D};";
        let old = "let x = \"hello\";";
        let new = "let x = \"world\";";
        let result = apply_edit(content, old, new);
        // Should find via fuzzy and apply replacement
        assert!(result.is_ok());
    }

    // ===============================================================
    // generate_diff
    // ===============================================================

    #[test]
    fn test_diff_identical_content() {
        let diff = generate_diff("same", "same", "test.rs");
        // No changes — diff may be empty or just a header
        assert!(!diff.contains("\n+") && !diff.contains("\n-"));
    }

    #[test]
    fn test_diff_added_line() {
        let old = "line1\nline2\n";
        let new = "line1\ninserted\nline2\n";
        let diff = generate_diff(old, new, "test.rs");
        assert!(diff.contains("+inserted"));
    }

    #[test]
    fn test_diff_removed_line() {
        let old = "line1\nremove_me\nline2\n";
        let new = "line1\nline2\n";
        let diff = generate_diff(old, new, "test.rs");
        assert!(diff.contains("-remove_me"));
    }

    #[test]
    fn test_diff_modified_line() {
        let old = "old_value\n";
        let new = "new_value\n";
        let diff = generate_diff(old, new, "test.rs");
        assert!(diff.contains("-old_value"));
        assert!(diff.contains("+new_value"));
    }

    #[test]
    fn test_diff_contains_file_path() {
        let diff = generate_diff("a\n", "b\n", "src/main.rs");
        assert!(diff.contains("src/main.rs"));
    }

    #[test]
    fn test_diff_empty_old_is_pure_addition() {
        let diff = generate_diff("", "new content\n", "new_file.rs");
        assert!(diff.contains("+new content"));
    }

    #[test]
    fn test_diff_empty_new_is_pure_deletion() {
        let diff = generate_diff("old content\n", "", "deleted.rs");
        assert!(diff.contains("-old content"));
    }

    // ===============================================================
    // EditTool metadata + schema
    // ===============================================================

    #[test]
    fn test_name() {
        let tool = edit_tool();
        assert_eq!(tool.name(), "edit");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = edit_tool();
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_schema_has_required_fields() {
        let tool = edit_tool();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("file_path").is_some());
        assert!(props.get("old_text").is_some());
        assert!(props.get("new_text").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("file_path")));
        assert!(required.iter().any(|v| v.as_str() == Some("old_text")));
        assert!(required.iter().any(|v| v.as_str() == Some("new_text")));
    }

    #[tokio::test]
    async fn test_missing_args_returns_error() {
        let tool = edit_tool();
        let output = tool.execute(json!({})).await;
        assert!(output.is_error);
    }

    // ===============================================================
    // fuzzy_find — multiple fuzzy matches
    // ===============================================================

    #[test]
    fn test_find_multiple_fuzzy_matches() {
        // Content has two occurrences that would both fuzzy-match
        let content = "let a = \u{201C}hello\u{201D};\nlet b = \u{201C}hello\u{201D};\n";
        let needle = "let a = \"hello\";"; // straight quotes — fuzzy matches first line
        let result = fuzzy_find(content, needle);
        // Should find at least one match (the first occurrence)
        assert!(result.is_some());
    }

    // ===============================================================
    // apply_edit — fuzzy match with multiple fuzzy hits (uniqueness)
    // ===============================================================

    #[test]
    fn test_apply_fuzzy_multiple_hits_returns_error() {
        // Two lines that both fuzzy-match the same needle via smart-quote normalization
        let content = "let x = \u{201C}val\u{201D};\nlet y = \u{201C}val\u{201D};\n";
        let old = "let x = \"val\";"; // Would fuzzy-match... but wait, only 1st line has "x"
        // Use a needle that truly matches both lines after normalization:
        let content2 = "\u{201C}val\u{201D}\n\u{201C}val\u{201D}\n";
        let result = apply_edit(content2, "\"val\"", "\"new\"");
        // Multiple fuzzy matches — should return MultipleMatches error
        assert!(result.is_err());
        match result.unwrap_err() {
            EditError::MultipleMatches(count) => {
                assert!(count >= 2, "expected at least 2 matches, got {}", count);
            }
            other => panic!("expected MultipleMatches, got {:?}", other),
        }
    }

    // ===============================================================
    // apply_edit — replacement creates new match pattern
    // ===============================================================

    #[test]
    fn test_apply_replacement_does_not_recurse() {
        // old="a", new="aa", content="xa" — exactly one "a" to replace
        // After replacement: "xaa" — which contains "a" twice.
        // The edit should only apply once (no recursive replacement).
        let content = "xa";
        let result = apply_edit(content, "a", "aa").unwrap();
        assert_eq!(result.new_content, "xaa");
    }

    // ===============================================================
    // State combination: consecutive edits on same content
    // ===============================================================

    #[test]
    fn test_consecutive_edits_on_same_content() {
        // First edit: change "foo" to "bar"
        let content = "let x = foo;\nlet y = baz;\n";
        let result1 = apply_edit(content, "foo", "bar").unwrap();
        assert_eq!(result1.new_content, "let x = bar;\nlet y = baz;\n");

        // Second edit on the result: change "baz" to "qux"
        let result2 = apply_edit(&result1.new_content, "baz", "qux").unwrap();
        assert_eq!(result2.new_content, "let x = bar;\nlet y = qux;\n");
    }

    #[test]
    fn test_edit_result_can_be_edited_again() {
        // Apply edit, then try to edit the SAME pattern that was just replaced
        let content = "hello world";
        let result1 = apply_edit(content, "hello", "goodbye").unwrap();
        assert_eq!(result1.new_content, "goodbye world");

        // Now "goodbye" exists — edit it to "farewell"
        let result2 = apply_edit(&result1.new_content, "goodbye", "farewell").unwrap();
        assert_eq!(result2.new_content, "farewell world");
    }

    // ===============================================================
    // apply_edit preserves surrounding content
    // ===============================================================

    #[test]
    fn test_apply_preserves_surrounding_content() {
        let content = "line 1\nline 2 TARGET line 2 end\nline 3\n";
        let result = apply_edit(content, "TARGET", "REPLACED").unwrap();
        assert_eq!(
            result.new_content,
            "line 1\nline 2 REPLACED line 2 end\nline 3\n"
        );
        // Verify line 1 and line 3 are untouched
        assert!(result.new_content.starts_with("line 1\n"));
        assert!(result.new_content.ends_with("line 3\n"));
    }

    // ===============================================================
    // EditTool execute — integration-style tests
    // ===============================================================

    #[test]
    fn test_edit_tool_name() {
        let tool = edit_tool();
        assert_eq!(tool.name(), "edit");
    }

    #[test]
    fn test_edit_tool_schema_has_required_fields() {
        let tool = edit_tool();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("file_path").is_some());
        assert!(props.get("old_string").is_some() || props.get("old_text").is_some());
        assert!(props.get("new_string").is_some() || props.get("new_text").is_some());
    }

    // ===============================================================
    // Execute-level: file not found
    // ===============================================================

    #[tokio::test]
    async fn test_execute_file_not_found() {
        let tool = edit_tool();
        let output = tool
            .execute(json!({
                "file_path": "/nonexistent_path_12345/no_such_file.rs",
                "old_string": "foo",
                "new_string": "bar"
            }))
            .await;
        assert!(
            output.is_error,
            "editing nonexistent file must return is_error=true"
        );
    }

    #[tokio::test]
    async fn test_execute_empty_file_path() {
        let tool = edit_tool();
        let output = tool
            .execute(json!({
                "file_path": "",
                "old_string": "foo",
                "new_string": "bar"
            }))
            .await;
        assert!(output.is_error, "empty file_path must return is_error=true");
    }

    // ===============================================================
    // apply_edit — same old_text and new_text (noop)
    // ===============================================================

    #[test]
    fn test_apply_edit_same_old_and_new_text() {
        let content = "hello world foo bar";
        let result = apply_edit(content, "foo", "foo");
        // Replacing "foo" with "foo" is a valid noop edit — content unchanged
        match result {
            Ok(r) => {
                assert_eq!(r.new_content, content, "content should be unchanged");
            }
            Err(_) => {
                // Some implementations may allow noop edits, others may treat them as no-change
                // Either behavior is acceptable
            }
        }
    }

    // ===============================================================
    // fuzzy_find — partial line matching (needle starts/ends mid-line)
    // ===============================================================

    #[test]
    fn test_fuzzy_find_partial_line_match() {
        let content = "let value = compute(alpha, beta);\n";
        // Needle starts and ends in the middle of the line
        let needle = "compute(alpha";
        let result = fuzzy_find(content, needle);
        assert!(
            result.is_some(),
            "should find partial line match for '{}' in content",
            needle
        );
        match result.unwrap() {
            FuzzyMatch::Exact(pos) => {
                assert_eq!(
                    &content[pos..pos + needle.len()],
                    needle,
                    "matched substring should equal needle"
                );
            }
            other => panic!("expected Exact match for substring, got {:?}", other),
        }
    }
}

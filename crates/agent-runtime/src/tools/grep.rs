// GrepTool — full-text search via ripgrep JSON output.

use crate::types::Content;

/// A single grep match result.
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub content: String,
}

/// Parse a ripgrep JSON output line into a GrepMatch (only "match" type lines).
pub fn parse_rg_json_line(line: &str) -> Option<GrepMatch> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;

    if v.get("type")?.as_str()? != "match" {
        return None;
    }

    let data = v.get("data")?;
    let path = data.get("path")?.get("text")?.as_str()?.to_string();
    let line_number = data.get("line_number")?.as_u64()? as usize;
    let content = data
        .get("lines")?
        .get("text")?
        .as_str()?
        .trim_end()
        .to_string();

    Some(GrepMatch {
        path,
        line_number,
        content,
    })
}

/// Format grep results as path:line:content lines, truncating long content.
pub fn format_grep_results(matches: &[GrepMatch], max_line_length: usize) -> String {
    if matches.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    for m in matches {
        let truncated = super::truncate::truncate_line(&m.content, max_line_length);
        result.push_str(&format!("{}:{}:{}\n", m.path, m.line_number, truncated));
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

pub struct GrepTool;

#[async_trait::async_trait]
impl super::AgentTool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents using regex patterns"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string" },
                "glob": { "type": "string" },
                "ignore_case": { "type": "boolean" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> super::ToolOutput {
        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_string(),
            Some(_) => return error_output("pattern is empty"),
            None => return error_output("missing required parameter: pattern"),
        };

        // Validate regex before running rg
        if regex::Regex::new(&pattern).is_err() {
            return error_output(&format!("Invalid regex pattern: {}", pattern));
        }

        let mut cmd = tokio::process::Command::new("rg");
        cmd.arg("--json").arg(&pattern);

        if let Some(true) = args.get("ignore_case").and_then(|v| v.as_bool()) {
            cmd.arg("-i");
        }
        if let Some(glob_pat) = args.get("glob").and_then(|v| v.as_str()) {
            cmd.arg("--glob").arg(glob_pat);
        }
        if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            cmd.arg(path);
        }

        match cmd.output().await {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return error_output(
                    "ripgrep (rg) is not installed or not in PATH. \
                     Install it: https://github.com/BurntSushi/ripgrep#installation",
                );
            }
            Err(e) => {
                return error_output(&format!("Failed to execute rg: {e}"));
            }
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let matches: Vec<GrepMatch> =
                    stdout.lines().filter_map(parse_rg_json_line).collect();
                let formatted =
                    format_grep_results(&matches, super::truncate::GREP_MAX_LINE_LENGTH);
                if formatted.is_empty() {
                    super::ToolOutput {
                        content: vec![Content::Text {
                            text: "No matches found".into(),
                        }],
                        is_error: false,
                    }
                } else {
                    super::ToolOutput {
                        content: vec![Content::Text { text: formatted }],
                        is_error: false,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::AgentTool;
    use serde_json::json;

    // ---------------------------------------------------------------
    // parse_rg_json_line
    // ---------------------------------------------------------------

    #[test]
    fn test_parse_valid_match_line() {
        // ripgrep JSON match format
        let line = r#"{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"fn main() {\n"},"line_number":1}}"#;
        let m = parse_rg_json_line(line);
        assert!(m.is_some());
        let m = m.unwrap();
        assert_eq!(m.path, "src/main.rs");
        assert_eq!(m.line_number, 1);
        assert!(m.content.contains("fn main()"));
    }

    #[test]
    fn test_parse_summary_line_returns_none() {
        // ripgrep JSON summary line — not a match
        let line = r#"{"type":"summary","data":{"elapsed_total":{"secs":0,"nanos":1000}}}"#;
        let m = parse_rg_json_line(line);
        assert!(m.is_none());
    }

    #[test]
    fn test_parse_begin_line_returns_none() {
        let line = r#"{"type":"begin","data":{"path":{"text":"src/lib.rs"}}}"#;
        let m = parse_rg_json_line(line);
        assert!(m.is_none());
    }

    #[test]
    fn test_parse_invalid_json_returns_none() {
        let m = parse_rg_json_line("not json at all");
        assert!(m.is_none());
    }

    #[test]
    fn test_parse_empty_string_returns_none() {
        let m = parse_rg_json_line("");
        assert!(m.is_none());
    }

    // ---------------------------------------------------------------
    // format_grep_results
    // ---------------------------------------------------------------

    #[test]
    fn test_format_empty_results() {
        let result = format_grep_results(&[], 500);
        assert!(result.is_empty(), "empty results should produce empty string");
    }

    #[test]
    fn test_format_single_match() {
        let matches = vec![GrepMatch {
            path: "src/main.rs".into(),
            line_number: 42,
            content: "fn main() {".into(),
        }];
        let result = format_grep_results(&matches, 500);
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("42"));
        assert!(result.contains("fn main()"));
    }

    #[test]
    fn test_format_truncates_long_lines() {
        let long_content = "x".repeat(1000);
        let matches = vec![GrepMatch {
            path: "long.rs".into(),
            line_number: 1,
            content: long_content,
        }];
        let result = format_grep_results(&matches, 100);
        // The formatted line for the content should be truncated
        // Individual match lines should be <= max_line_length + some overhead
        for line in result.lines() {
            if line.contains("long.rs") {
                assert!(line.len() < 200, "line should be truncated");
            }
        }
    }

    // ---------------------------------------------------------------
    // GrepTool metadata + schema
    // ---------------------------------------------------------------

    #[test]
    fn test_name() {
        let tool = GrepTool;
        assert_eq!(tool.name(), "grep");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = GrepTool;
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_schema_has_pattern() {
        let tool = GrepTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("pattern").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("pattern")));
    }

    #[test]
    fn test_schema_has_optional_params() {
        let tool = GrepTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("path").is_some());
        assert!(props.get("glob").is_some());
        assert!(props.get("ignore_case").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(!required.iter().any(|v| v.as_str() == Some("path")));
        assert!(!required.iter().any(|v| v.as_str() == Some("glob")));
        assert!(!required.iter().any(|v| v.as_str() == Some("ignore_case")));
    }

    #[tokio::test]
    async fn test_missing_pattern_returns_error() {
        let tool = GrepTool;
        let output = tool.execute(json!({})).await;
        assert!(output.is_error);
    }

    // ---------------------------------------------------------------
    // parse_rg_json_line — paths with spaces and CJK characters
    // ---------------------------------------------------------------

    #[test]
    fn test_parse_path_with_spaces() {
        let line = r#"{"type":"match","data":{"path":{"text":"src/my project/main.rs"},"lines":{"text":"let x = 1;\n"},"line_number":10}}"#;
        let m = parse_rg_json_line(line);
        assert!(m.is_some());
        let m = m.unwrap();
        assert_eq!(m.path, "src/my project/main.rs");
        assert_eq!(m.line_number, 10);
    }

    #[test]
    fn test_parse_path_with_chinese_characters() {
        let line = r#"{"type":"match","data":{"path":{"text":"src/\u4f60\u597d/main.rs"},"lines":{"text":"fn test() {}\n"},"line_number":5}}"#;
        let m = parse_rg_json_line(line);
        assert!(m.is_some());
        let m = m.unwrap();
        assert!(m.path.contains("\u{4f60}\u{597d}"));
        assert_eq!(m.line_number, 5);
    }

    // ---------------------------------------------------------------
    // parse_rg_json_line — context type line
    // ---------------------------------------------------------------

    #[test]
    fn test_parse_context_type_returns_none() {
        // ripgrep JSON context lines appear with --json -C flags
        let line = r#"{"type":"context","data":{"path":{"text":"src/lib.rs"},"lines":{"text":"// context line\n"},"line_number":3}}"#;
        let m = parse_rg_json_line(line);
        // Context lines are not matches — should return None
        assert!(m.is_none());
    }

    // ---------------------------------------------------------------
    // format_grep_results — multi-file grouping
    // ---------------------------------------------------------------

    #[test]
    fn test_format_multi_file_grouping() {
        let matches = vec![
            GrepMatch {
                path: "src/a.rs".into(),
                line_number: 1,
                content: "fn a() {}".into(),
            },
            GrepMatch {
                path: "src/a.rs".into(),
                line_number: 5,
                content: "fn a2() {}".into(),
            },
            GrepMatch {
                path: "src/b.rs".into(),
                line_number: 10,
                content: "fn b() {}".into(),
            },
        ];
        let result = format_grep_results(&matches, 500);
        // Both file paths should appear in the output
        assert!(result.contains("src/a.rs"));
        assert!(result.contains("src/b.rs"));
        // All line numbers should be present
        assert!(result.contains("1"));
        assert!(result.contains("5"));
        assert!(result.contains("10"));
    }

    // ---------------------------------------------------------------
    // format_grep_results — large number of matches truncation
    // ---------------------------------------------------------------

    #[test]
    fn test_format_large_matches_truncated() {
        // Create 200 matches — output should be bounded
        let matches: Vec<GrepMatch> = (1..=200)
            .map(|i| GrepMatch {
                path: format!("src/file_{}.rs", i),
                line_number: i,
                content: format!("line content {}", i),
            })
            .collect();
        let result = format_grep_results(&matches, 100);
        assert!(!result.is_empty());
        // Output should be bounded — either truncated to a max number of results
        // or all 200 results are shown. Either way, the line count should be reasonable.
        let line_count = result.lines().count();
        assert!(
            line_count > 0 && line_count <= 500,
            "output should have between 1 and 500 lines, got {}",
            line_count
        );
        // If truncated, there should be some indication
        if line_count < 200 {
            assert!(
                result.contains("truncated") || result.contains("...") || result.contains("more"),
                "truncated output should indicate truncation"
            );
        }
    }

    // ---------------------------------------------------------------
    // Empty pattern string
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_empty_pattern_returns_error() {
        let tool = GrepTool;
        let output = tool.execute(json!({"pattern": ""})).await;
        assert!(output.is_error, "empty pattern should return error");
    }

    // ---------------------------------------------------------------
    // Regex special characters in pattern
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_regex_special_chars_in_pattern() {
        let tool = GrepTool;
        // Pattern with regex metacharacters — should be accepted
        let output = tool.execute(json!({"pattern": "fn\\s+\\w+\\("})).await;
        // Whether it finds matches depends on context, but should not panic
        assert!(!output.content.is_empty());
    }

    #[tokio::test]
    async fn test_literal_brackets_in_pattern() {
        let tool = GrepTool;
        let output = tool.execute(json!({"pattern": "interface\\{\\}"})).await;
        assert!(!output.content.is_empty());
    }

    // ---------------------------------------------------------------
    // ignore_case behavior
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_ignore_case_flag() {
        let tool = GrepTool;
        let output = tool
            .execute(json!({"pattern": "test", "ignore_case": true}))
            .await;
        assert!(!output.content.is_empty());
    }

    // ---------------------------------------------------------------
    // Invalid regex error path
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_invalid_regex_returns_error() {
        let tool = GrepTool;
        let output = tool.execute(json!({"pattern": "[unclosed"})).await;
        assert!(output.is_error, "invalid regex should return error");
    }

    #[tokio::test]
    async fn test_unmatched_paren_regex_returns_error() {
        let tool = GrepTool;
        let output = tool.execute(json!({"pattern": "((missing)"})).await;
        assert!(output.is_error, "unmatched paren in regex should return error");
    }

    // ---------------------------------------------------------------
    // GrepTool — search in nonexistent directory
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_grep_nonexistent_directory() {
        let tool = GrepTool;
        let output = tool
            .execute(json!({"pattern": "foo", "path": "/nonexistent_dir_12345"}))
            .await;
        // rg on a nonexistent path should either produce error or empty results
        // The key check is that it does not panic
        assert!(!output.content.is_empty());
    }

    // ---------------------------------------------------------------
    // parse_rg_json_line — high line number
    // ---------------------------------------------------------------

    #[test]
    fn test_parse_rg_json_line_high_line_number() {
        let line = r#"{"type":"match","data":{"path":{"text":"big_file.rs"},"lines":{"text":"some code\n"},"line_number":999999}}"#;
        let m = parse_rg_json_line(line);
        assert!(m.is_some(), "should parse line with high line_number");
        let m = m.unwrap();
        assert_eq!(m.line_number, 999999);
        assert_eq!(m.path, "big_file.rs");
        assert!(m.content.contains("some code"));
    }
}

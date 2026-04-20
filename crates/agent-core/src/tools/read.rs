// ReadTool — file reading with line numbers (cat -n format).

use std::sync::Arc;

use crate::types::Content;

use super::backend::ToolBackend;

/// Format content with line numbers in cat -n format (6-char wide, right-aligned).
pub fn add_line_numbers(content: &str, start_line: usize) -> String {
    if content.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = content.lines().collect();
    let mut result = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            result.push('\n');
        }
        result.push_str(&format!("{:>6}\t{}", start_line + i, line));
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

pub struct ReadTool(pub Arc<dyn ToolBackend>);

#[async_trait::async_trait]
impl super::AgentTool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a LOCAL workspace file with line numbers. \
         Use this for SKILL.md / INDEX.md / AGENT.md / workspace files. \
         Does NOT fetch remote content (URLs, cloud docs, APIs); for \
         those use `bash <cli>` (e.g. `bash lark-cli doc get …`)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "offset": { "type": "integer" },
                "limit": { "type": "integer" }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> super::ToolOutput {
        let file_path = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            Some(_) => return error_output("file_path is empty"),
            None => return error_output("missing required parameter: file_path"),
        };

        if let Some(n) = args.get("offset").and_then(|v| v.as_i64())
            && n < 0
        {
            return error_output("offset must be non-negative");
        }
        if let Some(n) = args.get("limit").and_then(|v| v.as_i64())
            && n < 0
        {
            return error_output("limit must be non-negative");
        }

        match self.0.read_file(file_path).await {
            Ok(bytes) => {
                let content = String::from_utf8_lossy(&bytes).into_owned();
                let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;

                let lines: Vec<&str> = content.lines().collect();
                let start = offset.min(lines.len());
                let end = (start + limit).min(lines.len());
                let selected_content = lines[start..end].join("\n");

                let numbered = add_line_numbers(&selected_content, start + 1);
                super::ToolOutput {
                    content: vec![Content::Text { text: numbered }],
                    is_error: false,
                }
            }
            Err(e) => error_output(&format!("Failed to read {}: {}", file_path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::AgentTool;
    use crate::tools::backend::LocalBackend;
    use serde_json::json;

    fn read_tool() -> ReadTool {
        ReadTool(LocalBackend::new())
    }

    // ---------------------------------------------------------------
    // add_line_numbers
    // ---------------------------------------------------------------

    #[test]
    fn test_line_numbers_empty_content() {
        let result = add_line_numbers("", 1);
        assert!(result.is_empty());
    }

    #[test]
    fn test_line_numbers_single_line() {
        let result = add_line_numbers("hello world", 1);
        assert!(result.contains("1\t"));
        assert!(result.contains("hello world"));
    }

    #[test]
    fn test_line_numbers_multiple_lines() {
        let result = add_line_numbers("aaa\nbbb\nccc", 1);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("     1\t"));
        assert!(lines[0].ends_with("aaa"));
        assert!(lines[1].starts_with("     2\t"));
        assert!(lines[1].ends_with("bbb"));
        assert!(lines[2].starts_with("     3\t"));
        assert!(lines[2].ends_with("ccc"));
    }

    #[test]
    fn test_line_numbers_with_offset() {
        let result = add_line_numbers("first\nsecond", 42);
        let lines: Vec<&str> = result.lines().collect();
        assert!(lines[0].contains("42\t"));
        assert!(lines[1].contains("43\t"));
    }

    #[test]
    fn test_line_numbers_right_aligned_format() {
        // cat -n uses 6-char wide right-aligned line numbers
        let result = add_line_numbers("x", 1);
        // "     1\tx"
        let prefix = result.split('\t').next().unwrap();
        assert_eq!(prefix.len(), 6, "line number should be 6 chars wide");
    }

    #[test]
    fn test_line_numbers_large_offset() {
        let result = add_line_numbers("test", 9999);
        assert!(result.contains("9999\t"));
    }

    #[test]
    fn test_line_numbers_trailing_newline_in_content() {
        let result = add_line_numbers("a\nb\n", 1);
        // "a\nb\n" has 2 content lines, the trailing newline should not create an extra numbered line
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    // ---------------------------------------------------------------
    // ReadTool metadata
    // ---------------------------------------------------------------

    #[test]
    fn test_name() {
        let tool = read_tool();
        assert_eq!(tool.name(), "read");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = read_tool();
        assert!(!tool.description().is_empty());
    }

    // ---------------------------------------------------------------
    // Parameter schema
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_has_file_path() {
        let tool = read_tool();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("file_path").is_some());
    }

    #[test]
    fn test_schema_file_path_required() {
        let tool = read_tool();
        let schema = tool.parameters_schema();
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("file_path")));
    }

    #[test]
    fn test_schema_has_optional_offset_and_limit() {
        let tool = read_tool();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("offset").is_some());
        assert!(props.get("limit").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(!required.iter().any(|v| v.as_str() == Some("offset")));
        assert!(!required.iter().any(|v| v.as_str() == Some("limit")));
    }

    // ---------------------------------------------------------------
    // Argument parsing
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_missing_file_path_returns_error() {
        let tool = read_tool();
        let output = tool.execute(json!({})).await;
        assert!(output.is_error);
    }

    // ---------------------------------------------------------------
    // offset + limit combo functional tests
    // ---------------------------------------------------------------

    #[test]
    fn test_line_numbers_offset_zero_limit_three() {
        // offset=0 means start from line 0 (or 1), limit=3 means read 3 lines
        let content = "aaa\nbbb\nccc\nddd\neee";
        let result = add_line_numbers(content, 1);
        let lines: Vec<&str> = result.lines().collect();
        // All 5 lines should be numbered starting from 1
        assert_eq!(lines.len(), 5);
        assert!(lines[0].contains("1\t"));
        assert!(lines[4].contains("5\t"));
    }

    #[test]
    fn test_line_numbers_offset_beyond_content() {
        // Offset larger than content — should still work for numbering
        let result = add_line_numbers("only line", 999999);
        assert!(result.contains("999999\t"));
        assert!(result.contains("only line"));
    }

    #[test]
    fn test_line_numbers_limit_zero_returns_empty_or_nothing() {
        // If we ask for 0 lines of a content, the add_line_numbers function
        // operates on the content it receives. If empty content is passed, empty result.
        let result = add_line_numbers("", 1);
        assert!(result.is_empty());
    }

    // ---------------------------------------------------------------
    // Error paths — file operations
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_nonexistent_file_returns_error() {
        let tool = read_tool();
        let output = tool
            .execute(json!({"file_path": "/nonexistent_path_12345/no_such_file.txt"}))
            .await;
        assert!(
            output.is_error,
            "reading nonexistent file should return error"
        );
        // Error message should mention the file or "not found"
        match &output.content[0] {
            crate::types::Content::Text { text } => {
                assert!(!text.is_empty(), "error message should not be empty");
            }
            _ => panic!("expected Text content in error"),
        }
    }

    #[tokio::test]
    async fn test_empty_file_path_returns_error() {
        let tool = read_tool();
        let output = tool.execute(json!({"file_path": ""})).await;
        assert!(output.is_error, "empty file_path should return error");
    }

    // ---------------------------------------------------------------
    // offset + limit parameter validation
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_negative_offset_returns_error() {
        let tool = read_tool();
        let output = tool
            .execute(json!({"file_path": "/tmp/test.txt", "offset": -1}))
            .await;
        // Negative offset must be rejected as invalid argument
        assert!(output.is_error, "negative offset must return is_error=true");
    }

    #[tokio::test]
    async fn test_negative_limit_returns_error() {
        let tool = read_tool();
        let output = tool
            .execute(json!({"file_path": "/tmp/test.txt", "limit": -1}))
            .await;
        // Negative limit must be rejected as invalid argument
        assert!(output.is_error, "negative limit must return is_error=true");
    }

    // ---------------------------------------------------------------
    // ReadTool success path — read real file
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_read_real_file_success() {
        let tool = read_tool();
        let file_path = std::env::temp_dir().join(format!("sage_read_test_{}", std::process::id()));
        std::fs::write(&file_path, "first line\nsecond line\nthird line\n").expect("setup write");

        let output = tool
            .execute(json!({"file_path": file_path.to_str().unwrap()}))
            .await;
        assert!(!output.is_error, "reading existing file should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        // Output should have line numbers (cat -n format) and content
        assert!(text.contains("1\t"), "output should contain line number 1");
        assert!(
            text.contains("first line"),
            "output should contain file content"
        );
        assert!(
            text.contains("second line"),
            "output should contain second line"
        );

        let _ = std::fs::remove_file(&file_path);
    }

    // ---------------------------------------------------------------
    // ReadTool offset + limit trimming
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_read_offset_and_limit_trim() {
        let tool = read_tool();
        let file_path =
            std::env::temp_dir().join(format!("sage_read_offset_test_{}", std::process::id()));
        // Create a 6-line file
        std::fs::write(&file_path, "line1\nline2\nline3\nline4\nline5\nline6\n")
            .expect("setup write");

        let output = tool
            .execute(json!({
                "file_path": file_path.to_str().unwrap(),
                "offset": 2,
                "limit": 3
            }))
            .await;
        assert!(!output.is_error, "read with offset+limit should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        // offset=2 means skip first 2 lines (line1, line2), start from line3
        // limit=3 means read 3 lines: line3, line4, line5
        assert!(
            text.contains("line3"),
            "should contain line3, got: {}",
            text
        );
        assert!(
            text.contains("line4"),
            "should contain line4, got: {}",
            text
        );
        assert!(
            text.contains("line5"),
            "should contain line5, got: {}",
            text
        );
        assert!(!text.contains("line1"), "should not contain line1");
        assert!(!text.contains("line6"), "should not contain line6");

        let _ = std::fs::remove_file(&file_path);
    }

    // ---------------------------------------------------------------
    // add_line_numbers — only newlines
    // ---------------------------------------------------------------

    #[test]
    fn test_line_numbers_only_newlines() {
        // "\n\n\n" — three newlines, which .lines() treats as empty lines
        let result = add_line_numbers("\n\n\n", 1);
        let lines: Vec<&str> = result.lines().collect();
        // .lines() on "\n\n\n" yields ["", "", ""] = 3 empty lines
        assert_eq!(
            lines.len(),
            3,
            "should have 3 numbered empty lines, got: {:?}",
            lines
        );
        assert!(
            lines[0].contains("1\t"),
            "first line should start with number 1"
        );
        assert!(
            lines[1].contains("2\t"),
            "second line should start with number 2"
        );
        assert!(
            lines[2].contains("3\t"),
            "third line should start with number 3"
        );
    }

    // ---------------------------------------------------------------
    // Read empty file
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_read_empty_file() {
        let tool = read_tool();
        let file_path =
            std::env::temp_dir().join(format!("sage_read_empty_{}", std::process::id()));
        std::fs::write(&file_path, "").expect("setup empty file");

        let output = tool
            .execute(json!({"file_path": file_path.to_str().unwrap()}))
            .await;
        assert!(
            !output.is_error,
            "reading an empty file should not be an error"
        );

        let _ = std::fs::remove_file(&file_path);
    }

    // ---------------------------------------------------------------
    // Read with offset exceeding file length
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_read_offset_exceeds_file_length() {
        let tool = read_tool();
        let file_path =
            std::env::temp_dir().join(format!("sage_read_offset_exceed_{}", std::process::id()));
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("setup file");

        let output = tool
            .execute(json!({
                "file_path": file_path.to_str().unwrap(),
                "offset": 100
            }))
            .await;
        // Should return gracefully — not an error, just empty or minimal content
        assert!(
            !output.is_error,
            "offset beyond file length should not be an error"
        );

        let _ = std::fs::remove_file(&file_path);
    }
}

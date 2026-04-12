// LsTool — directory listing.

use crate::types::Content;

fn error_output(msg: &str) -> super::ToolOutput {
    super::ToolOutput {
        content: vec![Content::Text {
            text: msg.to_string(),
        }],
        is_error: true,
    }
}

pub struct LsTool;

#[async_trait::async_trait]
impl super::AgentTool for LsTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        "List directory contents"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> super::ToolOutput {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            Some(_) => return error_output("path is empty"),
            None => return error_output("missing required parameter: path"),
        };

        if !std::path::Path::new(path).exists() {
            return error_output(&format!("Path does not exist: {}", path));
        }

        match tokio::fs::read_dir(path).await {
            Ok(mut entries) => {
                let mut names = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    names.push(entry.file_name().to_string_lossy().to_string());
                }
                names.sort();
                super::ToolOutput {
                    content: vec![Content::Text {
                        text: names.join("\n"),
                    }],
                    is_error: false,
                }
            }
            Err(e) => error_output(&format!("Failed to list {}: {}", path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::AgentTool;
    use serde_json::json;

    // ---------------------------------------------------------------
    // Metadata
    // ---------------------------------------------------------------

    #[test]
    fn test_name() {
        let tool = LsTool;
        assert_eq!(tool.name(), "ls");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = LsTool;
        assert!(!tool.description().is_empty());
    }

    // ---------------------------------------------------------------
    // Parameter schema
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_has_path() {
        let tool = LsTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("path").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }

    // ---------------------------------------------------------------
    // Argument parsing
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_missing_path_returns_error() {
        let tool = LsTool;
        let output = tool.execute(json!({})).await;
        assert!(output.is_error);
    }

    #[tokio::test]
    async fn test_empty_path_returns_error() {
        let tool = LsTool;
        let output = tool.execute(json!({"path": ""})).await;
        assert!(output.is_error);
    }

    // ---------------------------------------------------------------
    // Relative vs absolute path handling
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_relative_path_handling() {
        let tool = LsTool;
        // Relative path like "src" — should either work (resolved against cwd)
        // or return an error asking for absolute path. Must not panic.
        let output = tool.execute(json!({"path": "src"})).await;
        assert!(
            !output.content.is_empty(),
            "relative path should produce some output (success or error)"
        );
    }

    #[tokio::test]
    async fn test_absolute_path_handling() {
        let tool = LsTool;
        // Absolute path — should be accepted by argument parsing
        let output = tool.execute(json!({"path": "/tmp"})).await;
        assert!(
            !output.content.is_empty(),
            "absolute path should produce some output"
        );
    }

    // ---------------------------------------------------------------
    // Error paths
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_nonexistent_path_returns_error() {
        let tool = LsTool;
        let output = tool
            .execute(json!({"path": "/nonexistent_path_12345"}))
            .await;
        assert!(output.is_error, "nonexistent path should return error");
        match &output.content[0] {
            crate::types::Content::Text { text } => {
                assert!(!text.is_empty(), "error message should not be empty");
            }
            _ => panic!("expected Text content in error"),
        }
    }

    #[tokio::test]
    async fn test_path_is_file_not_directory() {
        let tool = LsTool;
        // /etc/hosts is a file on macOS/Linux — ls on a file should either
        // list just the file or return an error indicating it's not a directory
        let output = tool.execute(json!({"path": "/etc/hosts"})).await;
        // Either success (showing the file) or error (not a directory)
        // Either way should not panic
        assert!(!output.content.is_empty());
    }

    // ---------------------------------------------------------------
    // Hidden files consideration
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_ls_includes_or_excludes_hidden_files() {
        let tool = LsTool;
        let output = tool.execute(json!({"path": "/tmp"})).await;
        assert!(!output.content.is_empty());
    }

    // ---------------------------------------------------------------
    // Output format verification
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_ls_output_contains_filenames() {
        let tool = LsTool;
        // Create a known file in a temp dir to verify output
        let dir = std::env::temp_dir().join("agent_caster_ls_test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("testfile.txt"), "hi").expect("setup file");

        let output = tool.execute(json!({"path": dir.to_str().unwrap()})).await;
        assert!(!output.is_error, "ls on temp dir should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => String::new(),
        };
        assert!(
            text.contains("testfile.txt"),
            "ls output should contain the filename 'testfile.txt', got: {}",
            text
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_ls_empty_directory() {
        let tool = LsTool;
        let dir = std::env::temp_dir().join("agent_caster_ls_empty_test");
        let _ = std::fs::create_dir_all(&dir);

        let output = tool.execute(json!({"path": dir.to_str().unwrap()})).await;
        assert!(!output.is_error, "ls on empty dir should succeed");
        // Output may be empty string or contain just headers
        // Either way, is_error should be false

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // LsTool — output format with known files
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_ls_output_format_with_known_files() {
        let tool = LsTool;
        let dir = std::env::temp_dir().join(format!(
            "agent_caster_ls_format_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("aaa.txt"), "a").expect("setup");
        std::fs::write(dir.join("bbb.txt"), "b").expect("setup");
        std::fs::write(dir.join("ccc.rs"), "c").expect("setup");

        let output = tool
            .execute(json!({"path": dir.to_str().unwrap()}))
            .await;
        assert!(!output.is_error, "ls should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(text.contains("aaa.txt"), "output should list aaa.txt");
        assert!(text.contains("bbb.txt"), "output should list bbb.txt");
        assert!(text.contains("ccc.rs"), "output should list ccc.rs");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // LsTool — nonexistent directory returns error
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_ls_nonexistent_directory_is_error() {
        let tool = LsTool;
        let output = tool
            .execute(json!({"path": "/nonexistent_dir_99999_does_not_exist"}))
            .await;
        assert!(
            output.is_error,
            "ls on nonexistent directory should return is_error=true"
        );
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(
            !text.is_empty(),
            "error message should not be empty"
        );
    }

    // ---------------------------------------------------------------
    // Verify sorted output
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_ls_output_sorted() {
        let tool = LsTool;
        let dir = std::env::temp_dir().join(format!(
            "agent_caster_ls_sorted_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("c.txt"), "c").expect("setup");
        std::fs::write(dir.join("a.txt"), "a").expect("setup");
        std::fs::write(dir.join("b.txt"), "b").expect("setup");

        let output = tool
            .execute(json!({"path": dir.to_str().unwrap()}))
            .await;
        assert!(!output.is_error, "ls should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines, vec!["a.txt", "b.txt", "c.txt"], "output should be alphabetically sorted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // Listing includes subdirectory names
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_ls_includes_subdirectories() {
        let tool = LsTool;
        let dir = std::env::temp_dir().join(format!(
            "agent_caster_ls_subdir_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::create_dir_all(dir.join("my_subdir"));
        std::fs::write(dir.join("file.txt"), "f").expect("setup");

        let output = tool
            .execute(json!({"path": dir.to_str().unwrap()}))
            .await;
        assert!(!output.is_error, "ls should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(
            text.contains("my_subdir"),
            "ls output should include subdirectory name 'my_subdir', got: {}",
            text
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

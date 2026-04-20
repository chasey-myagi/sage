// WriteTool — file writing via backend.

use std::sync::Arc;

use crate::types::Content;

use super::backend::ToolBackend;

fn error_output(msg: &str) -> super::ToolOutput {
    super::ToolOutput {
        content: vec![Content::Text {
            text: msg.to_string(),
        }],
        is_error: true,
    }
}

pub struct WriteTool(pub Arc<dyn ToolBackend>);

#[async_trait::async_trait]
impl super::AgentTool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write (create or overwrite) a LOCAL workspace file with the \
         given content. Use this to sediment knowledge into SKILL.md / \
         INDEX.md / AGENT.md after a task. Does NOT upload to external \
         services; for those use `bash <cli>`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> super::ToolOutput {
        let file_path = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_string(),
            Some(_) => return error_output("file_path is empty"),
            None => return error_output("missing required parameter: file_path"),
        };

        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return error_output("missing required parameter: content"),
        };

        let path = std::path::Path::new(&file_path);
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return error_output(&format!("Failed to create parent directory: {e}"));
        }

        match self.0.write_file(&file_path, content.as_bytes()).await {
            Ok(()) => super::ToolOutput {
                content: vec![Content::Text {
                    text: format!(
                        "Successfully wrote {} bytes to {}",
                        content.len(),
                        file_path
                    ),
                }],
                is_error: false,
            },
            Err(e) => error_output(&format!("Failed to write {}: {}", file_path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::AgentTool;
    use crate::tools::backend::LocalBackend;
    use serde_json::json;

    fn write_tool() -> WriteTool {
        WriteTool(LocalBackend::new())
    }

    // ---------------------------------------------------------------
    // Metadata
    // ---------------------------------------------------------------

    #[test]
    fn test_name() {
        let tool = write_tool();
        assert_eq!(tool.name(), "write");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = write_tool();
        assert!(!tool.description().is_empty());
    }

    // ---------------------------------------------------------------
    // Parameter schema
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_has_file_path_and_content() {
        let tool = write_tool();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("file_path").is_some());
        assert!(props.get("content").is_some());
    }

    #[test]
    fn test_schema_both_fields_required() {
        let tool = write_tool();
        let schema = tool.parameters_schema();
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("file_path")));
        assert!(required.iter().any(|v| v.as_str() == Some("content")));
    }

    // ---------------------------------------------------------------
    // Argument parsing
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_missing_file_path_returns_error() {
        let tool = write_tool();
        let output = tool.execute(json!({"content": "hello"})).await;
        assert!(output.is_error);
    }

    #[tokio::test]
    async fn test_missing_content_returns_error() {
        let tool = write_tool();
        let output = tool.execute(json!({"file_path": "/tmp/test.txt"})).await;
        assert!(output.is_error);
    }

    #[tokio::test]
    async fn test_empty_args_returns_error() {
        let tool = write_tool();
        let output = tool.execute(json!({})).await;
        assert!(output.is_error);
    }

    // ---------------------------------------------------------------
    // file_path empty string
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_empty_file_path_returns_error() {
        let tool = write_tool();
        let output = tool
            .execute(json!({"file_path": "", "content": "hello"}))
            .await;
        assert!(output.is_error, "empty file_path should return error");
    }

    // ---------------------------------------------------------------
    // content empty string (valid — clears file)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_empty_content_is_valid_argument() {
        let tool = write_tool();
        let output = tool
            .execute(json!({"file_path": "/tmp/test_empty.txt", "content": ""}))
            .await;
        // Empty content should pass argument validation — any error would be from
        // filesystem/sandbox, not from the content being empty.
        // Check that if there IS an error, it's not about "content" being invalid
        if output.is_error {
            let text = match &output.content[0] {
                crate::types::Content::Text { text } => text.clone(),
                _ => String::new(),
            };
            assert!(
                !text.to_lowercase().contains("content is required")
                    && !text.to_lowercase().contains("content cannot be empty"),
                "empty content should not be rejected as invalid argument: {}",
                text
            );
        }
    }

    // ---------------------------------------------------------------
    // Parent directory does not exist
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_nonexistent_parent_dir_returns_error() {
        let tool = write_tool();
        let output = tool
            .execute(json!({
                "file_path": "/nonexistent_parent_dir_12345/subdir/file.txt",
                "content": "hello"
            }))
            .await;
        assert!(
            output.is_error,
            "writing to nonexistent parent dir should return error"
        );
    }

    // ---------------------------------------------------------------
    // Unicode filename
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_unicode_file_path() {
        let tool = write_tool();
        let output = tool
            .execute(json!({
                "file_path": "/tmp/测试文件.txt",
                "content": "hello"
            }))
            .await;
        assert!(!output.content.is_empty());
    }

    // ---------------------------------------------------------------
    // Success path — write and verify content
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_write_success_path() {
        let tool = write_tool();
        let test_path = "/tmp/sage_write_test.txt";
        let test_content = "hello from write test\nsecond line\n";
        let output = tool
            .execute(json!({
                "file_path": test_path,
                "content": test_content
            }))
            .await;
        // On success, is_error should be false
        assert!(!output.is_error, "write to /tmp should succeed");
        // Verify file was written correctly
        let written = std::fs::read_to_string(test_path).expect("file should exist after write");
        assert_eq!(written, test_content, "written content should match");
        // Cleanup
        let _ = std::fs::remove_file(test_path);
    }

    #[tokio::test]
    async fn test_write_overwrites_existing_file() {
        let tool = write_tool();
        let test_path = "/tmp/sage_overwrite_test.txt";
        // Write initial content
        std::fs::write(test_path, "old content").expect("setup write");
        // Overwrite with new content
        let output = tool
            .execute(json!({
                "file_path": test_path,
                "content": "new content"
            }))
            .await;
        assert!(!output.is_error, "overwrite should succeed");
        let written = std::fs::read_to_string(test_path).expect("file should exist");
        assert_eq!(written, "new content");
        let _ = std::fs::remove_file(test_path);
    }

    // ---------------------------------------------------------------
    // WriteTool — unicode content round-trip
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_write_unicode_content_roundtrip() {
        let tool = write_tool();
        let test_path =
            std::env::temp_dir().join(format!("sage_write_unicode_{}", std::process::id()));
        let path_str = test_path.to_str().unwrap();
        let unicode_content = "你好世界 🌍🚀\nEmoji: 😀💻\n日本語テスト\n";

        let output = tool
            .execute(json!({
                "file_path": path_str,
                "content": unicode_content
            }))
            .await;
        assert!(!output.is_error, "writing unicode content should succeed");

        let read_back = std::fs::read_to_string(&test_path).expect("file should exist");
        assert_eq!(
            read_back, unicode_content,
            "unicode content round-trip should be identical"
        );

        let _ = std::fs::remove_file(&test_path);
    }

    // ---------------------------------------------------------------
    // WriteTool — large content
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_write_large_content() {
        let tool = write_tool();
        let test_path =
            std::env::temp_dir().join(format!("sage_write_large_{}", std::process::id()));
        let path_str = test_path.to_str().unwrap();
        let large_content = "x".repeat(1024 * 1024); // 1MB

        let output = tool
            .execute(json!({
                "file_path": path_str,
                "content": large_content
            }))
            .await;
        assert!(!output.is_error, "writing 1MB content should succeed");

        let read_back = std::fs::read_to_string(&test_path).expect("file should exist");
        assert_eq!(read_back.len(), 1024 * 1024, "written file should be 1MB");

        let _ = std::fs::remove_file(&test_path);
    }

    // ---------------------------------------------------------------
    // WriteTool — output message on success
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_write_success_output_message() {
        let tool = write_tool();
        let test_path = std::env::temp_dir().join(format!("sage_write_msg_{}", std::process::id()));
        let path_str = test_path.to_str().unwrap();

        let output = tool
            .execute(json!({
                "file_path": path_str,
                "content": "hello"
            }))
            .await;
        assert!(!output.is_error, "write should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        // The success message should mention "Successfully" or "wrote" and the file path
        assert!(
            text.to_lowercase().contains("success") || text.to_lowercase().contains("wrote"),
            "success output should contain confirmation, got: {}",
            text
        );

        let _ = std::fs::remove_file(&test_path);
    }

    // ---------------------------------------------------------------
    // Verify byte count in success message
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_write_success_message_byte_count() {
        let tool = write_tool();
        let test_path =
            std::env::temp_dir().join(format!("sage_write_bytecount_{}", std::process::id()));
        let path_str = test_path.to_str().unwrap();
        let content = "hello world!"; // 12 bytes

        let output = tool
            .execute(json!({
                "file_path": path_str,
                "content": content
            }))
            .await;
        assert!(!output.is_error, "write should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(
            text.contains("12"),
            "success message should contain byte count '12', got: {}",
            text
        );

        let _ = std::fs::remove_file(&test_path);
    }
}

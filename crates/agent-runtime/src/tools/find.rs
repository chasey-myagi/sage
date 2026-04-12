// FindTool — file pattern matching (glob-based).

use crate::types::Content;
use std::path::Path;

/// Default recursion depth limit to prevent unbounded filesystem traversal.
const DEFAULT_MAX_DEPTH: usize = 20;

fn error_output(msg: &str) -> super::ToolOutput {
    super::ToolOutput {
        content: vec![Content::Text {
            text: msg.to_string(),
        }],
        is_error: true,
    }
}

/// Async recursive file search using tokio::fs to avoid blocking the runtime.
async fn find_files_recursive(
    base: &Path,
    pattern: &glob::Pattern,
    max_depth: usize,
    depth: usize,
) -> Vec<String> {
    let mut results = Vec::new();
    if depth > max_depth {
        return results;
    }
    let mut entries = match tokio::fs::read_dir(base).await {
        Ok(e) => e,
        Err(_) => return results,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if pattern.matches(&name) {
            results.push(path.to_string_lossy().to_string());
        }
        // Use symlink_metadata (async, doesn't follow symlinks) to avoid:
        // 1. Blocking the tokio runtime with sync std::fs::metadata
        // 2. Infinite recursion through symlink cycles
        let is_dir = match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) => meta.is_dir(),
            Err(_) => false,
        };
        if is_dir {
            let sub = Box::pin(find_files_recursive(&path, pattern, max_depth, depth + 1)).await;
            results.extend(sub);
        }
    }
    results
}

pub struct FindTool;

#[async_trait::async_trait]
impl super::AgentTool for FindTool {
    fn name(&self) -> &str {
        "find"
    }

    fn description(&self) -> &str {
        "Find files matching glob patterns"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string" },
                "file_type": { "type": "string" },
                "depth": { "type": "integer" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> super::ToolOutput {
        let pattern_str = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            Some(_) => return error_output("pattern is empty"),
            None => return error_output("missing required parameter: pattern"),
        };

        let pattern = match glob::Pattern::new(pattern_str) {
            Ok(p) => p,
            Err(e) => return error_output(&format!("Invalid glob pattern: {}", e)),
        };

        let base_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let path = Path::new(base_path);
        if tokio::fs::symlink_metadata(path).await.is_err() {
            return error_output(&format!("Path does not exist: {}", base_path));
        }

        let max_depth = args
            .get("depth")
            .and_then(|v| v.as_u64())
            .map(|d| d as usize)
            .unwrap_or(DEFAULT_MAX_DEPTH);
        let results = find_files_recursive(path, &pattern, max_depth, 0).await;

        let text = if results.is_empty() {
            "No matching files found".into()
        } else {
            results.join("\n")
        };

        super::ToolOutput {
            content: vec![Content::Text { text }],
            is_error: false,
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
        let tool = FindTool;
        assert_eq!(tool.name(), "find");
    }

    #[test]
    fn test_description_not_empty() {
        let tool = FindTool;
        assert!(!tool.description().is_empty());
    }

    // ---------------------------------------------------------------
    // Parameter schema
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_has_pattern() {
        let tool = FindTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("pattern").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("pattern")));
    }

    #[test]
    fn test_schema_has_optional_path() {
        let tool = FindTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("path").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(!required.iter().any(|v| v.as_str() == Some("path")));
    }

    // ---------------------------------------------------------------
    // Argument parsing
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_missing_pattern_returns_error() {
        let tool = FindTool;
        let output = tool.execute(json!({})).await;
        assert!(output.is_error);
    }

    // ---------------------------------------------------------------
    // Empty pattern string
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_empty_pattern_returns_error() {
        let tool = FindTool;
        let output = tool.execute(json!({"pattern": ""})).await;
        assert!(output.is_error, "empty pattern should return error");
    }

    // ---------------------------------------------------------------
    // Optional parameters present in schema
    // ---------------------------------------------------------------

    #[test]
    fn test_schema_has_optional_type_param() {
        let tool = FindTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        // "type" or "file_type" should exist as optional filter
        let has_type = props.get("type").is_some() || props.get("file_type").is_some();
        assert!(has_type, "schema should include a type/file_type filter parameter");
    }

    #[test]
    fn test_schema_has_optional_depth_param() {
        let tool = FindTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        // "depth" or "max_depth" should exist as optional
        let has_depth = props.get("depth").is_some() || props.get("max_depth").is_some();
        assert!(has_depth, "schema should include a depth/max_depth parameter");
    }

    #[test]
    fn test_schema_type_and_depth_are_optional() {
        let tool = FindTool;
        let schema = tool.parameters_schema();
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(
            !required.iter().any(|v| v.as_str() == Some("type")
                || v.as_str() == Some("file_type")),
            "type should not be required"
        );
        assert!(
            !required.iter().any(|v| v.as_str() == Some("depth")
                || v.as_str() == Some("max_depth")),
            "depth should not be required"
        );
    }

    // ---------------------------------------------------------------
    // Glob matching functional tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_find_with_glob_pattern() {
        let tool = FindTool;
        // A typical glob pattern — should be accepted and executed
        let output = tool.execute(json!({"pattern": "*.rs"})).await;
        // Whether it finds files or not depends on cwd, but should not panic
        assert!(!output.content.is_empty());
    }

    #[tokio::test]
    async fn test_find_with_path_and_pattern() {
        let tool = FindTool;
        let output = tool
            .execute(json!({"pattern": "*.toml", "path": "/tmp"}))
            .await;
        // Should execute without panic
        assert!(!output.content.is_empty());
    }

    #[tokio::test]
    async fn test_find_glob_special_chars() {
        let tool = FindTool;
        // Glob with brackets and braces
        let output = tool.execute(json!({"pattern": "*.{rs,toml}"})).await;
        assert!(!output.content.is_empty());
    }

    // ---------------------------------------------------------------
    // Error paths
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_find_nonexistent_path_returns_error() {
        let tool = FindTool;
        let output = tool
            .execute(json!({"pattern": "*.rs", "path": "/nonexistent_dir_12345"}))
            .await;
        assert!(
            output.is_error,
            "searching in nonexistent path should return error"
        );
    }

    #[tokio::test]
    async fn test_find_depth_zero() {
        let tool = FindTool;
        // depth=0 means only the directory itself, no descent
        let output = tool
            .execute(json!({"pattern": "*.rs", "path": "/tmp", "depth": 0}))
            .await;
        // Should not panic; may return empty or error
        assert!(!output.content.is_empty());
    }

    // ---------------------------------------------------------------
    // FindTool — success path with known files
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_find_success_returns_file_list() {
        let tool = FindTool;
        let dir = std::env::temp_dir().join(format!(
            "agent_caster_find_test_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("alpha.txt"), "a").expect("setup");
        std::fs::write(dir.join("beta.txt"), "b").expect("setup");
        std::fs::write(dir.join("gamma.rs"), "c").expect("setup");

        let output = tool
            .execute(json!({"pattern": "*.txt", "path": dir.to_str().unwrap()}))
            .await;
        assert!(!output.is_error, "find in temp dir should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(
            text.contains("alpha.txt"),
            "output should contain alpha.txt, got: {}",
            text
        );
        assert!(
            text.contains("beta.txt"),
            "output should contain beta.txt, got: {}",
            text
        );
        assert!(
            !text.contains("gamma.rs"),
            "output should NOT contain gamma.rs (pattern is *.txt)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // FindTool — recursive search in nested directories
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_find_recursive_nested_dirs() {
        let tool = FindTool;
        let dir = std::env::temp_dir().join(format!(
            "agent_caster_find_recursive_{}",
            std::process::id()
        ));
        let sub1 = dir.join("sub1");
        let sub2 = dir.join("sub1").join("sub2");
        let _ = std::fs::create_dir_all(&sub2);
        std::fs::write(dir.join("root.log"), "r").expect("setup");
        std::fs::write(sub1.join("level1.log"), "l1").expect("setup");
        std::fs::write(sub2.join("level2.log"), "l2").expect("setup");

        let output = tool
            .execute(json!({"pattern": "*.log", "path": dir.to_str().unwrap()}))
            .await;
        assert!(!output.is_error, "recursive find should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(
            text.contains("root.log"),
            "should find root.log, got: {}",
            text
        );
        assert!(
            text.contains("level1.log"),
            "should find level1.log in sub1, got: {}",
            text
        );
        assert!(
            text.contains("level2.log"),
            "should find level2.log in sub1/sub2, got: {}",
            text
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // Verify depth=1 stops at correct level
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_find_depth_one_limits_recursion() {
        let tool = FindTool;
        let dir = std::env::temp_dir().join(format!(
            "agent_caster_find_depth1_{}",
            std::process::id()
        ));
        let sub = dir.join("sub");
        let deep = sub.join("deep");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&deep);
        std::fs::write(dir.join("a.txt"), "a").expect("setup");
        std::fs::write(sub.join("b.txt"), "b").expect("setup");
        std::fs::write(deep.join("c.txt"), "c").expect("setup");

        // depth=1 means initial call (depth=0) + one level of recursion (depth=1)
        // So root-level files and sub/ files should be found, but deep/ files should NOT
        let output = tool
            .execute(json!({
                "pattern": "*.txt",
                "path": dir.to_str().unwrap(),
                "depth": 1
            }))
            .await;
        assert!(!output.is_error, "find with depth=1 should succeed");
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(
            text.contains("a.txt"),
            "depth=1 should find a.txt at root level, got: {}",
            text
        );
        assert!(
            text.contains("b.txt"),
            "depth=1 should find b.txt one level deep, got: {}",
            text
        );
        assert!(
            !text.contains("c.txt"),
            "depth=1 should NOT find c.txt two levels deep, got: {}",
            text
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

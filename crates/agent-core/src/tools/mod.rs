// Tools module — Phase 3
// AgentTool trait, ToolRegistry, parallel/sequential execution.

pub mod backend;
pub mod bash;
pub mod craft_manage;
pub mod edit;
pub mod find;
pub mod grep;
pub mod ls;
pub mod policy;
pub mod read;
pub mod truncate;
pub mod write;

use crate::types::Content;

/// Output returned by any tool execution.
pub struct ToolOutput {
    pub content: Vec<Content>,
    pub is_error: bool,
}

/// Trait that all agent tools implement.
#[async_trait::async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> ToolOutput;
}

// Task #71: blanket impl so `Arc<dyn AgentTool>` satisfies
// `Box::new(…): Box<dyn AgentTool>` without the old `ArcTool` wrapper.
#[async_trait::async_trait]
impl<T: ?Sized + AgentTool> AgentTool for std::sync::Arc<T> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn description(&self) -> &str {
        (**self).description()
    }
    fn parameters_schema(&self) -> serde_json::Value {
        (**self).parameters_schema()
    }
    async fn execute(&self, args: serde_json::Value) -> ToolOutput {
        (**self).execute(args).await
    }
}

/// Registry holding all available tools.
pub struct ToolRegistry {
    tools: Vec<Box<dyn AgentTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn AgentTool>) {
        let name = tool.name().to_string();
        self.tools.retain(|t| t.name() != name);
        self.tools.push(tool);
    }

    pub fn list(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn AgentTool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    pub fn schemas(&self) -> Vec<serde_json::Value> {
        self.tools.iter().map(|t| t.parameters_schema()).collect()
    }
}

/// Factory function: create a tool by name, using the given backend for I/O.
///
/// For tools that need extra context beyond `backend` (e.g. `craft_manage`
/// which is workspace-scoped not sandbox-scoped), use
/// [`create_workspace_tool`] instead.
pub fn create_tool(
    name: &str,
    backend: std::sync::Arc<dyn backend::ToolBackend>,
) -> Option<Box<dyn AgentTool>> {
    match name {
        "bash" => Some(Box::new(bash::BashTool(backend))),
        "read" => Some(Box::new(read::ReadTool(backend))),
        "write" => Some(Box::new(write::WriteTool(backend))),
        "edit" => Some(Box::new(edit::EditTool(backend))),
        "grep" => Some(Box::new(grep::GrepTool(backend))),
        "find" => Some(Box::new(find::FindTool(backend))),
        "ls" => Some(Box::new(ls::LsTool(backend))),
        _ => None,
    }
}

/// Factory for workspace-scoped tools (operate on the agent's persistent
/// workspace directory, bypassing the sandbox `ToolBackend`).
///
/// Sprint 10 S10.1: `craft_manage` — creates / lists CRAFT.md in
/// `<workspace>/craft/<name>/`. Unlike `bash` / `read` / `ls` etc., these
/// tools don't go through the sandbox because they write agent-private data
/// (SOPs, templates, craft scores) that the sandbox has no need to see.
pub fn create_workspace_tool(
    name: &str,
    workspace_dir: std::path::PathBuf,
) -> Option<Box<dyn AgentTool>> {
    match name {
        "craft_manage" => Some(Box::new(craft_manage::CraftManageTool::new(workspace_dir))),
        _ => None,
    }
}

/// Execute tool calls concurrently, preserving call order in results.
pub async fn execute_parallel(
    registry: &ToolRegistry,
    calls: Vec<(String, serde_json::Value)>,
) -> Vec<ToolOutput> {
    use futures::future::join_all;

    let futs: Vec<_> = calls
        .into_iter()
        .map(|(name, args)| async move {
            match registry.get(&name) {
                Some(tool) => tool.execute(args).await,
                None => ToolOutput {
                    content: vec![Content::Text {
                        text: format!("Unknown tool: {}", name),
                    }],
                    is_error: true,
                },
            }
        })
        .collect();

    join_all(futs).await
}

/// Execute tool calls sequentially, preserving call order in results.
pub async fn execute_sequential(
    registry: &ToolRegistry,
    calls: Vec<(String, serde_json::Value)>,
) -> Vec<ToolOutput> {
    let mut results = Vec::with_capacity(calls.len());
    for (name, args) in calls {
        let output = match registry.get(&name) {
            Some(tool) => tool.execute(args).await,
            None => ToolOutput {
                content: vec![Content::Text {
                    text: format!("Unknown tool: {}", name),
                }],
                is_error: true,
            },
        };
        results.push(output);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::backend::LocalBackend;
    use crate::types::Content;
    use serde_json::json;

    // ---------------------------------------------------------------
    // Mock tool for testing trait + registry
    // ---------------------------------------------------------------

    struct MockTool {
        tool_name: String,
        delay_ms: u64,
    }

    impl MockTool {
        fn new(name: &str) -> Self {
            Self {
                tool_name: name.to_string(),
                delay_ms: 0,
            }
        }

        fn with_delay(name: &str, delay_ms: u64) -> Self {
            Self {
                tool_name: name.to_string(),
                delay_ms,
            }
        }
    }

    #[async_trait::async_trait]
    impl AgentTool for MockTool {
        fn name(&self) -> &str {
            &self.tool_name
        }

        fn description(&self) -> &str {
            "A mock tool for testing"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                },
                "required": ["input"]
            })
        }

        async fn execute(&self, args: serde_json::Value) -> ToolOutput {
            if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }
            let input = args
                .get("input")
                .and_then(|v| v.as_str())
                .unwrap_or("no input");
            ToolOutput {
                content: vec![Content::Text {
                    text: format!("{}:{}", self.tool_name, input),
                }],
                is_error: false,
            }
        }
    }

    // ---------------------------------------------------------------
    // ToolOutput
    // ---------------------------------------------------------------

    #[test]
    fn test_tool_output_success() {
        let output = ToolOutput {
            content: vec![Content::Text { text: "ok".into() }],
            is_error: false,
        };
        assert!(!output.is_error);
        assert_eq!(output.content.len(), 1);
    }

    #[test]
    fn test_tool_output_error() {
        let output = ToolOutput {
            content: vec![Content::Text {
                text: "file not found".into(),
            }],
            is_error: true,
        };
        assert!(output.is_error);
    }

    // ---------------------------------------------------------------
    // ToolRegistry
    // ---------------------------------------------------------------

    #[test]
    fn test_registry_new_is_empty() {
        let registry = ToolRegistry::new();
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_registry_register_and_list() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("bash")));
        registry.register(Box::new(MockTool::new("read")));

        let names = registry.list();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"read"));
    }

    #[test]
    fn test_registry_get_existing_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("bash")));

        let tool = registry.get("bash");
        assert!(tool.is_some());
        assert_eq!(tool.unwrap().name(), "bash");
    }

    #[test]
    fn test_registry_get_nonexistent_tool() {
        let registry = ToolRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_schemas() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("bash")));
        registry.register(Box::new(MockTool::new("read")));

        let schemas = registry.schemas();
        assert_eq!(schemas.len(), 2);
        for schema in &schemas {
            assert!(schema.get("type").is_some());
            assert_eq!(schema["type"], "object");
        }
    }

    #[test]
    fn test_registry_list_order_matches_registration() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("alpha")));
        registry.register(Box::new(MockTool::new("beta")));
        registry.register(Box::new(MockTool::new("gamma")));

        let names = registry.list();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    // ---------------------------------------------------------------
    // AgentTool trait via MockTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_mock_tool_name() {
        let tool = MockTool::new("test_tool");
        assert_eq!(tool.name(), "test_tool");
    }

    #[tokio::test]
    async fn test_mock_tool_description() {
        let tool = MockTool::new("test_tool");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn test_mock_tool_execute() {
        let tool = MockTool::new("mock");
        let output = tool.execute(json!({"input": "hello"})).await;
        assert!(!output.is_error);
        match &output.content[0] {
            Content::Text { text } => assert_eq!(text, "mock:hello"),
            _ => panic!("expected Text content"),
        }
    }

    // ---------------------------------------------------------------
    // execute_parallel
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_parallel_empty_calls() {
        let registry = ToolRegistry::new();
        let results = execute_parallel(&registry, vec![]).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_parallel_single_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("bash")));

        let calls = vec![("bash".to_string(), json!({"input": "ls"}))];
        let results = execute_parallel(&registry, calls).await;
        assert_eq!(results.len(), 1);
        assert!(!results[0].is_error);
    }

    #[tokio::test]
    async fn test_parallel_multiple_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("bash")));
        registry.register(Box::new(MockTool::new("read")));

        let calls = vec![
            ("bash".to_string(), json!({"input": "ls"})),
            ("read".to_string(), json!({"input": "file.txt"})),
        ];
        let results = execute_parallel(&registry, calls).await;
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_parallel_unknown_tool_returns_error() {
        let registry = ToolRegistry::new();
        let calls = vec![("nonexistent".to_string(), json!({"input": "x"}))];
        let results = execute_parallel(&registry, calls).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
    }

    #[tokio::test]
    async fn test_parallel_actually_concurrent() {
        // Two tools each sleep 50ms. If parallel, total < 100ms.
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::with_delay("slow1", 50)));
        registry.register(Box::new(MockTool::with_delay("slow2", 50)));

        let calls = vec![
            ("slow1".to_string(), json!({"input": "a"})),
            ("slow2".to_string(), json!({"input": "b"})),
        ];

        let start = std::time::Instant::now();
        let results = execute_parallel(&registry, calls).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 2);
        // If truly parallel, should complete in ~50ms, not ~100ms
        assert!(
            elapsed.as_millis() < 90,
            "expected parallel execution, took {}ms",
            elapsed.as_millis()
        );
    }

    // ---------------------------------------------------------------
    // execute_sequential
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_sequential_empty_calls() {
        let registry = ToolRegistry::new();
        let results = execute_sequential(&registry, vec![]).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_sequential_preserves_order() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("tool_a")));
        registry.register(Box::new(MockTool::new("tool_b")));

        let calls = vec![
            ("tool_a".to_string(), json!({"input": "first"})),
            ("tool_b".to_string(), json!({"input": "second"})),
        ];
        let results = execute_sequential(&registry, calls).await;
        assert_eq!(results.len(), 2);

        match &results[0].content[0] {
            Content::Text { text } => assert!(text.contains("first")),
            _ => panic!("expected Text"),
        }
        match &results[1].content[0] {
            Content::Text { text } => assert!(text.contains("second")),
            _ => panic!("expected Text"),
        }
    }

    #[tokio::test]
    async fn test_sequential_unknown_tool_returns_error() {
        let registry = ToolRegistry::new();
        let calls = vec![("ghost".to_string(), json!({"input": "x"}))];
        let results = execute_sequential(&registry, calls).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
    }

    // ---------------------------------------------------------------
    // ToolRegistry — duplicate registration behavior
    // ---------------------------------------------------------------

    #[test]
    fn test_registry_duplicate_name_overwrites() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("bash")));
        registry.register(Box::new(MockTool::new("bash")));

        // After registering "bash" twice, list should contain exactly one entry
        // (overwrite semantics) — or two if append semantics.
        // Either way, get("bash") must return a tool.
        let names = registry.list();
        let bash_count = names.iter().filter(|n| **n == "bash").count();
        // Exactly 1 means overwrite; 2 means append. We assert a definite behavior.
        assert!(
            bash_count == 1,
            "duplicate register should overwrite: expected 1, got {}",
            bash_count
        );
    }

    // ---------------------------------------------------------------
    // execute_parallel — mixed known + unknown tools
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_parallel_mixed_known_and_unknown_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("bash")));

        let calls = vec![
            ("bash".to_string(), json!({"input": "ls"})),
            ("nonexistent".to_string(), json!({"input": "x"})),
            ("bash".to_string(), json!({"input": "pwd"})),
        ];
        let results = execute_parallel(&registry, calls).await;
        assert_eq!(results.len(), 3);
        // First call: known tool, should succeed
        assert!(!results[0].is_error);
        // Second call: unknown tool, should error
        assert!(results[1].is_error);
        // Third call: known tool, should succeed
        assert!(!results[2].is_error);
    }

    // ---------------------------------------------------------------
    // execute_parallel — result order matches call order
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_parallel_result_order_matches_call_order() {
        let mut registry = ToolRegistry::new();
        // Register tools with different delays to verify ordering is preserved
        registry.register(Box::new(MockTool::with_delay("slow", 40)));
        registry.register(Box::new(MockTool::with_delay("fast", 5)));

        let calls = vec![
            ("slow".to_string(), json!({"input": "first"})),
            ("fast".to_string(), json!({"input": "second"})),
        ];
        let results = execute_parallel(&registry, calls).await;
        assert_eq!(results.len(), 2);

        // results[0] should correspond to the "slow" call (input "first")
        match &results[0].content[0] {
            Content::Text { text } => assert!(
                text.contains("first"),
                "results[0] should contain 'first', got: {}",
                text
            ),
            _ => panic!("expected Text content"),
        }
        // results[1] should correspond to the "fast" call (input "second")
        match &results[1].content[0] {
            Content::Text { text } => assert!(
                text.contains("second"),
                "results[1] should contain 'second', got: {}",
                text
            ),
            _ => panic!("expected Text content"),
        }
    }

    // ---------------------------------------------------------------
    // execute_sequential — continues after unknown tool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_sequential_continues_after_unknown_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("tool_a")));
        registry.register(Box::new(MockTool::new("tool_b")));

        let calls = vec![
            ("tool_a".to_string(), json!({"input": "one"})),
            ("unknown".to_string(), json!({"input": "two"})),
            ("tool_b".to_string(), json!({"input": "three"})),
        ];
        let results = execute_sequential(&registry, calls).await;
        assert_eq!(results.len(), 3);

        // First call succeeds
        assert!(!results[0].is_error);
        match &results[0].content[0] {
            Content::Text { text } => assert!(text.contains("one")),
            _ => panic!("expected Text"),
        }

        // Second call is unknown — error
        assert!(results[1].is_error);

        // Third call should still execute (not short-circuit)
        assert!(!results[2].is_error);
        match &results[2].content[0] {
            Content::Text { text } => assert!(text.contains("three")),
            _ => panic!("expected Text"),
        }
    }

    // ---------------------------------------------------------------
    // Large batch parallel execution
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_parallel_large_batch() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::with_delay("tool", 5)));

        let calls: Vec<_> = (0..20)
            .map(|i| ("tool".to_string(), json!({"input": format!("item_{}", i)})))
            .collect();

        let start = std::time::Instant::now();
        let results = execute_parallel(&registry, calls).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 20);
        assert!(results.iter().all(|r| !r.is_error));
        // 20 tasks * 5ms each. If parallel, should complete in ~5-10ms, not 100ms
        assert!(
            elapsed.as_millis() < 50,
            "20 parallel tasks of 5ms each should complete in under 50ms, took {}ms",
            elapsed.as_millis()
        );
    }

    // ---------------------------------------------------------------
    // Tool output content structure
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_tool_output_content_structure() {
        let tool = MockTool::new("test");
        let output = tool.execute(json!({"input": "hello"})).await;
        assert!(!output.is_error);
        assert_eq!(output.content.len(), 1);
        // Content should be Text type with tool_name:input format
        match &output.content[0] {
            Content::Text { text } => {
                assert!(text.contains("test"));
                assert!(text.contains("hello"));
            }
            _ => panic!("expected Text content"),
        }
    }

    // ---------------------------------------------------------------
    // Schema generation from registry
    // ---------------------------------------------------------------

    #[test]
    fn test_registry_schemas_match_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool::new("bash")));
        registry.register(Box::new(MockTool::new("read")));

        let schemas = registry.schemas();
        let names = registry.list();

        assert_eq!(schemas.len(), names.len());
        // Each schema should be a valid JSON object
        for schema in &schemas {
            assert!(schema.is_object());
            assert!(schema.get("type").is_some());
        }
    }

    // ---------------------------------------------------------------
    // Cross-tool integration tests with real tools
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_registry_real_tools_write_then_read() {
        let mut registry = ToolRegistry::new();
        let backend = LocalBackend::new();
        registry.register(Box::new(super::read::ReadTool(backend.clone())));
        registry.register(Box::new(super::write::WriteTool(backend)));

        // Create a unique temp file path
        let dir = std::env::temp_dir();
        let file_path = dir.join(format!("sage_cross_tool_{}", std::process::id()));
        let path_str = file_path.to_str().unwrap().to_string();

        // Write then read via execute_parallel
        let write_call = (
            "write".to_string(),
            json!({"file_path": path_str, "content": "cross-tool test content"}),
        );
        let write_results = execute_parallel(&registry, vec![write_call]).await;
        assert_eq!(write_results.len(), 1);
        assert!(!write_results[0].is_error, "write should succeed");

        let read_call = ("read".to_string(), json!({"file_path": path_str}));
        let read_results = execute_parallel(&registry, vec![read_call]).await;
        assert_eq!(read_results.len(), 1);
        assert!(!read_results[0].is_error, "read should succeed");

        match &read_results[0].content[0] {
            Content::Text { text } => {
                assert!(
                    text.contains("cross-tool test content"),
                    "read output should contain written content, got: {}",
                    text
                );
            }
            _ => panic!("expected Text content"),
        }

        let _ = std::fs::remove_file(&file_path);
    }

    #[tokio::test]
    async fn test_parallel_all_unknown_tools_return_errors() {
        let registry = ToolRegistry::new();
        let calls = vec![
            ("nonexistent_a".to_string(), json!({"input": "x"})),
            ("nonexistent_b".to_string(), json!({"input": "y"})),
            ("nonexistent_c".to_string(), json!({"input": "z"})),
        ];
        let results = execute_parallel(&registry, calls).await;
        assert_eq!(results.len(), 3);
        for (i, r) in results.iter().enumerate() {
            assert!(r.is_error, "result[{}] should be error for unknown tool", i);
            match &r.content[0] {
                Content::Text { text } => {
                    assert!(
                        text.contains("Unknown tool"),
                        "error message should contain 'Unknown tool', got: {}",
                        text
                    );
                }
                _ => panic!("expected Text content"),
            }
        }
    }

    #[test]
    fn test_registry_all_seven_real_tools() {
        let mut registry = ToolRegistry::new();
        let tool_names = ["bash", "read", "write", "edit", "grep", "find", "ls"];
        let backend = LocalBackend::new();
        for name in &tool_names {
            let tool = super::create_tool(name, backend.clone())
                .expect(&format!("create_tool({}) should succeed", name));
            registry.register(tool);
        }

        let names = registry.list();
        assert_eq!(names.len(), 7, "should have 7 tools registered");
        for expected in &tool_names {
            assert!(
                names.contains(expected),
                "registry should contain '{}', got: {:?}",
                expected,
                names
            );
        }
    }

    // ---------------------------------------------------------------
    // create_tool negative path
    // ---------------------------------------------------------------

    #[test]
    fn test_create_tool_unknown_returns_none() {
        assert!(super::create_tool("nonexistent_tool_xyz", LocalBackend::new()).is_none());
    }

    // ---------------------------------------------------------------
    // Cross-tool sequential workflow: Write→Edit→Read
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_sequential_write_edit_read_workflow() {
        let mut registry = ToolRegistry::new();
        let backend = LocalBackend::new();
        registry.register(Box::new(super::write::WriteTool(backend.clone())));
        registry.register(Box::new(super::edit::EditTool(backend.clone())));
        registry.register(Box::new(super::read::ReadTool(backend)));

        let dir = std::env::temp_dir();
        let file_path = dir.join(format!("sage_seq_workflow_{}", std::process::id()));
        let path_str = file_path.to_str().unwrap().to_string();

        // 1. Write file with "hello world"
        // 2. Edit: replace "hello" with "goodbye"
        // 3. Read file and verify "goodbye world"
        let calls = vec![
            (
                "write".to_string(),
                json!({"file_path": path_str, "content": "hello world"}),
            ),
            (
                "edit".to_string(),
                json!({
                    "file_path": path_str,
                    "old_text": "hello",
                    "new_text": "goodbye"
                }),
            ),
            ("read".to_string(), json!({"file_path": path_str})),
        ];
        let results = execute_sequential(&registry, calls).await;
        assert_eq!(results.len(), 3);

        // Write should succeed
        assert!(!results[0].is_error, "write step should succeed");
        // Edit should succeed
        assert!(!results[1].is_error, "edit step should succeed");
        // Read should succeed and contain "goodbye world"
        assert!(!results[2].is_error, "read step should succeed");
        match &results[2].content[0] {
            Content::Text { text } => {
                assert!(
                    text.contains("goodbye world"),
                    "final read should contain 'goodbye world', got: {}",
                    text
                );
            }
            _ => panic!("expected Text content"),
        }

        let _ = std::fs::remove_file(&file_path);
    }

    // ---------------------------------------------------------------
    // Cross-tool pipeline: Find→Read
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_find_then_read_pipeline() {
        let mut registry = ToolRegistry::new();
        let backend = LocalBackend::new();
        registry.register(Box::new(super::find::FindTool(backend.clone())));
        registry.register(Box::new(super::read::ReadTool(backend)));

        let dir = std::env::temp_dir().join(format!("sage_find_read_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("target.dat"), "secret payload").expect("setup file");

        // Step 1: Find the file
        let find_calls = vec![(
            "find".to_string(),
            json!({"pattern": "target.dat", "path": dir.to_str().unwrap()}),
        )];
        let find_results = execute_sequential(&registry, find_calls).await;
        assert!(!find_results[0].is_error, "find should succeed");

        let found_path = match &find_results[0].content[0] {
            Content::Text { text } => text.trim().to_string(),
            _ => panic!("expected Text content"),
        };
        assert!(
            found_path.contains("target.dat"),
            "find output should contain target.dat, got: {}",
            found_path
        );

        // Step 2: Read the found file
        let read_calls = vec![("read".to_string(), json!({"file_path": found_path}))];
        let read_results = execute_sequential(&registry, read_calls).await;
        assert!(!read_results[0].is_error, "read should succeed");
        match &read_results[0].content[0] {
            Content::Text { text } => {
                assert!(
                    text.contains("secret payload"),
                    "read output should contain file content, got: {}",
                    text
                );
            }
            _ => panic!("expected Text content"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── create_workspace_tool — Sprint 10 S10.1 registration path ────────

    /// Linus v1 blocker #2: `craft_manage` is registerable through the
    /// factory → ToolRegistry → agent tool-calling path. This test catches
    /// the failure mode where the tool is written but never pluggable.
    #[tokio::test]
    async fn create_workspace_tool_craft_manage_returns_some_and_registers() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = create_workspace_tool("craft_manage", tmp.path().to_path_buf())
            .expect("craft_manage must be registered in create_workspace_tool");
        assert_eq!(tool.name(), "craft_manage");
        // Proves end-to-end: factory produces a tool that accepts the
        // standard AgentTool.execute() calling convention.
        let out = tool
            .execute(serde_json::json!({ "action": "list" }))
            .await;
        assert!(!out.is_error, "list on empty workspace must succeed");

        // And the factory-built tool registers into a ToolRegistry cleanly.
        let mut registry = ToolRegistry::new();
        let again = create_workspace_tool("craft_manage", tmp.path().to_path_buf()).unwrap();
        registry.register(again);
        assert!(
            registry.get("craft_manage").is_some(),
            "tool must be retrievable after register"
        );
    }

    #[test]
    fn create_workspace_tool_unknown_name_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(create_workspace_tool("not_a_tool", tmp.path().to_path_buf()).is_none());
    }
}

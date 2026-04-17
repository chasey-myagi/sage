// CraftManageTool — workspace-scoped craft (SOP/script/template) management.

use std::path::PathBuf;

use crate::tools::{AgentTool, ToolOutput};
use crate::types::{now_secs, Content};
use serde::{Deserialize, Serialize};

pub struct CraftManageTool {
    workspace_dir: PathBuf,
}

impl CraftManageTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

fn error_output(msg: &str) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text {
            text: msg.to_string(),
        }],
        is_error: true,
    }
}

fn ok_output(msg: &str) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text {
            text: msg.to_string(),
        }],
        is_error: false,
    }
}

/// Validate that a craft name is safe.
///
/// Rejects:
///   - empty
///   - path separators / traversal: `/` `\` `..` `.` `.trash`
///   - YAML structural characters: `\n` `\r` `:` `#` — without these a
///     malicious name like `"evil\nversion: 999"` would inject fake
///     frontmatter fields (code-review Critical #1). MVP uses hand-rolled
///     YAML so the writer can't defend via proper escaping.
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err(format!("invalid name '{}': must not contain path separators", name));
    }
    if name == ".." || name == "." || name == ".trash" {
        return Err(format!("invalid name '{}'", name));
    }
    if name.contains("..") {
        return Err(format!("invalid name '{}': must not contain '..'", name));
    }
    // YAML frontmatter injection guard (code-review C1).
    for ch in ['\n', '\r', ':', '#'] {
        if name.contains(ch) {
            return Err(format!(
                "invalid name '{}': must not contain YAML-reserved character '{}'",
                name.escape_debug(),
                ch.escape_debug(),
            ));
        }
    }
    Ok(())
}

/// Entry returned by the list action.
#[derive(Debug, Serialize, Deserialize)]
struct CraftEntry {
    name: String,
    #[serde(rename = "type")]
    craft_type: String,
    tags: Vec<String>,
    version: u64,
}

/// Parse frontmatter from CRAFT.md content.
/// Returns None if frontmatter is malformed or missing required fields.
fn parse_frontmatter(content: &str) -> Option<CraftEntry> {
    // Must start with "---\n"
    if !content.starts_with("---\n") {
        return None;
    }
    // Find the closing ---
    let rest = &content[4..];
    let end = rest.find("\n---")?;
    let fm = &rest[..end];

    let mut name: Option<String> = None;
    let mut craft_type: Option<String> = None;
    let mut tags: Option<Vec<String>> = None;
    let mut version: Option<u64> = None;

    for line in fm.lines() {
        if let Some(val) = line.strip_prefix("name: ") {
            name = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("type: ") {
            craft_type = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("tags: ") {
            // Parse YAML flow sequence: [tag1, tag2] or []
            let val = val.trim();
            let inner = val.strip_prefix('[')?.strip_suffix(']')?;
            let inner = inner.trim();
            if inner.is_empty() {
                tags = Some(vec![]);
            } else {
                let parsed: Vec<String> = inner
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                tags = Some(parsed);
            }
        } else if let Some(val) = line.strip_prefix("version: ") {
            version = val.trim().parse::<u64>().ok();
        }
    }

    Some(CraftEntry {
        name: name?,
        craft_type: craft_type?,
        tags: tags?,
        version: version?,
    })
}

#[async_trait::async_trait]
impl AgentTool for CraftManageTool {
    fn name(&self) -> &str {
        "craft_manage"
    }

    fn description(&self) -> &str {
        "Manage crafts (SOPs, scripts, templates) in the workspace. \
         Supports create and list actions for craft artifacts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list"],
                    "description": "Action to perform: 'create' a new craft or 'list' existing crafts"
                },
                "name": {
                    "type": "string"
                },
                "content": {
                    "type": "string"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "type": {
                    "type": "string"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> ToolOutput {
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return error_output("missing required parameter: action"),
        };

        match action {
            "create" => self.execute_create(&args),
            "list" => self.execute_list(),
            other => error_output(&format!("unknown action '{}'", other)),
        }
    }
}

impl CraftManageTool {
    fn execute_create(&self, args: &serde_json::Value) -> ToolOutput {
        // Validate name
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return error_output("missing required parameter: name"),
        };
        if let Err(e) = validate_name(name) {
            return error_output(&e);
        }

        // Validate content
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return error_output("missing required parameter: content"),
        };
        if content.is_empty() {
            return error_output("content must not be empty");
        }

        // Optional fields
        let craft_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("prompt");
        let tags: Vec<String> = args
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let craft_dir = self.workspace_dir.join("craft").join(name);
        let craft_md_path = craft_dir.join("CRAFT.md");

        if let Err(e) = std::fs::create_dir_all(&craft_dir) {
            return error_output(&format!("failed to create craft directory: {}", e));
        }

        // Build frontmatter + body
        let now = now_secs();
        let tags_str = if tags.is_empty() {
            "[]".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        };
        let file_content = format!(
            "---\nname: {}\ntype: {}\ntags: {}\ncreated_at: {}\nversion: 1\n---\n\n{}",
            name, craft_type, tags_str, now, content
        );

        // Atomic create-or-fail — avoids the TOCTOU race of checking
        // `.exists()` and then `write()`. Two concurrent `create` of the
        // same name are guaranteed to produce exactly one winner.
        // (Linus v1 blocker #1)
        use std::io::Write as _;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&craft_md_path)
        {
            Ok(mut f) => {
                if let Err(e) = f.write_all(file_content.as_bytes()) {
                    // Code-review I2: partial write leaves a zero-byte file
                    // whose `AlreadyExists` on next `create` would mislead
                    // the user. Best-effort remove — if cleanup itself fails
                    // (read-only FS?), the original write error still wins.
                    drop(f);
                    let _ = std::fs::remove_file(&craft_md_path);
                    return error_output(&format!("failed to write CRAFT.md: {}", e));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return error_output(&format!("craft '{}' already exists", name));
            }
            Err(e) => {
                return error_output(&format!("failed to create CRAFT.md: {}", e));
            }
        }

        ok_output(&format!(
            "created craft '{}' at craft/{}/CRAFT.md",
            name, name
        ))
    }

    fn execute_list(&self) -> ToolOutput {
        let craft_base = self.workspace_dir.join("craft");

        // Collect all CRAFT.md paths via glob
        let pattern = format!("{}/*/CRAFT.md", craft_base.display());
        let paths = match glob::glob(&pattern) {
            Ok(p) => p,
            Err(_) => {
                return ok_output("[]");
            }
        };

        let mut entries: Vec<CraftEntry> = Vec::new();
        for path_result in paths {
            let path = match path_result {
                Ok(p) => p,
                Err(_) => continue,
            };
            let file_content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "skipping unreadable craft file in list",
                    );
                    continue;
                }
            };
            match parse_frontmatter(&file_content) {
                Some(entry) => entries.push(entry),
                None => {
                    // Linus v1 blocker #3: malformed frontmatter is a silent
                    // skip without observability. Warn-log so operators can
                    // see their craft file is broken instead of wondering
                    // why the dropdown is missing an entry.
                    tracing::warn!(
                        path = %path.display(),
                        "skipping craft with malformed frontmatter",
                    );
                }
            }
        }

        // Sort by name for stable output
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
        ok_output(&json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn make_tool(dir: &TempDir) -> CraftManageTool {
        CraftManageTool::new(dir.path().to_path_buf())
    }

    // Helper: extract text from first content item.
    fn text_of(output: &ToolOutput) -> &str {
        match &output.content[0] {
            Content::Text { text } => text.as_str(),
            _ => panic!("expected Content::Text"),
        }
    }

    // ---------------------------------------------------------------
    // Tool trait 基础 (1-4)
    // ---------------------------------------------------------------

    #[test]
    fn craft_manage_tool_name_is_craft_manage() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        assert_eq!(tool.name(), "craft_manage");
    }

    #[test]
    fn craft_manage_tool_description_non_empty() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn craft_manage_tool_schema_has_action_enum_create_and_list() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let schema = tool.parameters_schema(); // panics until implemented
        let props = schema["properties"].as_object().unwrap();
        let action = &props["action"];
        let variants = action["enum"].as_array().unwrap();
        let strs: Vec<&str> = variants.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(strs.contains(&"create"), "enum must contain 'create'");
        assert!(strs.contains(&"list"), "enum must contain 'list'");
    }

    #[test]
    fn craft_manage_tool_schema_requires_action() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let schema = tool.parameters_schema(); // panics until implemented
        let required = schema["required"].as_array().unwrap();
        let strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(strs.contains(&"action"), "'action' must be in required");
    }

    // ---------------------------------------------------------------
    // create — happy path (5-12)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn create_writes_craft_md_at_expected_path() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        tool.execute(json!({
            "action": "create",
            "name": "deploy",
            "content": "## Deploy steps"
        }))
        .await; // panics (todo!())
        // After implementation:
        let craft_path = dir.path().join("craft/deploy/CRAFT.md");
        assert!(craft_path.exists(), "CRAFT.md should exist at expected path");
    }

    #[tokio::test]
    async fn create_writes_frontmatter_with_name_type_version_created_at() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        tool.execute(json!({
            "action": "create",
            "name": "deploy",
            "content": "## Deploy steps"
        }))
        .await;
        let craft_path = dir.path().join("craft/deploy/CRAFT.md");
        let content = fs::read_to_string(&craft_path).unwrap();
        assert!(content.contains("name: deploy"), "frontmatter must contain name");
        assert!(content.contains("type: prompt"), "frontmatter must contain default type");
        assert!(content.contains("version: 1"), "frontmatter must contain version: 1");
        // created_at should be a number (Unix timestamp)
        assert!(
            content.contains("created_at:"),
            "frontmatter must contain created_at"
        );
        let after_created_at = content.split("created_at:").nth(1).unwrap().trim();
        let ts_str = after_created_at.lines().next().unwrap().trim();
        ts_str.parse::<u64>().expect("created_at must be a valid Unix timestamp");
    }

    #[tokio::test]
    async fn create_writes_body_after_frontmatter_blank_line() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let body = "## Deploy steps\nRun cargo build\n";
        tool.execute(json!({
            "action": "create",
            "name": "deploy",
            "content": body
        }))
        .await;
        let craft_path = dir.path().join("craft/deploy/CRAFT.md");
        let content = fs::read_to_string(&craft_path).unwrap();
        // After closing ---, there should be a blank line, then the body.
        let after_fence = content.splitn(3, "---").nth(2).unwrap();
        // after_fence starts with "\n\n" + body
        assert!(
            after_fence.trim_start_matches('\n').starts_with(body.trim_start()),
            "body should appear verbatim after frontmatter blank line, got: {}",
            after_fence
        );
    }

    #[tokio::test]
    async fn create_returns_success_output_with_relative_path() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool
            .execute(json!({
                "action": "create",
                "name": "deploy",
                "content": "some content"
            }))
            .await;
        assert!(!output.is_error, "create should succeed");
        let t = text_of(&output);
        assert!(
            t.contains("created"),
            "output must contain 'created', got: {}",
            t
        );
        assert!(
            t.contains("deploy"),
            "output must contain craft name, got: {}",
            t
        );
        // relative path like "craft/deploy/CRAFT.md"
        assert!(
            t.contains("craft/deploy"),
            "output must contain relative path, got: {}",
            t
        );
    }

    #[tokio::test]
    async fn create_accepts_tags_array() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        tool.execute(json!({
            "action": "create",
            "name": "deploy",
            "content": "steps",
            "tags": ["devops", "ci"]
        }))
        .await;
        let content = fs::read_to_string(dir.path().join("craft/deploy/CRAFT.md")).unwrap();
        assert!(
            content.contains("devops"),
            "frontmatter tags should contain 'devops', got: {}",
            content
        );
        assert!(
            content.contains("ci"),
            "frontmatter tags should contain 'ci', got: {}",
            content
        );
    }

    #[tokio::test]
    async fn create_accepts_custom_type() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        tool.execute(json!({
            "action": "create",
            "name": "deploy",
            "content": "steps",
            "type": "recipe"
        }))
        .await;
        let content = fs::read_to_string(dir.path().join("craft/deploy/CRAFT.md")).unwrap();
        assert!(
            content.contains("type: recipe"),
            "frontmatter should have type: recipe, got: {}",
            content
        );
    }

    #[tokio::test]
    async fn create_defaults_type_to_prompt_when_absent() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        tool.execute(json!({
            "action": "create",
            "name": "deploy",
            "content": "steps"
        }))
        .await;
        let content = fs::read_to_string(dir.path().join("craft/deploy/CRAFT.md")).unwrap();
        assert!(
            content.contains("type: prompt"),
            "missing type should default to 'prompt', got: {}",
            content
        );
    }

    #[tokio::test]
    async fn create_empty_tags_defaults_to_empty_array() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        tool.execute(json!({
            "action": "create",
            "name": "deploy",
            "content": "steps"
        }))
        .await;
        let content = fs::read_to_string(dir.path().join("craft/deploy/CRAFT.md")).unwrap();
        assert!(
            content.contains("tags: []"),
            "missing tags should default to empty array, got: {}",
            content
        );
    }

    // ---------------------------------------------------------------
    // create — error path (13-21)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn create_rejects_missing_name() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool
            .execute(json!({"action": "create", "content": "body"}))
            .await;
        assert!(output.is_error, "missing name should return error");
        assert!(
            text_of(&output).to_lowercase().contains("name"),
            "error should mention 'name', got: {}",
            text_of(&output)
        );
    }

    #[tokio::test]
    async fn create_rejects_empty_name() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool
            .execute(json!({"action": "create", "name": "", "content": "body"}))
            .await;
        assert!(output.is_error, "empty name should return error");
    }

    #[tokio::test]
    async fn create_rejects_path_traversal_slash() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool
            .execute(json!({"action": "create", "name": "../etc", "content": "body"}))
            .await;
        assert!(output.is_error, "name with slash should be rejected");
    }

    #[tokio::test]
    async fn create_rejects_path_traversal_dotdot() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool
            .execute(json!({"action": "create", "name": "..", "content": "body"}))
            .await;
        assert!(output.is_error, "name '..' should be rejected");
    }

    #[tokio::test]
    async fn create_rejects_missing_content() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool
            .execute(json!({"action": "create", "name": "deploy"}))
            .await;
        assert!(output.is_error, "missing content should return error");
    }

    #[tokio::test]
    async fn create_rejects_empty_content() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool
            .execute(json!({"action": "create", "name": "deploy", "content": ""}))
            .await;
        assert!(output.is_error, "empty content should return error");
    }

    #[tokio::test]
    async fn create_rejects_duplicate_craft_name() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        // First create succeeds.
        tool.execute(json!({"action": "create", "name": "deploy", "content": "v1"}))
            .await;
        // Second create with same name should fail.
        let output = tool
            .execute(json!({"action": "create", "name": "deploy", "content": "v2"}))
            .await;
        assert!(output.is_error, "duplicate craft name should return error");
        let t = text_of(&output).to_lowercase();
        assert!(
            t.contains("exists") || t.contains("already"),
            "error must mention 'exists' or 'already', got: {}",
            t
        );
    }

    #[tokio::test]
    async fn create_rejects_unknown_action() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool
            .execute(json!({"action": "unknown-xyz", "name": "x", "content": "y"}))
            .await;
        assert!(output.is_error, "unknown action should return error");
    }

    #[tokio::test]
    async fn create_rejects_missing_action() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool
            .execute(json!({"name": "deploy", "content": "body"}))
            .await;
        assert!(output.is_error, "missing action should return error");
    }

    // ---------------------------------------------------------------
    // list — happy path (22-24)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn list_on_empty_workspace_returns_empty_array() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let output = tool.execute(json!({"action": "list"})).await;
        assert!(!output.is_error, "list on empty workspace should not error");
        assert_eq!(
            text_of(&output).trim(),
            "[]",
            "list on empty workspace must return '[]'"
        );
    }

    #[tokio::test]
    async fn list_after_two_creates_returns_both_entries() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        tool.execute(json!({"action": "create", "name": "deploy", "content": "d"}))
            .await;
        tool.execute(json!({"action": "create", "name": "review", "content": "r"}))
            .await;
        let output = tool.execute(json!({"action": "list"})).await;
        assert!(!output.is_error);
        let arr: serde_json::Value = serde_json::from_str(text_of(&output)).unwrap();
        assert_eq!(
            arr.as_array().unwrap().len(),
            2,
            "list should return 2 entries"
        );
    }

    #[tokio::test]
    async fn list_entry_includes_name_type_tags_version() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        tool.execute(json!({
            "action": "create",
            "name": "deploy",
            "content": "steps",
            "type": "recipe",
            "tags": ["x"]
        }))
        .await;
        let output = tool.execute(json!({"action": "list"})).await;
        assert!(!output.is_error);
        let arr: Vec<serde_json::Value> = serde_json::from_str(text_of(&output)).unwrap();
        assert_eq!(arr.len(), 1);
        let entry = &arr[0];
        assert_eq!(entry["name"], "deploy", "entry.name must be 'deploy'");
        assert_eq!(entry["type"], "recipe", "entry.type must be 'recipe'");
        assert_eq!(entry["version"], 1, "entry.version must be 1");
        let tags = entry["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], "x");
    }

    // ---------------------------------------------------------------
    // list — resilience (25-26)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn list_skips_craft_without_frontmatter() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        // Create a valid craft first.
        tool.execute(json!({"action": "create", "name": "good", "content": "ok"}))
            .await;
        // Manually plant a broken craft (no frontmatter).
        let broken = dir.path().join("craft/broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("CRAFT.md"), "no frontmatter here").unwrap();
        // list should return the good craft only, not crash.
        let output = tool.execute(json!({"action": "list"})).await;
        assert!(!output.is_error, "list should not error on broken craft");
        let arr: Vec<serde_json::Value> = serde_json::from_str(text_of(&output)).unwrap();
        assert_eq!(arr.len(), 1, "broken craft should be skipped");
        assert_eq!(arr[0]["name"], "good");
    }

    #[tokio::test]
    async fn list_skips_craft_missing_craft_md() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        // Create a valid craft.
        tool.execute(json!({"action": "create", "name": "good", "content": "ok"}))
            .await;
        // Create a directory without CRAFT.md.
        let empty = dir.path().join("craft/empty");
        fs::create_dir_all(&empty).unwrap();
        // list should return only the good craft.
        let output = tool.execute(json!({"action": "list"})).await;
        assert!(!output.is_error, "list should not error when CRAFT.md is missing");
        let arr: Vec<serde_json::Value> = serde_json::from_str(text_of(&output)).unwrap();
        assert_eq!(arr.len(), 1, "craft without CRAFT.md should be skipped");
        assert_eq!(arr[0]["name"], "good");
    }

    // ── Code-review post-v1 补测：C1 注入 + I1 round-trip ─────────────────

    /// Code-review Critical #1: name containing `\n` / `\r` / `:` / `#`
    /// must be rejected so attackers can't inject fake frontmatter fields.
    #[tokio::test]
    async fn create_rejects_yaml_reserved_characters_in_name() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        for bad in ["evil\nversion: 999", "foo:bar", "x\rfoo", "foo#bar"] {
            let out = tool
                .execute(json!({ "action": "create", "name": bad, "content": "body" }))
                .await;
            assert!(
                out.is_error,
                "name '{}' should be rejected (YAML injection guard)",
                bad.escape_debug()
            );
        }
        // And nothing was written on disk
        let craft_dir = dir.path().join("craft");
        assert!(
            !craft_dir.exists() || std::fs::read_dir(&craft_dir).unwrap().next().is_none(),
            "no craft directory should have been created"
        );
    }

    /// Code-review Important #1: round-trip guard — create → list must return
    /// an entry whose fields match the create input. Protects S10.3 and
    /// beyond from silent schema drift between the hand-rolled writer and
    /// parser.
    #[tokio::test]
    async fn create_then_list_round_trips_all_frontmatter_fields() {
        let dir = TempDir::new().unwrap();
        let tool = make_tool(&dir);
        let _ = tool
            .execute(json!({
                "action": "create",
                "name": "deploy",
                "content": "body",
                "type": "recipe",
                "tags": ["devops", "ci"],
            }))
            .await;
        let out = tool.execute(json!({"action": "list"})).await;
        let arr: Vec<serde_json::Value> = serde_json::from_str(text_of(&out)).unwrap();
        assert_eq!(arr.len(), 1);
        let entry = &arr[0];
        assert_eq!(entry["name"], "deploy");
        assert_eq!(entry["type"], "recipe");
        assert_eq!(entry["tags"], json!(["devops", "ci"]));
        assert_eq!(entry["version"], 1);
    }
}

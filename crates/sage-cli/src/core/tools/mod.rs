//! Built-in tool definitions for the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/index.ts`.
//!
//! Each tool is defined with a name, description, and JSON-schema-style input spec.
//! Descriptions and parameter schemas mirror pi-mono **exactly** — the LLM relies on
//! these strings when deciding how to call each tool.
//!
//! Execution delegates to `agent-core::tools` via `ToolBackend`.

// Sub-modules (one per tool source file, mirroring pi-mono tool files)
pub mod bash;
pub mod edit;
pub mod edit_diff;
pub mod file_mutation_queue;
pub mod find;
pub mod grep;
pub mod ls;
pub mod path_utils;
pub mod plan_mode;
pub mod read;
pub mod render_utils;
pub mod tool_definition_wrapper;
pub mod truncate;
pub mod web_fetch;
pub mod web_search;
pub mod write;

use std::collections::HashMap;

use ai::types::LlmTool;

// ============================================================================
// Tool name enum
// ============================================================================

/// All available tool names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolName {
    Read,
    Bash,
    Edit,
    Write,
    Grep,
    Find,
    Ls,
    WebFetch,
    WebSearch,
}

impl std::str::FromStr for ToolName {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read" => Ok(ToolName::Read),
            "bash" => Ok(ToolName::Bash),
            "edit" => Ok(ToolName::Edit),
            "write" => Ok(ToolName::Write),
            "grep" => Ok(ToolName::Grep),
            "find" => Ok(ToolName::Find),
            "ls" => Ok(ToolName::Ls),
            "web_fetch" => Ok(ToolName::WebFetch),
            "web_search" => Ok(ToolName::WebSearch),
            other => Err(format!("unknown tool: {other}")),
        }
    }
}

impl std::fmt::Display for ToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ToolName::Read => "read",
            ToolName::Bash => "bash",
            ToolName::Edit => "edit",
            ToolName::Write => "write",
            ToolName::Grep => "grep",
            ToolName::Find => "find",
            ToolName::Ls => "ls",
            ToolName::WebFetch => "web_fetch",
            ToolName::WebSearch => "web_search",
        };
        write!(f, "{s}")
    }
}

/// All valid tool name strings.
pub const ALL_TOOL_NAMES: &[&str] = &[
    "read",
    "bash",
    "edit",
    "write",
    "grep",
    "find",
    "ls",
    "web_fetch",
    "web_search",
];

/// Default tools used when no `--tools` flag is provided.
pub const DEFAULT_TOOL_NAMES: &[&str] = &["read", "bash", "edit", "write"];

// ============================================================================
// Tool descriptor
// ============================================================================

/// A lightweight descriptor for a coding-agent tool.
///
/// The `description` and `parameters` fields mirror pi-mono **exactly**:
/// the LLM uses them verbatim when deciding which tool to call and how.
///
/// Execution is handled by `agent-core::tools` via `ToolBackend`.
#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    pub name: ToolName,
    /// LLM-facing description (must match pi-mono exactly).
    pub description: &'static str,
    /// JSON Schema for the tool's input parameters (must match pi-mono exactly).
    pub parameters: serde_json::Value,
    /// Whether the tool modifies files (used to filter read-only presets).
    pub mutating: bool,
}

impl ToolDescriptor {
    /// Convert this descriptor into an `ai::types::LlmTool` for LLM API requests.
    pub fn to_llm_tool(&self) -> LlmTool {
        LlmTool {
            name: self.name.to_string(),
            description: self.description.to_string(),
            parameters: self.parameters.clone(),
        }
    }
}

// ============================================================================
// Per-tool descriptors (description + schema mirror pi-mono exactly)
// ============================================================================

fn bash_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::Bash,
        // Translated from pi-mono bash.ts: createBashToolDefinition description
        description: "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.",
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Bash command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Timeout in seconds (optional, no default timeout)"
                }
            },
            "required": ["command"]
        }),
        mutating: true,
    }
}

fn read_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::Read,
        // Translated from pi-mono read.ts: createReadToolDefinition description
        description: "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.",
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read (relative or absolute)"
                },
                "offset": {
                    "type": "number",
                    "description": "Line number to start reading from (1-indexed)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["path"]
        }),
        mutating: false,
    }
}

fn write_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::Write,
        // Translated from pi-mono write.ts: createWriteToolDefinition description
        description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.",
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write (relative or absolute)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        }),
        mutating: true,
    }
}

fn edit_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::Edit,
        // Translated from pi-mono edit.ts: createEditToolDefinition description
        description: "Edit a file by replacing exact text. The oldText must match exactly (including whitespace). Use this for precise, surgical edits.",
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit (relative or absolute)"
                },
                "oldText": {
                    "type": "string",
                    "description": "Exact text to find and replace (must match exactly)"
                },
                "newText": {
                    "type": "string",
                    "description": "New text to replace the old text with"
                }
            },
            "required": ["path", "oldText", "newText"]
        }),
        mutating: true,
    }
}

fn grep_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::Grep,
        // Translated from pi-mono grep.ts: createGrepToolDefinition description
        description: "Search file contents for a pattern. Returns matching lines with file paths and line numbers. Respects .gitignore. Output is truncated to 100 matches or 50KB (whichever is hit first). Long lines are truncated to 500 chars.",
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Search pattern (regex or literal string)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search (default: current directory)"
                },
                "glob": {
                    "type": "string",
                    "description": "Filter files by glob pattern, e.g. '*.ts' or '**/*.spec.ts'"
                },
                "ignoreCase": {
                    "type": "boolean",
                    "description": "Case-insensitive search (default: false)"
                },
                "literal": {
                    "type": "boolean",
                    "description": "Treat pattern as literal string instead of regex (default: false)"
                },
                "context": {
                    "type": "number",
                    "description": "Number of lines to show before and after each match (default: 0)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of matches to return (default: 100)"
                }
            },
            "required": ["pattern"]
        }),
        mutating: false,
    }
}

fn find_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::Find,
        // Translated from pi-mono find.ts: createFindToolDefinition description
        description: "Search for files by glob pattern. Returns matching file paths relative to the search directory. Respects .gitignore. Output is truncated to 1000 results or 50KB (whichever is hit first).",
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files, e.g. '*.ts', '**/*.json', or 'src/**/*.spec.ts'"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default: current directory)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of results (default: 1000)"
                }
            },
            "required": ["pattern"]
        }),
        mutating: false,
    }
}

fn ls_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::Ls,
        // Translated from pi-mono ls.ts: createLsToolDefinition description
        description: "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. Output is truncated to 500 entries or 50KB (whichever is hit first).",
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory to list (default: current directory)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of entries to return (default: 500)"
                }
            },
            "required": []
        }),
        mutating: false,
    }
}

// ============================================================================
// Public API
// ============================================================================

fn web_fetch_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::WebFetch,
        description: "Fetch content from a URL and return it as plain text. \
                      HTML pages are converted to readable text. \
                      Use this to read documentation, web pages, or any URL-accessible content.",
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                },
                "prompt": {
                    "type": "string",
                    "description": "Optional instruction for how to process the fetched content"
                }
            },
            "required": ["url"]
        }),
        mutating: false,
    }
}

fn web_search_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::WebSearch,
        description: "Search the web for current information. Returns a list of relevant URLs and titles. \
                      Use this when you need up-to-date information that may not be in your training data.",
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query to use"
                },
                "allowed_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only include search results from these domains (optional)"
                },
                "blocked_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Never include search results from these domains (optional)"
                }
            },
            "required": ["query"]
        }),
        mutating: false,
    }
}

/// All built-in tool descriptors, mirroring `allTools` from `tools/index.ts`.
///
/// Descriptions and parameter schemas are exact translations from pi-mono.
pub fn all_tool_descriptors() -> HashMap<ToolName, ToolDescriptor> {
    let tools = vec![
        read_descriptor(),
        bash_descriptor(),
        edit_descriptor(),
        write_descriptor(),
        grep_descriptor(),
        find_descriptor(),
        ls_descriptor(),
        web_fetch_descriptor(),
        web_search_descriptor(),
    ];
    tools.into_iter().map(|t| (t.name.clone(), t)).collect()
}

/// Return `LlmTool` definitions for the given tool names.
///
/// These are the tool definitions sent to the LLM API. Descriptions and
/// parameter schemas mirror pi-mono **exactly**.
pub fn llm_tools_for(names: &[ToolName]) -> Vec<LlmTool> {
    let all = all_tool_descriptors();
    names
        .iter()
        .filter_map(|n| all.get(n).map(|d| d.to_llm_tool()))
        .collect()
}

/// Return `LlmTool` definitions for all tools (mirroring `allToolDefinitions` from pi-mono).
pub fn all_llm_tools() -> Vec<LlmTool> {
    llm_tools_for(&[
        ToolName::Read,
        ToolName::Bash,
        ToolName::Edit,
        ToolName::Write,
        ToolName::Grep,
        ToolName::Find,
        ToolName::Ls,
        ToolName::WebFetch,
        ToolName::WebSearch,
    ])
}

/// `LlmTool` definitions for the default coding tool set (read, bash, edit, write).
///
/// Mirrors pi-mono's `createCodingToolDefinitions`.
pub fn coding_llm_tools() -> Vec<LlmTool> {
    llm_tools_for(&[
        ToolName::Read,
        ToolName::Bash,
        ToolName::Edit,
        ToolName::Write,
    ])
}

/// `LlmTool` definitions for the read-only tool set (read, grep, find, ls).
///
/// Mirrors pi-mono's `createReadOnlyToolDefinitions`.
pub fn read_only_llm_tools() -> Vec<LlmTool> {
    llm_tools_for(&[ToolName::Read, ToolName::Grep, ToolName::Find, ToolName::Ls])
}

/// The default set of coding tools (read, bash, edit, write).
pub fn default_coding_tools() -> Vec<ToolName> {
    vec![
        ToolName::Read,
        ToolName::Bash,
        ToolName::Edit,
        ToolName::Write,
    ]
}

/// Read-only tool set (read, grep, find, ls).
pub fn read_only_tools() -> Vec<ToolName> {
    vec![ToolName::Read, ToolName::Grep, ToolName::Find, ToolName::Ls]
}

/// Resolve a list of tool names from CLI strings, warning on unknown names.
pub fn resolve_tools(names: &[String]) -> (Vec<ToolName>, Vec<String>) {
    let mut tools = Vec::new();
    let mut warnings = Vec::new();
    for name in names {
        match name.parse::<ToolName>() {
            Ok(t) => tools.push(t),
            Err(_) => warnings.push(format!(
                "Unknown tool \"{name}\". Valid tools: {}",
                ALL_TOOL_NAMES.join(", ")
            )),
        }
    }
    (tools, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tool_names_parse() {
        for name in ALL_TOOL_NAMES {
            let t: ToolName = name.parse().expect("should parse");
            assert_eq!(t.to_string(), *name);
        }
    }

    #[test]
    fn unknown_tool_error() {
        let result: Result<ToolName, _> = "unknown".parse();
        assert!(result.is_err());
    }

    #[test]
    fn all_tool_descriptors_count() {
        let descs = all_tool_descriptors();
        assert_eq!(
            descs.len(),
            ALL_TOOL_NAMES.len(),
            "descriptor count must match ALL_TOOL_NAMES"
        );
    }

    #[test]
    fn default_coding_tools() {
        let tools = super::default_coding_tools();
        assert!(tools.contains(&ToolName::Read));
        assert!(tools.contains(&ToolName::Bash));
        assert!(tools.contains(&ToolName::Edit));
        assert!(tools.contains(&ToolName::Write));
        assert!(!tools.contains(&ToolName::Grep));
    }

    #[test]
    fn read_only_tools_no_mutating() {
        let tools = super::read_only_tools();
        let descs = all_tool_descriptors();
        for t in &tools {
            assert!(
                !descs[t].mutating,
                "{t} should not be mutating in read-only set"
            );
        }
    }

    #[test]
    fn resolve_tools_known() {
        let names = vec!["read".to_string(), "bash".to_string()];
        let (tools, warnings) = resolve_tools(&names);
        assert_eq!(tools, vec![ToolName::Read, ToolName::Bash]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn resolve_tools_unknown_warns() {
        let names = vec!["read".to_string(), "blorp".to_string()];
        let (tools, warnings) = resolve_tools(&names);
        assert_eq!(tools, vec![ToolName::Read]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("blorp"));
    }

    #[test]
    fn tool_descriptor_mutating_flags() {
        let descs = all_tool_descriptors();
        assert!(!descs[&ToolName::Read].mutating);
        assert!(descs[&ToolName::Bash].mutating);
        assert!(descs[&ToolName::Edit].mutating);
        assert!(descs[&ToolName::Write].mutating);
        assert!(!descs[&ToolName::Grep].mutating);
        assert!(!descs[&ToolName::Find].mutating);
        assert!(!descs[&ToolName::Ls].mutating);
    }

    // =========================================================================
    // Description accuracy tests — verify pi-mono description strings are used
    // =========================================================================

    #[test]
    fn bash_description_matches_pi_mono() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Bash].description;
        assert!(
            desc.contains("Execute a bash command"),
            "bash description should start with 'Execute a bash command'"
        );
        assert!(
            desc.contains("2000 lines"),
            "bash description should mention 2000 lines"
        );
        assert!(
            desc.contains("50KB"),
            "bash description should mention 50KB"
        );
        assert!(
            desc.contains("timeout"),
            "bash description should mention timeout"
        );
    }

    #[test]
    fn read_description_matches_pi_mono() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Read].description;
        assert!(
            desc.contains("Read the contents of a file"),
            "read description mismatch"
        );
        assert!(
            desc.contains("jpg, png, gif, webp"),
            "read description should list image formats"
        );
        assert!(
            desc.contains("2000 lines"),
            "read description should mention 2000 lines"
        );
        assert!(
            desc.contains("50KB"),
            "read description should mention 50KB"
        );
        assert!(
            desc.contains("offset"),
            "read description should mention offset"
        );
    }

    #[test]
    fn write_description_matches_pi_mono() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Write].description;
        assert!(
            desc.contains("Write content to a file"),
            "write description mismatch"
        );
        assert!(
            desc.contains("parent directories"),
            "write description should mention parent directories"
        );
    }

    #[test]
    fn edit_description_matches_pi_mono() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Edit].description;
        assert!(
            desc.contains("Edit a file by replacing exact text"),
            "edit description mismatch"
        );
        assert!(
            desc.contains("whitespace"),
            "edit description should mention whitespace"
        );
        assert!(
            desc.contains("surgical"),
            "edit description should mention surgical"
        );
    }

    #[test]
    fn grep_description_matches_pi_mono() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Grep].description;
        assert!(
            desc.contains("Search file contents for a pattern"),
            "grep description mismatch"
        );
        assert!(
            desc.contains(".gitignore"),
            "grep description should mention .gitignore"
        );
        assert!(
            desc.contains("100 matches"),
            "grep description should mention 100 matches limit"
        );
        assert!(
            desc.contains("50KB"),
            "grep description should mention 50KB"
        );
        assert!(
            desc.contains("500 chars"),
            "grep description should mention 500 chars line truncation"
        );
    }

    #[test]
    fn find_description_matches_pi_mono() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Find].description;
        assert!(
            desc.contains("Search for files by glob pattern"),
            "find description mismatch"
        );
        assert!(
            desc.contains(".gitignore"),
            "find description should mention .gitignore"
        );
        assert!(
            desc.contains("1000 results"),
            "find description should mention 1000 results"
        );
        assert!(
            desc.contains("50KB"),
            "find description should mention 50KB"
        );
    }

    #[test]
    fn ls_description_matches_pi_mono() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Ls].description;
        assert!(
            desc.contains("List directory contents"),
            "ls description mismatch"
        );
        assert!(
            desc.contains("alphabetically"),
            "ls description should mention alphabetically"
        );
        assert!(
            desc.contains("dotfiles"),
            "ls description should mention dotfiles"
        );
        assert!(
            desc.contains("500 entries"),
            "ls description should mention 500 entries"
        );
        assert!(desc.contains("50KB"), "ls description should mention 50KB");
    }

    // =========================================================================
    // Parameter schema tests
    // =========================================================================

    #[test]
    fn bash_schema_has_command_required() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Bash].parameters;
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
        assert!(schema["properties"]["command"].is_object());
        assert!(schema["properties"]["timeout"].is_object());
    }

    #[test]
    fn read_schema_has_path_required() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Read].parameters;
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
        assert!(schema["properties"]["offset"].is_object());
        assert!(schema["properties"]["limit"].is_object());
    }

    #[test]
    fn edit_schema_has_three_required_fields() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Edit].parameters;
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 3);
        let req_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_names.contains(&"path"));
        assert!(req_names.contains(&"oldText"));
        assert!(req_names.contains(&"newText"));
    }

    #[test]
    fn grep_schema_has_optional_params() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Grep].parameters;
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0].as_str(), Some("pattern"));
        // Optional params must exist
        assert!(schema["properties"]["glob"].is_object());
        assert!(schema["properties"]["ignoreCase"].is_object());
        assert!(schema["properties"]["literal"].is_object());
        assert!(schema["properties"]["context"].is_object());
        assert!(schema["properties"]["limit"].is_object());
    }

    #[test]
    fn ls_schema_has_no_required_fields() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Ls].parameters;
        let required = schema["required"].as_array().unwrap();
        assert!(required.is_empty(), "ls has no required fields");
    }

    // =========================================================================
    // LlmTool conversion tests
    // =========================================================================

    #[test]
    fn to_llm_tool_preserves_name_description_schema() {
        let descs = all_tool_descriptors();
        let llm = descs[&ToolName::Bash].to_llm_tool();
        assert_eq!(llm.name, "bash");
        assert_eq!(llm.description, descs[&ToolName::Bash].description);
        assert!(llm.parameters.is_object());
    }

    #[test]
    fn all_llm_tools_returns_nine() {
        let tools = all_llm_tools();
        assert_eq!(tools.len(), 9);
    }

    #[test]
    fn coding_llm_tools_returns_four() {
        let tools = coding_llm_tools();
        assert_eq!(tools.len(), 4);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"edit"));
        assert!(names.contains(&"write"));
        assert!(!names.contains(&"grep"));
    }

    #[test]
    fn read_only_llm_tools_returns_four() {
        let tools = read_only_llm_tools();
        assert_eq!(tools.len(), 4);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"find"));
        assert!(names.contains(&"ls"));
        assert!(!names.contains(&"bash"));
    }

    // =========================================================================
    // Additional schema / metadata tests (mirrors tools.test.ts)
    // =========================================================================

    /// read has offset (1-indexed line number to start from)
    #[test]
    fn read_schema_offset_is_number() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Read].parameters;
        assert_eq!(
            schema["properties"]["offset"]["type"].as_str(),
            Some("number")
        );
    }

    /// read has limit
    #[test]
    fn read_schema_limit_is_number() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Read].parameters;
        assert_eq!(
            schema["properties"]["limit"]["type"].as_str(),
            Some("number")
        );
    }

    /// read schema path is string
    #[test]
    fn read_schema_path_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Read].parameters;
        assert_eq!(
            schema["properties"]["path"]["type"].as_str(),
            Some("string")
        );
    }

    /// write schema path is string
    #[test]
    fn write_schema_path_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Write].parameters;
        assert_eq!(
            schema["properties"]["path"]["type"].as_str(),
            Some("string")
        );
    }

    /// write schema content is string
    #[test]
    fn write_schema_content_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Write].parameters;
        assert_eq!(
            schema["properties"]["content"]["type"].as_str(),
            Some("string")
        );
    }

    /// edit schema path is string
    #[test]
    fn edit_schema_path_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Edit].parameters;
        assert_eq!(
            schema["properties"]["path"]["type"].as_str(),
            Some("string")
        );
    }

    /// edit schema oldText is string
    #[test]
    fn edit_schema_old_text_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Edit].parameters;
        assert_eq!(
            schema["properties"]["oldText"]["type"].as_str(),
            Some("string")
        );
    }

    /// edit schema newText is string
    #[test]
    fn edit_schema_new_text_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Edit].parameters;
        assert_eq!(
            schema["properties"]["newText"]["type"].as_str(),
            Some("string")
        );
    }

    /// bash schema command is string
    #[test]
    fn bash_schema_command_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Bash].parameters;
        assert_eq!(
            schema["properties"]["command"]["type"].as_str(),
            Some("string")
        );
    }

    /// bash schema timeout is number
    #[test]
    fn bash_schema_timeout_is_number() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Bash].parameters;
        assert_eq!(
            schema["properties"]["timeout"]["type"].as_str(),
            Some("number")
        );
    }

    /// grep schema pattern is string
    #[test]
    fn grep_schema_pattern_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Grep].parameters;
        assert_eq!(
            schema["properties"]["pattern"]["type"].as_str(),
            Some("string")
        );
    }

    /// grep schema ignoreCase is boolean
    #[test]
    fn grep_schema_ignore_case_is_boolean() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Grep].parameters;
        assert_eq!(
            schema["properties"]["ignoreCase"]["type"].as_str(),
            Some("boolean")
        );
    }

    /// grep schema literal is boolean
    #[test]
    fn grep_schema_literal_is_boolean() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Grep].parameters;
        assert_eq!(
            schema["properties"]["literal"]["type"].as_str(),
            Some("boolean")
        );
    }

    /// grep schema context is number
    #[test]
    fn grep_schema_context_is_number() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Grep].parameters;
        assert_eq!(
            schema["properties"]["context"]["type"].as_str(),
            Some("number")
        );
    }

    /// grep schema limit is number
    #[test]
    fn grep_schema_limit_is_number() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Grep].parameters;
        assert_eq!(
            schema["properties"]["limit"]["type"].as_str(),
            Some("number")
        );
    }

    /// grep schema glob is string
    #[test]
    fn grep_schema_glob_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Grep].parameters;
        assert_eq!(
            schema["properties"]["glob"]["type"].as_str(),
            Some("string")
        );
    }

    /// find schema pattern is string
    #[test]
    fn find_schema_pattern_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Find].parameters;
        assert_eq!(
            schema["properties"]["pattern"]["type"].as_str(),
            Some("string")
        );
    }

    /// find schema limit is number
    #[test]
    fn find_schema_limit_is_number() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Find].parameters;
        assert_eq!(
            schema["properties"]["limit"]["type"].as_str(),
            Some("number")
        );
    }

    /// find schema path is string
    #[test]
    fn find_schema_path_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Find].parameters;
        assert_eq!(
            schema["properties"]["path"]["type"].as_str(),
            Some("string")
        );
    }

    /// ls schema limit is number
    #[test]
    fn ls_schema_limit_is_number() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Ls].parameters;
        assert_eq!(
            schema["properties"]["limit"]["type"].as_str(),
            Some("number")
        );
    }

    /// ls schema path is string
    #[test]
    fn ls_schema_path_is_string() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Ls].parameters;
        assert_eq!(
            schema["properties"]["path"]["type"].as_str(),
            Some("string")
        );
    }

    /// all_tool_descriptors maps ToolName → ToolDescriptor correctly
    #[test]
    fn all_descriptors_names_match_keys() {
        let descs = all_tool_descriptors();
        for (key, desc) in &descs {
            assert_eq!(key, &desc.name, "key should match desc.name");
        }
    }

    /// llm_tools_for with empty slice returns empty
    #[test]
    fn llm_tools_for_empty_slice_is_empty() {
        let tools = llm_tools_for(&[]);
        assert!(tools.is_empty());
    }

    /// llm_tools_for with single tool returns one entry
    #[test]
    fn llm_tools_for_single_tool() {
        let tools = llm_tools_for(&[ToolName::Read]);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "read");
    }

    /// resolve_tools deduplicates names
    #[test]
    fn resolve_tools_deduplicates() {
        let names = vec!["read".to_string(), "read".to_string(), "bash".to_string()];
        let (tools, warnings) = resolve_tools(&names);
        // Both are valid → no warnings, but duplicates are included as-is
        assert!(warnings.is_empty());
        // We get 3 results (no automatic dedup in resolve_tools)
        assert_eq!(tools.len(), 3);
    }

    /// ALL_TOOL_NAMES constant contains all 9 tool names
    #[test]
    fn all_tool_names_constant_has_all_tools() {
        assert_eq!(ALL_TOOL_NAMES.len(), 9);
        assert!(ALL_TOOL_NAMES.contains(&"read"));
        assert!(ALL_TOOL_NAMES.contains(&"bash"));
        assert!(ALL_TOOL_NAMES.contains(&"edit"));
        assert!(ALL_TOOL_NAMES.contains(&"write"));
        assert!(ALL_TOOL_NAMES.contains(&"grep"));
        assert!(ALL_TOOL_NAMES.contains(&"find"));
        assert!(ALL_TOOL_NAMES.contains(&"ls"));
        assert!(ALL_TOOL_NAMES.contains(&"web_fetch"));
        assert!(ALL_TOOL_NAMES.contains(&"web_search"));
    }

    /// DEFAULT_TOOL_NAMES only includes the 4 coding tools
    #[test]
    fn default_tool_names_has_four_tools() {
        assert_eq!(DEFAULT_TOOL_NAMES.len(), 4);
        assert!(DEFAULT_TOOL_NAMES.contains(&"read"));
        assert!(DEFAULT_TOOL_NAMES.contains(&"bash"));
        assert!(DEFAULT_TOOL_NAMES.contains(&"edit"));
        assert!(DEFAULT_TOOL_NAMES.contains(&"write"));
        assert!(!DEFAULT_TOOL_NAMES.contains(&"grep"));
        assert!(!DEFAULT_TOOL_NAMES.contains(&"find"));
        assert!(!DEFAULT_TOOL_NAMES.contains(&"ls"));
    }

    /// read_only_tools: grep, find, ls are non-mutating
    #[test]
    fn read_only_tools_are_non_mutating() {
        let tools = read_only_tools();
        assert_eq!(tools.len(), 4);
        let descs = all_tool_descriptors();
        for name in &tools {
            assert!(!descs[name].mutating, "{name} should not be mutating");
        }
    }

    /// default_coding_tools: bash and edit are mutating
    #[test]
    fn default_coding_tools_include_mutating() {
        let tools = super::default_coding_tools();
        let descs = all_tool_descriptors();
        let bash = descs.get(&ToolName::Bash).unwrap();
        let edit = descs.get(&ToolName::Edit).unwrap();
        assert!(bash.mutating);
        assert!(edit.mutating);
        assert!(tools.contains(&ToolName::Bash));
        assert!(tools.contains(&ToolName::Edit));
    }

    /// ToolName Display impl covers all variants
    #[test]
    fn tool_name_display_all_variants() {
        assert_eq!(ToolName::Read.to_string(), "read");
        assert_eq!(ToolName::Bash.to_string(), "bash");
        assert_eq!(ToolName::Edit.to_string(), "edit");
        assert_eq!(ToolName::Write.to_string(), "write");
        assert_eq!(ToolName::Grep.to_string(), "grep");
        assert_eq!(ToolName::Find.to_string(), "find");
        assert_eq!(ToolName::Ls.to_string(), "ls");
        assert_eq!(ToolName::WebFetch.to_string(), "web_fetch");
        assert_eq!(ToolName::WebSearch.to_string(), "web_search");
    }

    /// LlmTool parameters from to_llm_tool have correct type field
    #[test]
    fn llm_tool_parameters_have_type_object() {
        let descs = all_tool_descriptors();
        for desc in descs.values() {
            let llm = desc.to_llm_tool();
            assert_eq!(
                llm.parameters["type"].as_str(),
                Some("object"),
                "{} parameters should have type=object",
                llm.name
            );
        }
    }

    /// resolve_tools with all known names has no warnings
    #[test]
    fn resolve_tools_all_known_no_warnings() {
        let names: Vec<String> = ALL_TOOL_NAMES.iter().map(|s| s.to_string()).collect();
        let (tools, warnings) = resolve_tools(&names);
        assert_eq!(tools.len(), ALL_TOOL_NAMES.len());
        assert!(warnings.is_empty());
    }

    /// resolve_tools unknown tool warning mentions the name
    #[test]
    fn resolve_tools_unknown_warning_mentions_name() {
        let names = vec!["nonexistent-tool".to_string()];
        let (tools, warnings) = resolve_tools(&names);
        assert!(tools.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("nonexistent-tool"));
        assert!(warnings[0].contains("Valid tools:"));
    }
}

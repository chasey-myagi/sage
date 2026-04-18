// End-to-end contract test for every built-in tool × workspace_root routing.
//
// Purpose: catch "tool ignores workspace_root" regressions ONE layer above
// the backend unit tests — here we build the tool via the same
// `create_tool` factory the engine uses, invoke it with a RELATIVE path
// that only makes sense under an agent's workspace, and assert the tool
// returns the expected content.
//
// If a future change accidentally constructs a tool with `LocalBackend::
// new()` (the legacy constructor) instead of `with_workspace`, the
// relative-path assertion here will fail.

use sage_runtime::tools::backend::LocalBackend;
use sage_runtime::tools::{create_tool, AgentTool, ToolOutput};
use serde_json::{json, Value};
use tempfile::TempDir;

/// Build a minimal agent workspace with known content and return a
/// LocalBackend rooted at it. Every test below starts from this shape:
///
///   <tempdir>/
///     skills/
///       INDEX.md             "# Skills Index\n- demo ..."
///       demo/
///         SKILL.md            "<demo-skill-frontmatter>\nhello agent"
///         list-demo.sh        (+x) echoes "demo-script-ok"
///     memory/
///       MEMORY.md             "# memory\ndetail: quiz answer 42"
fn build_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let skills = root.join("skills");
    std::fs::create_dir_all(&skills).unwrap();
    std::fs::write(
        skills.join("INDEX.md"),
        "# Skills Index\n- demo — demonstrates the workspace_root contract.\n",
    )
    .unwrap();

    let demo = skills.join("demo");
    std::fs::create_dir_all(&demo).unwrap();
    std::fs::write(
        demo.join("SKILL.md"),
        "---\nname: demo\ntype: prompt\n---\n\nhello agent\n",
    )
    .unwrap();

    let script = demo.join("list-demo.sh");
    std::fs::write(&script, "#!/bin/bash\necho demo-script-ok\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&script).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&script, p).unwrap();
    }

    let mem = root.join("memory");
    std::fs::create_dir_all(&mem).unwrap();
    std::fs::write(mem.join("MEMORY.md"), "# memory\ndetail: quiz answer 42\n").unwrap();

    tmp
}

fn tool(name: &str, root: &std::path::Path) -> Box<dyn AgentTool> {
    let backend = LocalBackend::with_workspace(root.to_path_buf());
    create_tool(name, backend).unwrap_or_else(|| panic!("tool {name} not found"))
}

fn text_of(out: &ToolOutput) -> &str {
    match &out.content[0] {
        sage_runtime::types::Content::Text { text } => text.as_str(),
        _ => panic!("expected Content::Text"),
    }
}

// ---------------------------------------------------------------------------
// read — relative path resolves against workspace_root
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_tool_reads_skill_md_via_relative_path() {
    let ws = build_workspace();
    let t = tool("read", ws.path());
    let out = t.execute(json!({"file_path": "skills/demo/SKILL.md"})).await;
    assert!(!out.is_error, "read failed: {}", text_of(&out));
    let body = text_of(&out);
    assert!(body.contains("hello agent"), "body missing, got: {body}");
}

#[tokio::test]
async fn read_tool_reads_index_md_via_relative_path() {
    // The canonical "first step" the SAGE_CORE_PROMPT tells the model to
    // do — it MUST work end-to-end against any agent's workspace root.
    let ws = build_workspace();
    let t = tool("read", ws.path());
    let out = t.execute(json!({"file_path": "skills/INDEX.md"})).await;
    assert!(!out.is_error);
    assert!(text_of(&out).contains("Skills Index"));
}

#[tokio::test]
async fn read_tool_absolute_path_still_works() {
    let ws = build_workspace();
    let t = tool("read", ws.path());
    let abs = ws.path().join("memory").join("MEMORY.md");
    let out = t.execute(json!({"file_path": abs.to_str().unwrap()})).await;
    assert!(!out.is_error);
    assert!(text_of(&out).contains("quiz answer 42"));
}

// ---------------------------------------------------------------------------
// write — relative path writes under workspace_root
// ---------------------------------------------------------------------------

#[tokio::test]
async fn write_tool_lands_file_under_workspace_root() {
    let ws = build_workspace();
    let t = tool("write", ws.path());
    let out = t
        .execute(json!({
            "file_path": "skills/demo/notes.md",
            "content": "captured during session"
        }))
        .await;
    assert!(!out.is_error, "write failed: {}", text_of(&out));
    let expected = ws.path().join("skills/demo/notes.md");
    assert!(expected.exists(), "write did not land under workspace root");
    let bytes = std::fs::read(&expected).unwrap();
    assert_eq!(bytes, b"captured during session");
}

// ---------------------------------------------------------------------------
// edit — modify existing file via relative path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edit_tool_modifies_skill_md_in_place() {
    let ws = build_workspace();
    let t = tool("edit", ws.path());
    let out = t
        .execute(json!({
            "file_path": "skills/demo/SKILL.md",
            "old_string": "hello agent",
            "new_string": "rewritten content"
        }))
        .await;
    assert!(!out.is_error, "edit failed: {}", text_of(&out));
    let updated = std::fs::read_to_string(ws.path().join("skills/demo/SKILL.md")).unwrap();
    assert!(updated.contains("rewritten content"));
    assert!(!updated.contains("hello agent"));
}

// ---------------------------------------------------------------------------
// ls — directory listing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ls_tool_lists_skills_directory_by_relative_path() {
    let ws = build_workspace();
    let t = tool("ls", ws.path());
    let out = t.execute(json!({"path": "skills"})).await;
    assert!(!out.is_error, "ls failed: {}", text_of(&out));
    let listing = text_of(&out);
    assert!(listing.contains("INDEX.md"), "missing INDEX.md in: {listing}");
    assert!(listing.contains("demo"), "missing demo/ in: {listing}");
}

#[tokio::test]
async fn ls_tool_accepts_dot_for_root() {
    let ws = build_workspace();
    let t = tool("ls", ws.path());
    let out = t.execute(json!({"path": "."})).await;
    assert!(!out.is_error);
    let listing = text_of(&out);
    assert!(listing.contains("skills"));
    assert!(listing.contains("memory"));
}

// ---------------------------------------------------------------------------
// grep — rg with cwd at workspace_root
// ---------------------------------------------------------------------------

#[tokio::test]
async fn grep_tool_finds_content_scoped_to_workspace() {
    let ws = build_workspace();
    let t = tool("grep", ws.path());
    // Pattern is in memory/MEMORY.md only.
    let out = t.execute(json!({"pattern": "quiz answer"})).await;
    // rg may not be installed; treat its absence as a skipped assertion
    // rather than a failure so the E2E suite stays green on minimal
    // dev machines. The workspace_root contract is the thing under test.
    if out.is_error && text_of(&out).contains("ripgrep") {
        eprintln!("rg not installed — skipping grep E2E");
        return;
    }
    assert!(!out.is_error, "grep failed: {}", text_of(&out));
    let result = text_of(&out);
    assert!(
        result.contains("quiz answer") || result.contains("MEMORY.md"),
        "grep result should reference the match, got: {result}"
    );
}

// ---------------------------------------------------------------------------
// find — glob with default "." base
// ---------------------------------------------------------------------------

#[tokio::test]
async fn find_tool_locates_skill_md_under_workspace() {
    let ws = build_workspace();
    let t = tool("find", ws.path());
    let out = t
        .execute(json!({
            "pattern": "*.md",
            "path": "skills",
            "depth": 3
        }))
        .await;
    assert!(!out.is_error, "find failed: {}", text_of(&out));
    let listing = text_of(&out);
    assert!(
        listing.contains("INDEX.md") && listing.contains("SKILL.md"),
        "find should see both .md files, got: {listing}"
    );
}

// ---------------------------------------------------------------------------
// bash — cwd must be workspace_root
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bash_tool_runs_relative_script_under_workspace_cwd() {
    let ws = build_workspace();
    let t = tool("bash", ws.path());
    // Model-style: call the script by its workspace-relative path.
    let out = t
        .execute(json!({"command": "./skills/demo/list-demo.sh"}))
        .await;
    assert!(!out.is_error, "bash failed: {}", text_of(&out));
    let body = text_of(&out);
    assert!(
        body.contains("demo-script-ok"),
        "script stdout missing, got: {body}"
    );
}

#[tokio::test]
async fn bash_tool_sees_workspace_as_cwd_via_pwd() {
    let ws = build_workspace();
    let t = tool("bash", ws.path());
    let out = t.execute(json!({"command": "pwd"})).await;
    assert!(!out.is_error);
    // macOS symlinks /var → /private/var; just check the tempdir basename
    // appears somewhere in pwd output.
    let basename = ws.path().file_name().unwrap().to_string_lossy();
    let body = text_of(&out);
    assert!(
        body.contains(basename.as_ref()),
        "pwd output should include workspace basename '{basename}', got: {body}"
    );
}

// ---------------------------------------------------------------------------
// Cross-tool: write → read round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn write_then_read_round_trip_via_workspace() {
    let ws = build_workspace();
    let w = tool("write", ws.path());
    let r = tool("read", ws.path());
    let _ = w
        .execute(json!({
            "file_path": "skills/demo/fresh.md",
            "content": "fresh body"
        }))
        .await;
    let out = r
        .execute(json!({"file_path": "skills/demo/fresh.md"}))
        .await;
    assert!(!out.is_error, "read after write failed: {}", text_of(&out));
    assert!(text_of(&out).contains("fresh body"));
}

// ---------------------------------------------------------------------------
// Environment contract: skill install pattern
// ---------------------------------------------------------------------------
//
// The model-facing contract is "first read skills/INDEX.md" (no
// `workspace/` prefix — workspace_root already IS the workspace dir).
// This test reproduces the exact tool call a compliant model would
// issue on turn 1 and asserts:
//   (a) the correct path resolves
//   (b) the doubled-prefix path does NOT resolve, guarding against a
//       regression that re-adds the prefix to SAGE_CORE_PROMPT.

#[tokio::test]
async fn canonical_first_turn_read_index_md_must_succeed() {
    let ws = build_workspace();
    let t = tool("read", ws.path());

    let out = t.execute(json!({"file_path": "skills/INDEX.md"})).await;
    assert!(
        !out.is_error,
        "'skills/INDEX.md' MUST succeed — this is the path SAGE_CORE_PROMPT \
         and AGENT.md direct the model at"
    );

    let out2 = t
        .execute(json!({"file_path": "workspace/skills/INDEX.md"}))
        .await;
    assert!(
        out2.is_error,
        "'workspace/skills/INDEX.md' must fail — workspace_root already \
         IS the workspace dir; a `workspace/` prefix in prompts would \
         resolve to <root>/workspace/skills/INDEX.md"
    );
}

fn _unused_assertion_hint(_: Value) {} // silence unused warning import

// skills.rs — Skill frontmatter parsing + lazy body loading.
//
// Sprint 5 S5.2: replace the old "dump whole Markdown into system prompt" scheme
// with two steps:
//
//  1. At chat startup: `scan_skills` parses only the YAML frontmatter of each
//     `*.md` file under `~/.sage/skills/` (global) and `<agent>/workspace/skills/`
//     (workspace). `render_skill_index` turns the metadata into a compact index
//     block that sits in the system prompt.
//  2. When the user types `/skill-name ...`: `load_skill_body` reads the file
//     again and returns only the body (post-frontmatter) text, which the caller
//     then substitutes `$ARGUMENTS` into and feeds to the LLM as a user message.
//
// This keeps the system prompt small (O(#skills) index lines instead of
// O(sum of file sizes)) while still giving the model awareness of every skill.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Parsed metadata for a single skill Markdown file.
///
/// Produced by [`parse_skill_file`] and [`scan_skills`]. The body text is
/// *not* stored here — [`load_skill_body`] reads it lazily from `body_path`
/// only when the skill is actually invoked.
#[derive(Debug, Clone)]
pub struct SkillMeta {
    /// Skill name — either the `name:` frontmatter field or the file stem.
    pub name: String,
    /// Short human-readable description; shown in the skill index.
    pub description: String,
    /// Optional free-form guidance on *when* the LLM should invoke this skill.
    pub when_to_use: Option<String>,
    /// Optional allow-list of tool names this skill is permitted to call.
    pub allowed_tools: Vec<String>,
    /// If `Some(agent)`, the skill is only visible to that agent.
    pub agent: Option<String>,
    /// Optional hook names to run around this skill's invocation.
    pub hooks: Option<Vec<String>>,
    /// Ranking score; higher-scored skills float up in the index.
    pub score: f32,
    /// Author-bumped version number for cache invalidation.
    pub version: u32,
    /// Absolute path to the skill file — used by [`load_skill_body`].
    pub body_path: PathBuf,
}

/// Parse a skill Markdown file that optionally starts with a YAML frontmatter
/// block delimited by `---` on its own line at the top and before the body.
///
/// Returns `None` only if the file cannot be read (e.g. doesn't exist). A
/// malformed or missing frontmatter is *not* an error — the function falls
/// back to filename-derived defaults, so the caller still gets a [`SkillMeta`].
///
/// Frontmatter-failure policy: if the frontmatter YAML is syntactically broken
/// *or* any field has a type mismatch (e.g. `score: "bad"`), the entire
/// frontmatter is discarded and every field falls back to defaults. Partial
/// field-level rescue is intentionally not attempted — it would hide author
/// errors behind silent type coercion.
pub async fn parse_skill_file(path: &Path) -> Option<SkillMeta> {
    let content = fs::read_to_string(path).await.ok()?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let (fm_text, body) = split_frontmatter(&content);
    let fm_fields = match fm_text {
        Some(t) => match parse_frontmatter_fields(t) {
            Ok(f) => Some(f),
            Err(()) => {
                // Visibility over silence: a skill author who mistypes YAML
                // won't see their skill in the index. One warn line gives
                // them a thread to pull on.
                tracing::warn!(
                    path = %path.display(),
                    "frontmatter rejected (malformed YAML or field type mismatch); falling back to filename + body description"
                );
                None
            }
        },
        None => None,
    };

    if let Some(fields) = fm_fields {
        let description = match fields.description {
            Some(d) => d,
            None => first_paragraph_description(body),
        };
        Some(SkillMeta {
            name: fields.name.unwrap_or(stem),
            description,
            when_to_use: fields.when_to_use,
            allowed_tools: fields.allowed_tools.unwrap_or_default(),
            agent: fields.agent,
            hooks: fields.hooks,
            score: fields.score.unwrap_or(1.0),
            version: fields.version.unwrap_or(1),
            body_path: path.to_path_buf(),
        })
    } else {
        // Frontmatter was rejected (missing, unterminated, or type error).
        // If the header WAS present, `body` already skips past it so the
        // description fallback comes from post-header text; otherwise use the
        // whole content. Reuse fm_text — don't re-parse.
        let desc_source = if fm_text.is_some() { body } else { content.as_str() };
        Some(SkillMeta {
            name: stem,
            description: first_paragraph_description(desc_source),
            when_to_use: None,
            allowed_tools: vec![],
            agent: None,
            hooks: None,
            score: 1.0,
            version: 1,
            body_path: path.to_path_buf(),
        })
    }
}

/// Scan `global_dir` and `workspace_dir` for top-level `*.md` skill files,
/// filter by `current_agent`, and merge with workspace-wins semantics.
pub async fn scan_skills(
    global_dir: &Path,
    workspace_dir: &Path,
    current_agent: &str,
) -> Vec<SkillMeta> {
    let global = scan_dir(global_dir).await;
    let workspace = scan_dir(workspace_dir).await;

    let mut merged: Vec<SkillMeta> = global;
    for ws in workspace {
        merged.retain(|g| g.name != ws.name);
        merged.push(ws);
    }

    merged
        .into_iter()
        .filter(|s| match &s.agent {
            None => true,
            // Strict: empty current_agent only matches `agent = None`,
            // never `agent = Some("")`.
            Some(_) if current_agent.is_empty() => false,
            Some(a) => a == current_agent,
        })
        .collect()
}

/// Render a compact skill index block for the system prompt.
pub fn render_skill_index(skills: &[SkillMeta]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut out = String::from("--- AVAILABLE SKILLS ---\n");
    for s in skills {
        let desc = collapse_newlines(&s.description);
        out.push_str(&format!("- {}: {}\n", s.name, desc));
        if let Some(w) = &s.when_to_use {
            let w = collapse_newlines(w);
            out.push_str(&format!("  When to use: {w}\n"));
        }
    }
    out.push_str("--- END SKILLS ---\n");
    out
}

/// Load the body (post-frontmatter) text of a named skill.
pub async fn load_skill_body(
    name: &str,
    global_dir: &Path,
    workspace_dir: &Path,
) -> Option<String> {
    let candidates = [
        workspace_dir.join(format!("{name}.md")),
        global_dir.join(format!("{name}.md")),
    ];
    for candidate in candidates {
        if let Ok(content) = fs::read_to_string(&candidate).await {
            let (fm_text, body) = split_frontmatter(&content);
            let body_text = if fm_text.is_some() {
                strip_leading_blank_line(body).to_string()
            } else {
                content
            };
            return Some(body_text);
        }
    }
    None
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Split a file into (frontmatter_text, body). Returns `(None, whole)` if the
/// file doesn't have a proper `---\n...\n---\n` header.
fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    // Must start with `---` followed by a newline.
    let rest = if let Some(r) = content.strip_prefix("---\n") {
        r
    } else if let Some(r) = content.strip_prefix("---\r\n") {
        r
    } else {
        return (None, content);
    };

    // Find a line that is exactly `---` (with optional \r).
    let mut idx = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed == "---" {
            let fm = &rest[..idx];
            let body_start = idx + line.len();
            return (Some(fm), &rest[body_start..]);
        }
        idx += line.len();
    }
    // Last line without trailing newline.
    // (Already covered by split_inclusive which yields the final chunk too.)
    (None, content)
}

/// Parsed frontmatter fields. `None` means the field was absent; the whole
/// struct is returned as `None` if any field has a type mismatch.
struct FrontmatterFields {
    name: Option<String>,
    description: Option<String>,
    when_to_use: Option<String>,
    allowed_tools: Option<Vec<String>>,
    agent: Option<String>,
    hooks: Option<Vec<String>>,
    score: Option<f32>,
    version: Option<u32>,
}

fn parse_frontmatter_fields(text: &str) -> Result<FrontmatterFields, ()> {
    let map: HashMap<String, serde_yaml::Value> = serde_yaml::from_str(text).map_err(|_| ())?;

    Ok(FrontmatterFields {
        name: extract_string(&map, "name")?,
        description: extract_string(&map, "description")?,
        when_to_use: extract_string(&map, "when_to_use")?,
        allowed_tools: extract_string_vec(&map, "allowed_tools")?,
        agent: extract_string(&map, "agent")?,
        hooks: extract_string_vec(&map, "hooks")?,
        score: extract_f32(&map, "score")?,
        version: extract_u32(&map, "version")?,
    })
}

// extract_* helpers: Ok(Some(..)) = present & valid, Ok(None) = absent,
// Err(()) = present but wrong type. Caller propagates Err with `?` so any type
// mismatch rejects the whole frontmatter.

fn extract_string(
    map: &HashMap<String, serde_yaml::Value>,
    key: &str,
) -> Result<Option<String>, ()> {
    match map.get(key) {
        None => Ok(None),
        Some(serde_yaml::Value::String(s)) => Ok(Some(s.clone())),
        _ => Err(()),
    }
}

fn extract_string_vec(
    map: &HashMap<String, serde_yaml::Value>,
    key: &str,
) -> Result<Option<Vec<String>>, ()> {
    match map.get(key) {
        None => Ok(None),
        Some(serde_yaml::Value::Sequence(seq)) => {
            let mut out = Vec::with_capacity(seq.len());
            for v in seq {
                let serde_yaml::Value::String(s) = v else {
                    return Err(());
                };
                out.push(s.clone());
            }
            Ok(Some(out))
        }
        _ => Err(()),
    }
}

fn extract_f32(
    map: &HashMap<String, serde_yaml::Value>,
    key: &str,
) -> Result<Option<f32>, ()> {
    match map.get(key) {
        None => Ok(None),
        Some(serde_yaml::Value::Number(n)) => {
            let f = n.as_f64().ok_or(())?;
            // Reject NaN / ±Inf so downstream score-based sort/compare stays
            // well-defined. YAML happily accepts `.nan` / `.inf`; we don't.
            if !f.is_finite() {
                return Err(());
            }
            Ok(Some(f as f32))
        }
        _ => Err(()),
    }
}

fn extract_u32(
    map: &HashMap<String, serde_yaml::Value>,
    key: &str,
) -> Result<Option<u32>, ()> {
    match map.get(key) {
        None => Ok(None),
        Some(serde_yaml::Value::Number(n)) => {
            let i = n.as_u64().ok_or(())?;
            if i > u32::MAX as u64 {
                Err(())
            } else {
                Ok(Some(i as u32))
            }
        }
        _ => Err(()),
    }
}

/// First non-empty line/paragraph, Unicode-safe truncated to ≤ 120 chars.
fn first_paragraph_description(text: &str) -> String {
    let first = text
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string();
    first.chars().take(120).collect()
}

fn collapse_newlines(s: &str) -> String {
    s.replace("\r\n", " ").replace('\n', " ")
}

fn strip_leading_blank_line(body: &str) -> &str {
    if let Some(rest) = body.strip_prefix("\r\n") {
        rest
    } else if let Some(rest) = body.strip_prefix('\n') {
        rest
    } else {
        body
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use tokio::fs;

    // ── helpers ──────────────────────────────────────────────────────────────

    async fn write_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.unwrap();
        }
        fs::write(&path, content).await.unwrap();
        path
    }

    /// Produce a `SkillMeta` with only the fields the caller cares about —
    /// everything else is defaulted. Used by the `render_skill_index` tests.
    fn meta(name: &str, description: &str, when_to_use: Option<&str>) -> SkillMeta {
        SkillMeta {
            name: name.to_string(),
            description: description.to_string(),
            when_to_use: when_to_use.map(|s| s.to_string()),
            allowed_tools: vec![],
            agent: None,
            hooks: None,
            score: 1.0,
            version: 1,
            body_path: PathBuf::from(format!("/tmp/{name}.md")),
        }
    }

    // =============================================================
    // parse_skill_file
    // =============================================================

    #[tokio::test]
    async fn parse_full_frontmatter_populates_all_fields() {
        let tmp = TempDir::new().unwrap();
        let body = "\
---
name: weather-report
description: Check the weather in any city.
when_to_use: When user asks about weather or climate.
allowed_tools:
  - bash
  - http
agent: feishu
hooks:
  - pre-weather
  - post-weather
score: 2.5
version: 3
---
Body content goes here.
";
        let path = write_file(tmp.path(), "weather-report.md", body).await;
        let meta = parse_skill_file(&path).await.expect("should parse");

        assert_eq!(meta.name, "weather-report");
        assert_eq!(meta.description, "Check the weather in any city.");
        assert_eq!(meta.when_to_use.as_deref(), Some("When user asks about weather or climate."));
        assert_eq!(meta.allowed_tools, vec!["bash".to_string(), "http".to_string()]);
        assert_eq!(meta.agent.as_deref(), Some("feishu"));
        assert_eq!(
            meta.hooks.as_ref().map(|v| v.as_slice()),
            Some(&["pre-weather".to_string(), "post-weather".to_string()][..]),
        );
        assert!((meta.score - 2.5).abs() < f32::EPSILON);
        assert_eq!(meta.version, 3);
        assert_eq!(meta.body_path, path);
    }

    #[tokio::test]
    async fn parse_minimal_frontmatter_fills_defaults() {
        let tmp = TempDir::new().unwrap();
        let body = "\
---
name: minimal
description: A tiny skill.
---
body
";
        let path = write_file(tmp.path(), "minimal.md", body).await;
        let meta = parse_skill_file(&path).await.unwrap();

        assert_eq!(meta.name, "minimal");
        assert_eq!(meta.description, "A tiny skill.");
        assert!(meta.when_to_use.is_none());
        assert!(meta.allowed_tools.is_empty());
        assert!(meta.agent.is_none());
        assert!(meta.hooks.is_none());
        assert!((meta.score - 1.0).abs() < f32::EPSILON);
        assert_eq!(meta.version, 1);
    }

    #[tokio::test]
    async fn parse_no_frontmatter_uses_stem_and_first_paragraph() {
        let tmp = TempDir::new().unwrap();
        let body = "This is the first paragraph.\n\nSecond paragraph should be ignored.\n";
        let path = write_file(tmp.path(), "plain-skill.md", body).await;
        let meta = parse_skill_file(&path).await.unwrap();

        assert_eq!(meta.name, "plain-skill");
        assert_eq!(meta.description, "This is the first paragraph.");
        assert!(meta.when_to_use.is_none());
        assert!(meta.allowed_tools.is_empty());
        assert!(meta.agent.is_none());
        assert!(meta.hooks.is_none());
        assert!((meta.score - 1.0).abs() < f32::EPSILON);
        assert_eq!(meta.version, 1);
    }

    #[tokio::test]
    async fn parse_no_frontmatter_empty_file_yields_empty_description() {
        let tmp = TempDir::new().unwrap();
        let path = write_file(tmp.path(), "empty.md", "").await;
        let meta = parse_skill_file(&path).await.unwrap();

        assert_eq!(meta.name, "empty");
        assert_eq!(meta.description, "");
    }

    #[tokio::test]
    async fn parse_no_frontmatter_long_line_truncates_to_120_ascii_chars() {
        let tmp = TempDir::new().unwrap();
        // 200 ASCII chars on a single line.
        let long = "a".repeat(200);
        let path = write_file(tmp.path(), "long.md", &long).await;
        let meta = parse_skill_file(&path).await.unwrap();

        assert_eq!(
            meta.description.chars().count(),
            120,
            "description must be truncated to 120 chars, got {} chars",
            meta.description.chars().count(),
        );
    }

    #[tokio::test]
    async fn parse_no_frontmatter_truncation_respects_unicode_boundaries() {
        let tmp = TempDir::new().unwrap();
        // 200 CJK chars — each is multi-byte in UTF-8. Truncation must count
        // characters, not bytes, and must never split a codepoint.
        let long: String = std::iter::repeat('中').take(200).collect();
        let path = write_file(tmp.path(), "cjk.md", &long).await;
        let meta = parse_skill_file(&path).await.unwrap();

        assert_eq!(meta.description.chars().count(), 120);
        // All kept chars must be the CJK character, never a partial byte.
        assert!(meta.description.chars().all(|c| c == '中'));
    }

    #[tokio::test]
    async fn parse_unclosed_frontmatter_falls_back_to_defaults() {
        let tmp = TempDir::new().unwrap();
        // Opens with `---` but never closes it — whole file must be treated
        // as body, frontmatter ignored.
        let body = "\
---
name: should-not-be-used
description: ignored
this never closes
";
        let path = write_file(tmp.path(), "unclosed.md", body).await;
        let meta = parse_skill_file(&path).await.unwrap();

        // Name falls back to file stem, not the `name:` inside the unclosed block.
        assert_eq!(meta.name, "unclosed");
        assert_ne!(meta.description, "ignored");
    }

    #[tokio::test]
    async fn parse_malformed_yaml_falls_back_without_panic() {
        let tmp = TempDir::new().unwrap();
        // Syntactically broken YAML between the delimiters.
        let body = "\
---
name: bad
description: [unclosed
  sequence: : :
---
body text
";
        let path = write_file(tmp.path(), "broken.md", body).await;
        // Must not panic and must return Some(_) with stem-derived name.
        let meta = parse_skill_file(&path).await.unwrap();
        assert_eq!(meta.name, "broken");
    }

    #[tokio::test]
    async fn parse_field_type_mismatch_rejects_entire_frontmatter() {
        let tmp = TempDir::new().unwrap();
        // `score` is declared f32 but supplied as a string — policy is
        // "whole frontmatter invalid → full fallback to defaults". We assert
        // that by checking the *name* also falls back (i.e. the `name:` field
        // from the frontmatter is NOT rescued).
        let body = "\
---
name: typed-wrong
description: should also be dropped
score: \"bad\"
---
Body first line.
";
        let path = write_file(tmp.path(), "typed-wrong.md", body).await;
        let meta = parse_skill_file(&path).await.unwrap();

        assert_eq!(meta.name, "typed-wrong", "name falls back to stem");
        // Description must come from the body (fallback), not from frontmatter.
        assert_eq!(meta.description, "Body first line.");
        assert!((meta.score - 1.0).abs() < f32::EPSILON);
        assert_eq!(meta.version, 1);
    }

    #[tokio::test]
    async fn parse_missing_file_returns_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.md");
        assert!(parse_skill_file(&path).await.is_none());
    }

    #[tokio::test]
    async fn parse_frontmatter_without_description_falls_back_to_first_line() {
        let tmp = TempDir::new().unwrap();
        let body = "\
---
name: no-desc
score: 1.5
---
First body line becomes description.

Second paragraph ignored.
";
        let path = write_file(tmp.path(), "no-desc.md", body).await;
        let meta = parse_skill_file(&path).await.unwrap();

        assert_eq!(meta.name, "no-desc");
        assert_eq!(meta.description, "First body line becomes description.");
        // Other frontmatter fields that DID parse cleanly should still be honoured.
        assert!((meta.score - 1.5).abs() < f32::EPSILON);
    }

    // =============================================================
    // scan_skills
    // =============================================================

    #[tokio::test]
    async fn scan_merges_global_and_workspace_skills() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(global.path(), "g1.md", "---\nname: g1\ndescription: d\n---\nbody\n").await;
        write_file(global.path(), "g2.md", "---\nname: g2\ndescription: d\n---\nbody\n").await;
        write_file(workspace.path(), "w1.md", "---\nname: w1\ndescription: d\n---\nbody\n").await;
        write_file(workspace.path(), "w2.md", "---\nname: w2\ndescription: d\n---\nbody\n").await;

        let skills = scan_skills(global.path(), workspace.path(), "feishu").await;
        assert_eq!(skills.len(), 4, "should merge all four skills");

        let names: Vec<_> = skills.iter().map(|s| s.name.as_str()).collect();
        for expected in ["g1", "g2", "w1", "w2"] {
            assert!(names.contains(&expected), "missing skill: {expected}");
        }
    }

    #[tokio::test]
    async fn scan_workspace_shadows_global_on_name_collision() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(
            global.path(),
            "shared.md",
            "---\nname: shared\ndescription: global version\n---\nGLOBAL BODY\n",
        )
        .await;
        let workspace_path = write_file(
            workspace.path(),
            "shared.md",
            "---\nname: shared\ndescription: workspace version\n---\nWORKSPACE BODY\n",
        )
        .await;

        let skills = scan_skills(global.path(), workspace.path(), "").await;
        let shared: Vec<_> = skills.iter().filter(|s| s.name == "shared").collect();
        assert_eq!(shared.len(), 1, "dedup must keep exactly one 'shared' entry");

        let kept = shared[0];
        assert_eq!(kept.description, "workspace version");
        assert_eq!(kept.body_path, workspace_path);
    }

    #[tokio::test]
    async fn scan_filters_by_current_agent() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(
            global.path(),
            "anyone.md",
            "---\nname: anyone\ndescription: d\n---\nbody\n",
        )
        .await;
        write_file(
            global.path(),
            "feishu-only.md",
            "---\nname: feishu-only\ndescription: d\nagent: feishu\n---\nbody\n",
        )
        .await;
        write_file(
            global.path(),
            "coder-only.md",
            "---\nname: coder-only\ndescription: d\nagent: coder\n---\nbody\n",
        )
        .await;

        let skills = scan_skills(global.path(), workspace.path(), "feishu").await;
        let names: Vec<_> = skills.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"anyone"), "agent=None must survive");
        assert!(names.contains(&"feishu-only"), "agent=feishu must survive for feishu");
        assert!(!names.contains(&"coder-only"), "agent=coder must be filtered out");
        assert_eq!(skills.len(), 2);
    }

    #[tokio::test]
    async fn scan_empty_current_agent_strictly_matches_only_none() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(
            global.path(),
            "nullagent.md",
            "---\nname: nullagent\ndescription: d\n---\nbody\n",
        )
        .await;
        write_file(
            global.path(),
            "empty-str-agent.md",
            "---\nname: empty-str-agent\ndescription: d\nagent: \"\"\n---\nbody\n",
        )
        .await;

        let skills = scan_skills(global.path(), workspace.path(), "").await;
        let names: Vec<_> = skills.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"nullagent"), "agent=None kept for empty current_agent");
        assert!(
            !names.contains(&"empty-str-agent"),
            "agent=Some(\"\") strictly not equal to current_agent=\"\""
        );
    }

    #[tokio::test]
    async fn scan_ignores_non_md_files() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(global.path(), "real.md", "---\nname: real\ndescription: d\n---\nbody\n").await;
        write_file(global.path(), "notes.txt", "not a skill").await;
        write_file(global.path(), "README", "no extension").await;

        let skills = scan_skills(global.path(), workspace.path(), "").await;
        let names: Vec<_> = skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["real"]);
    }

    #[tokio::test]
    async fn scan_missing_workspace_dir_returns_only_global() {
        let global = TempDir::new().unwrap();
        write_file(global.path(), "g.md", "---\nname: g\ndescription: d\n---\nbody\n").await;

        let missing_workspace = global.path().join("does-not-exist");
        let skills = scan_skills(global.path(), &missing_workspace, "").await;
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "g");
    }

    #[tokio::test]
    async fn scan_both_dirs_missing_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("nope-a");
        let b = tmp.path().join("nope-b");
        let skills = scan_skills(&a, &b, "feishu").await;
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn scan_workspace_shadowing_with_agent_mismatch_filters_out() {
        // global has `shared` with agent=None, workspace has `shared` with agent=coder.
        // For current_agent="feishu", workspace entry shadows global, then the
        // agent filter drops the shadowed entry → the name vanishes entirely.
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(
            global.path(),
            "shared.md",
            "---\nname: shared\ndescription: global\n---\nbody\n",
        )
        .await;
        write_file(
            workspace.path(),
            "shared.md",
            "---\nname: shared\ndescription: workspace\nagent: coder\n---\nbody\n",
        )
        .await;

        let skills = scan_skills(global.path(), workspace.path(), "feishu").await;
        assert!(
            skills.iter().all(|s| s.name != "shared"),
            "workspace shadows global, then agent filter drops it entirely"
        );
    }

    #[tokio::test]
    async fn scan_does_not_recurse_into_subdirectories() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(global.path(), "top.md", "---\nname: top\ndescription: d\n---\nbody\n").await;
        // Nested skill file in a subdirectory — must NOT be picked up.
        write_file(
            global.path(),
            "nested/inner.md",
            "---\nname: inner\ndescription: d\n---\nbody\n",
        )
        .await;

        let skills = scan_skills(global.path(), workspace.path(), "").await;
        let names: Vec<_> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"top"));
        assert!(!names.contains(&"inner"), "subdirectory files must not be scanned");
    }

    // =============================================================
    // render_skill_index
    // =============================================================

    #[test]
    fn render_empty_slice_returns_empty_string() {
        assert_eq!(render_skill_index(&[]), "");
    }

    #[test]
    fn render_single_skill_with_when_to_use() {
        let skills = vec![meta("weather", "Check the weather.", Some("When asked about weather."))];
        let out = render_skill_index(&skills);

        assert!(out.contains("- weather: Check the weather."));
        assert!(out.contains("When to use: When asked about weather."));
    }

    #[test]
    fn render_single_skill_without_when_to_use_omits_line() {
        let skills = vec![meta("calc", "Basic calculator.", None)];
        let out = render_skill_index(&skills);

        assert!(out.contains("- calc: Basic calculator."));
        assert!(
            !out.contains("When to use:"),
            "When to use line must be absent when field is None; got:\n{out}"
        );
    }

    #[test]
    fn render_three_skills_produces_three_entries_with_delimiters() {
        let skills = vec![
            meta("a", "first", None),
            meta("b", "second", None),
            meta("c", "third", None),
        ];
        let out = render_skill_index(&skills);

        assert_eq!(
            out.matches("\n- ").count() + if out.starts_with("- ") { 1 } else { 0 },
            // Each entry begins with "- " at start of a line; we expect 3 such lines.
            3,
            "must have exactly 3 skill entries; output:\n{out}"
        );
        assert!(out.contains("- a: first"));
        assert!(out.contains("- b: second"));
        assert!(out.contains("- c: third"));
    }

    #[test]
    fn render_has_header_and_footer_delimiters() {
        let skills = vec![meta("x", "y", None)];
        let out = render_skill_index(&skills);

        assert!(
            out.contains("--- AVAILABLE SKILLS ---"),
            "must contain AVAILABLE SKILLS header; got:\n{out}"
        );
        assert!(
            out.contains("--- END SKILLS ---"),
            "must contain END SKILLS footer; got:\n{out}"
        );
        let start = out.find("--- AVAILABLE SKILLS ---").unwrap();
        let end = out.find("--- END SKILLS ---").unwrap();
        assert!(start < end, "header must come before footer");
    }

    #[test]
    fn render_description_with_colon_does_not_break_format() {
        let skills = vec![meta("cfg", "Usage: set value", None)];
        let out = render_skill_index(&skills);
        // The literal "- cfg: Usage: set value" should appear — the renderer
        // simply prints "- <name>: <description>"; inner colons are fine.
        assert!(
            out.contains("- cfg: Usage: set value"),
            "colon inside description must survive unchanged; got:\n{out}"
        );
    }

    #[test]
    fn render_description_with_newline_is_replaced_with_space() {
        // Policy: summary line must be exactly one line → embedded newlines
        // in description are replaced with a single space.
        let skills = vec![meta("multi", "line one\nline two", None)];
        let out = render_skill_index(&skills);

        assert!(
            out.contains("- multi: line one line two"),
            "embedded newline in description must be replaced with a space; got:\n{out}"
        );
        // There must not be a lone "line two" on its own line (which would
        // mean the newline was preserved and broke the one-line invariant).
        assert!(
            !out.contains("\nline two"),
            "description newline must not be preserved; got:\n{out}"
        );
    }

    // =============================================================
    // load_skill_body
    // =============================================================

    #[tokio::test]
    async fn load_body_strips_frontmatter() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(
            global.path(),
            "greet.md",
            "---\nname: greet\ndescription: d\n---\nHello, $ARGUMENTS!\n",
        )
        .await;

        let body = load_skill_body("greet", global.path(), workspace.path())
            .await
            .unwrap();

        assert!(!body.contains("---"), "frontmatter delimiters must be gone:\n{body}");
        assert!(!body.contains("name:"), "frontmatter fields must be gone:\n{body}");
        assert!(body.contains("Hello, $ARGUMENTS!"));
    }

    #[tokio::test]
    async fn load_body_no_frontmatter_returns_entire_file() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        let content = "Just body text.\nNo frontmatter here.\n";
        write_file(global.path(), "plain.md", content).await;

        let body = load_skill_body("plain", global.path(), workspace.path())
            .await
            .unwrap();
        assert_eq!(body, content);
    }

    #[tokio::test]
    async fn load_body_preserves_arguments_placeholder() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(
            global.path(),
            "echo.md",
            "---\nname: echo\ndescription: d\n---\nArgs were: $ARGUMENTS\n",
        )
        .await;

        let body = load_skill_body("echo", global.path(), workspace.path())
            .await
            .unwrap();
        assert!(body.contains("$ARGUMENTS"), "substitution is caller's job; got:\n{body}");
    }

    #[tokio::test]
    async fn load_body_workspace_shadows_global() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        write_file(
            global.path(),
            "shared.md",
            "---\nname: shared\ndescription: d\n---\nGLOBAL BODY MARKER\n",
        )
        .await;
        write_file(
            workspace.path(),
            "shared.md",
            "---\nname: shared\ndescription: d\n---\nWORKSPACE BODY MARKER\n",
        )
        .await;

        let body = load_skill_body("shared", global.path(), workspace.path())
            .await
            .unwrap();

        assert!(
            body.contains("WORKSPACE BODY MARKER"),
            "workspace must win; got:\n{body}"
        );
        assert!(
            !body.contains("GLOBAL BODY MARKER"),
            "global body must not appear when workspace shadows it; got:\n{body}"
        );
    }

    #[tokio::test]
    async fn load_body_missing_name_returns_none() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();
        assert!(
            load_skill_body("no-such-skill", global.path(), workspace.path())
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn load_body_drops_leading_blank_line_after_closing_delimiter() {
        let global = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        // Typical frontmatter pattern: `---` on its own line immediately
        // followed by a newline then content. The caller wants clean body
        // text, not a leading blank.
        write_file(
            global.path(),
            "clean.md",
            "---\nname: clean\ndescription: d\n---\nFirst line of body.\nSecond line.\n",
        )
        .await;

        let body = load_skill_body("clean", global.path(), workspace.path())
            .await
            .unwrap();
        assert!(
            body.starts_with("First line of body."),
            "body must not start with a blank line; got: {:?}",
            body
        );
    }
}

/// Scan a single directory for top-level `*.md` skill files. Missing or
/// unreadable directories yield an empty vector — we don't error.
async fn scan_dir(dir: &Path) -> Vec<SkillMeta> {
    let mut entries = match fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        paths.push(path);
    }
    // Stable ordering — filesystem traversal order isn't guaranteed.
    paths.sort();

    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        if let Some(meta) = parse_skill_file(&p).await {
            out.push(meta);
        }
    }
    out
}

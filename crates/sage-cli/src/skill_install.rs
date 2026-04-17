//! `sage skill add` / `sage skill list` implementation — task #82.
//!
//! Installs a skill (external directory containing `SKILL.md` + optional
//! `references/` / `scenes/` subdirs) into a target agent's
//! `workspace/skills/<name>/` tree, and maintains the self-describing
//! `INDEX.md` on disk.
//!
//! Source kinds supported in v0.0.2:
//! - **Local path** — starts with `/`, `.`, `~` → `cp -r` semantic.
//! - **Git URL** — `http(s)://…`, `git@…:…`, or any path ending in `.git`
//!   → `git clone --depth 1 <url> <dst>`. Requires `git` on `$PATH`.
//!
//! `@user/repo` npm-style (via `npx skills add`) is deferred to v0.0.3
//! (task C per the #82 split).

use anyhow::{Context as _, Result, anyhow};
use std::path::{Path, PathBuf};

/// Classification of an `<source>` argument to `sage skill add`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    /// Local filesystem directory. `~` is expanded by the caller.
    LocalPath(PathBuf),
    /// Git-clonable URL. We preserve the original string; the transport
    /// layer (git) does its own parsing.
    GitUrl(String),
    /// npm-style skill package identifier (e.g. `@scope/name` or
    /// `owner/name`). Installed via `npx skills add` rather than a raw
    /// git clone; the shell-out handles registry resolution and caching.
    NpmStyle(String),
}

/// Decide which kind of source the user passed.
///
/// Rules (order matters — the first match wins):
/// - `.git` suffix, `http(s)://`, or `git@…:` prefix → `GitUrl`
/// - `/`, `.`, or `~` leading → `LocalPath`
/// - `@scope/name` or `owner/repo` shape (one `/`, no `:`) → `NpmStyle`
/// - anything else → error
pub fn detect_source(source: &str) -> Result<SourceKind> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("skill source must not be empty"));
    }
    // Git URL patterns — check first because some git URLs contain `/`.
    if trimmed.ends_with(".git")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("git@")
        || trimmed.starts_with("git://")
        || trimmed.starts_with("ssh://")
    {
        return Ok(SourceKind::GitUrl(trimmed.to_string()));
    }
    // Local path: absolute, ~-prefixed, or explicit relative.
    if trimmed.starts_with('/')
        || trimmed.starts_with('~')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed == "."
        || trimmed == ".."
    {
        return Ok(SourceKind::LocalPath(expand_tilde(trimmed)));
    }
    // npm-style: `@scope/name` OR `owner/name`. Constraints:
    //   - Exactly one `/` (more slashes could be a mistyped path)
    //   - No `:` (filters out SSH URLs we missed above, plus Windows C:\)
    //   - No whitespace / control chars
    //   - Both halves non-empty
    if !trimmed.contains(':') && !trimmed.chars().any(char::is_whitespace) {
        let body = trimmed.strip_prefix('@').unwrap_or(trimmed);
        let parts: Vec<&str> = body.split('/').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Ok(SourceKind::NpmStyle(trimmed.to_string()));
        }
    }
    Err(anyhow!(
        "unsupported skill source '{source}': expected local path (./, /, ~), \
         git URL (https://…, git@…:…, or .git suffix), or npm-style \
         package (@scope/name or owner/repo)."
    ))
}

/// `~/path` → `<home>/path`. Leaves non-tilde paths unchanged.
fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix('~') {
        if let Some(home) = sage_runner::home_dir() {
            if rest.is_empty() {
                return home;
            }
            if let Some(rest) = rest.strip_prefix('/') {
                return home.join(rest);
            }
            // `~user/…` — unsupported, fall through as literal.
        }
    }
    PathBuf::from(s)
}

/// Derive the default skill directory name from a source.
///
/// - Local path: basename of the path (last component).
/// - Git URL: repo name with `.git` suffix stripped.
pub fn default_name_for(kind: &SourceKind) -> Result<String> {
    match kind {
        SourceKind::LocalPath(p) => {
            // Literal `file_name` first so hypothetical paths like
            // `/home/u/skills/lark-base` still yield `lark-base` without
            // needing the dir to physically exist on the test host.
            if let Some(name) = p
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.is_empty())
            {
                return Ok(name.to_string());
            }
            // `Path::file_name` returns None on `.`, `..`, and paths that
            // normalise to bare roots — `sage skill add . --agent foo`
            // from inside the skill dir is a common user flow. Canonicalise
            // to reach the real directory name.
            let resolved = std::fs::canonicalize(p)
                .with_context(|| format!("cannot resolve local source {}", p.display()))?;
            resolved
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("cannot derive skill name from path {}", p.display()))
        }
        SourceKind::GitUrl(url) => {
            // Strip trailing slash, then take tail after last `/` or `:`.
            let trimmed = url.trim_end_matches('/');
            let tail = trimmed
                .rsplit_once('/')
                .map(|(_, t)| t)
                .or_else(|| trimmed.rsplit_once(':').map(|(_, t)| t))
                .unwrap_or(trimmed);
            let name = tail.trim_end_matches(".git");
            if name.is_empty() {
                return Err(anyhow!("cannot derive skill name from git URL '{url}'"));
            }
            Ok(name.to_string())
        }
        SourceKind::NpmStyle(spec) => {
            // `@scope/name` → `name`; `owner/repo` → `repo`.
            let tail = spec
                .rsplit_once('/')
                .map(|(_, t)| t)
                .unwrap_or(spec.as_str());
            if tail.is_empty() {
                return Err(anyhow!("cannot derive skill name from npm spec '{spec}'"));
            }
            Ok(tail.to_string())
        }
    }
}

/// Result of a successful install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledSkill {
    pub name: String,
    /// One-line description extracted from SKILL.md frontmatter; empty if
    /// the skill lacks a frontmatter `description:` field.
    pub description: String,
    pub dir: PathBuf,
}

/// Install a skill from `source` into `<workspace_skills_dir>/<name>/`.
///
/// Returns `InstalledSkill` describing what was installed; also updates
/// `<workspace_skills_dir>/INDEX.md` with a new entry when the skill name
/// is not already present.
pub async fn install_skill(
    workspace_skills_dir: &Path,
    source: &str,
    name_override: Option<&str>,
) -> Result<InstalledSkill> {
    let kind = detect_source(source)?;
    let name = match name_override {
        Some(n) => n.to_string(),
        None => default_name_for(&kind)?,
    };
    validate_skill_name(&name)?;

    let dst = workspace_skills_dir.join(&name);
    if dst.exists() {
        return Err(anyhow!(
            "skill '{name}' already exists at {} — remove it first or pass \
             a different --name",
            dst.display()
        ));
    }

    // Ensure the parent workspace/skills/ dir exists.
    tokio::fs::create_dir_all(workspace_skills_dir)
        .await
        .with_context(|| {
            format!(
                "failed to create workspace skills dir {}",
                workspace_skills_dir.display()
            )
        })?;

    // All three transports can leave a half-populated `dst` on error
    // (copy fails on an unreadable file, git clone dies on auth/network,
    // npx exits non-zero mid-download). Without rollback here the user
    // sees "skill already exists" on retry and has to `rm -rf` manually.
    // Centralise cleanup at the boundary: any Err → remove dst.
    let fetch_result: Result<()> = match &kind {
        SourceKind::LocalPath(src) => copy_dir_recursive(src, &dst).await,
        SourceKind::GitUrl(url) => git_clone(url, &dst).await,
        SourceKind::NpmStyle(spec) => npx_skills_add(spec, workspace_skills_dir).await,
    };
    if let Err(e) = fetch_result {
        let _ = tokio::fs::remove_dir_all(&dst).await;
        return Err(e);
    }

    // Verify the installed skill actually has a SKILL.md.
    let skill_md = dst.join("SKILL.md");
    if !skill_md.exists() {
        // Roll back: the installed tree is invalid.
        let _ = tokio::fs::remove_dir_all(&dst).await;
        return Err(anyhow!(
            "source '{source}' does not contain SKILL.md at its root"
        ));
    }

    let description = read_skill_description(&skill_md).await.unwrap_or_default();
    ensure_index_entry(workspace_skills_dir, &name, &description).await?;

    Ok(InstalledSkill {
        name,
        description,
        dir: dst,
    })
}

/// Validate a skill directory name — same spirit as `validate_agent_name`
/// but the skills live in a per-agent dir, so `/` and `..` are the hard
/// stops. YAML-reserved chars etc. don't apply here (we don't synthesize
/// YAML frontmatter from the name).
fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("skill name must not be empty"));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(anyhow!(
            "invalid skill name '{name}': must not contain path separators"
        ));
    }
    if name == "." || name == ".." || name == "INDEX.md" {
        return Err(anyhow!("invalid skill name '{name}': reserved"));
    }
    Ok(())
}

/// Copy `src` → `dst` recursively. Refuses if `src` is not a directory.
async fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    let src_meta = tokio::fs::metadata(src)
        .await
        .with_context(|| format!("local source {} does not exist", src.display()))?;
    if !src_meta.is_dir() {
        return Err(anyhow!(
            "local source {} must be a directory containing SKILL.md",
            src.display()
        ));
    }
    copy_dir_inner(src, dst).await
}

// Boxed to break the async fn recursion size cycle.
fn copy_dir_inner<'a>(
    src: &'a Path,
    dst: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(dst)
            .await
            .with_context(|| format!("failed to create {}", dst.display()))?;
        let mut rd = tokio::fs::read_dir(src)
            .await
            .with_context(|| format!("failed to read {}", src.display()))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .with_context(|| format!("read_dir entry in {}", src.display()))?
        {
            let ft = entry.file_type().await?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if ft.is_dir() {
                copy_dir_inner(&from, &to).await?;
            } else if ft.is_symlink() {
                // Skip symlinks — copying a dangling/external symlink into
                // the sandbox would leak host paths.
                tracing::warn!(path = %from.display(), "skipping symlink during skill install");
            } else {
                tokio::fs::copy(&from, &to)
                    .await
                    .with_context(|| format!("copy {} → {}", from.display(), to.display()))?;
            }
        }
        Ok(())
    })
}

/// Shell out to `npx --yes skills add <spec>` inside `workspace_skills_dir`.
///
/// This hands the registry / cache / auth concerns to the `skills` CLI
/// rather than reimplementing them; we only own the integration point.
/// The `--yes` flag suppresses npm's "install X?" interactive prompt.
///
/// When `npx` is not on PATH we surface a concrete remediation hint
/// instead of the raw `NotFound` IO error. npm-style is a user-facing
/// convenience; the friendly error is load-bearing UX.
async fn npx_skills_add(spec: &str, workspace_skills_dir: &Path) -> Result<()> {
    // Windows ships `npx` as `npx.cmd` (a shim). std::process::Command
    // doesn't auto-append `.cmd`, so explicit platform split keeps
    // Windows's friendly-error path honest (code-review Important #6).
    #[cfg(windows)]
    let program = "npx.cmd";
    #[cfg(not(windows))]
    let program = "npx";

    let mut cmd = tokio::process::Command::new(program);
    cmd.arg("--yes")
        .arg("skills")
        .arg("add")
        .arg(spec)
        .current_dir(workspace_skills_dir);
    let status = cmd.status().await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "`npx` was not found on PATH — install Node.js (>= 18) to enable \
                 npm-style skill install, or fall back to \
                 `sage skill add --agent <name> <local-path-or-git-url>`. \
                 Original error: {e}"
            )
        } else {
            anyhow!("failed to invoke `npx skills add {spec}`: {e}")
        }
    })?;
    if !status.success() {
        return Err(anyhow!(
            "`npx skills add {spec}` exited with status {status}"
        ));
    }
    Ok(())
}

/// Shell out to `git clone --depth 1 <url> <dst>`.
async fn git_clone(url: &str, dst: &Path) -> Result<()> {
    let status = tokio::process::Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--")
        .arg(url)
        .arg(dst)
        .status()
        .await
        .with_context(|| "failed to invoke `git` — is it installed and on PATH?")?;
    if !status.success() {
        return Err(anyhow!(
            "git clone exited with status {status} for url '{url}'"
        ));
    }
    // Remove .git to save space and prevent accidental commits inside the
    // agent's workspace on subsequent `git` invocations.
    let dot_git = dst.join(".git");
    if dot_git.exists() {
        let _ = tokio::fs::remove_dir_all(&dot_git).await;
    }
    Ok(())
}

/// Read the `description:` field from a SKILL.md YAML frontmatter.
///
/// Returns an empty string on any parse failure — the description is a
/// best-effort UX hint, not load-bearing state.
async fn read_skill_description(skill_md: &Path) -> Result<String> {
    let content = tokio::fs::read_to_string(skill_md).await?;
    if !content.starts_with("---") {
        return Ok(String::new());
    }
    // Grab between first two `---` lines.
    let rest = content.strip_prefix("---").unwrap_or(&content);
    let end = rest.find("\n---").unwrap_or(rest.len());
    let frontmatter = &rest[..end];
    for line in frontmatter.lines() {
        if let Some(val) = line.strip_prefix("description:") {
            // Strip surrounding quotes + whitespace.
            let v = val.trim().trim_matches('"').trim_matches('\'').to_string();
            return Ok(v);
        }
    }
    Ok(String::new())
}

/// Append `- **<name>** — <description>` to INDEX.md when `name` is not
/// already present. Idempotent: re-running the installer is a no-op for
/// the index when nothing changes.
async fn ensure_index_entry(
    workspace_skills_dir: &Path,
    name: &str,
    description: &str,
) -> Result<()> {
    let index_path = workspace_skills_dir.join("INDEX.md");
    let existing = tokio::fs::read_to_string(&index_path).await.unwrap_or_default();

    // Fast path: name already mentioned as a markdown bullet's bold token.
    let marker = format!("**{name}**");
    if existing.contains(&marker) {
        return Ok(());
    }

    let entry_line = if description.is_empty() {
        format!("- **{name}** — (no description)\n")
    } else {
        format!("- **{name}** — {description}\n")
    };

    // Preserve any seed content; append a trailing newline if the file
    // doesn't already end with one.
    let mut new_content = existing;
    if !new_content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }
    new_content.push_str(&entry_line);
    tokio::fs::write(&index_path, new_content)
        .await
        .with_context(|| format!("failed to update {}", index_path.display()))?;
    Ok(())
}

/// List installed skills by scanning `<workspace_skills_dir>/<name>/SKILL.md`.
///
/// Independent of INDEX.md: the filesystem is the source of truth, INDEX.md
/// is the agent's maintained view over it.
pub async fn list_installed(workspace_skills_dir: &Path) -> Result<Vec<InstalledSkill>> {
    let mut out = Vec::new();
    let mut rd = match tokio::fs::read_dir(workspace_skills_dir).await {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e).with_context(|| workspace_skills_dir.display().to_string()),
    };
    while let Some(entry) = rd.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .unwrap_or_else(|os| os.to_string_lossy().into_owned());
        let description = read_skill_description(&skill_md).await.unwrap_or_default();
        out.push(InstalledSkill {
            name,
            description,
            dir: entry.path(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

// ── CLI entry points ─────────────────────────────────────────────────────

/// `sage skill add --agent <name> <source> [--name <n>]`.
pub async fn run_skill_add(
    agent: &str,
    source: &str,
    name_override: Option<&str>,
) -> Result<()> {
    crate::serve::validate_agent_name(agent)?;
    let agent_dir = crate::serve::sage_agents_dir()?.join(agent);
    // Agent must have been initialised first — otherwise `install_skill`
    // happily creates `~/.sage/agents/<typo>/workspace/skills/` and leaves
    // the operator with an orphaned tree (no config.yaml / AGENT.md) that
    // every subsequent `sage <cmd> --agent <typo>` will reject. Gate on
    // config.yaml presence so the mistake surfaces at install time.
    let config_path = agent_dir.join("config.yaml");
    if !config_path.exists() {
        return Err(anyhow!(
            "agent '{agent}' is not initialised (missing {}); \
             run `sage init --agent {agent}` first",
            config_path.display()
        ));
    }
    let skills_dir = agent_dir.join("workspace").join("skills");
    let installed = install_skill(&skills_dir, source, name_override).await?;
    println!(
        "✓ installed skill '{}' at {}",
        installed.name,
        installed.dir.display()
    );
    if !installed.description.is_empty() {
        println!("  description: {}", installed.description);
    }
    Ok(())
}

/// `sage skill list --agent <name>`.
pub async fn run_skill_list(agent: &str) -> Result<()> {
    crate::serve::validate_agent_name(agent)?;
    let skills_dir = crate::serve::sage_agents_dir()?
        .join(agent)
        .join("workspace")
        .join("skills");
    let list = list_installed(&skills_dir).await?;
    if list.is_empty() {
        println!("no skills installed — try `sage skill add --agent {agent} <source>`");
        return Ok(());
    }
    for s in list {
        if s.description.is_empty() {
            println!("- {}", s.name);
        } else {
            println!("- {} — {}", s.name, s.description);
        }
    }
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_source ─────────────────────────────────────────────────────

    #[test]
    fn detect_source_absolute_path_is_local() {
        assert!(matches!(
            detect_source("/abs/path/to/skill").unwrap(),
            SourceKind::LocalPath(_)
        ));
    }

    #[test]
    fn detect_source_tilde_prefix_is_local() {
        assert!(matches!(
            detect_source("~/my-skill").unwrap(),
            SourceKind::LocalPath(_)
        ));
    }

    #[test]
    fn detect_source_dot_slash_is_local() {
        assert!(matches!(
            detect_source("./local-skill").unwrap(),
            SourceKind::LocalPath(_)
        ));
    }

    #[test]
    fn detect_source_https_url_is_git() {
        assert!(matches!(
            detect_source("https://github.com/u/r").unwrap(),
            SourceKind::GitUrl(_)
        ));
    }

    #[test]
    fn detect_source_git_suffix_is_git() {
        assert!(matches!(
            detect_source("https://github.com/u/r.git").unwrap(),
            SourceKind::GitUrl(_)
        ));
    }

    #[test]
    fn detect_source_ssh_git_is_git() {
        assert!(matches!(
            detect_source("git@github.com:u/r.git").unwrap(),
            SourceKind::GitUrl(_)
        ));
    }

    /// v0.0.3 #81 Wave 4: `owner/repo` now routes to NpmStyle instead of
    /// erroring. Detection is decoupled from whether npx is installed —
    /// the install step surfaces that failure later with a clearer message.
    #[test]
    fn detect_source_owner_repo_is_npm_style() {
        assert!(matches!(
            detect_source("pbakaus/impeccable").unwrap(),
            SourceKind::NpmStyle(_)
        ));
    }

    /// `@scope/name` (leading `@`) is the canonical npm-style shape and
    /// must route to NpmStyle — v0.0.3 needs this for distribution paths
    /// that publish under npm scopes.
    #[test]
    fn detect_source_at_scope_name_is_npm_style() {
        let src = detect_source("@larksuite/lark-base").unwrap();
        assert!(matches!(src, SourceKind::NpmStyle(_)));
        if let SourceKind::NpmStyle(spec) = src {
            assert_eq!(spec, "@larksuite/lark-base", "spec must be preserved verbatim");
        }
    }

    /// `foo/bar/baz` has too many slashes — it could be a mistyped path or
    /// an unsupported scope nesting. Error rather than misclassify.
    #[test]
    fn detect_source_multi_slash_bareword_is_error() {
        let err = detect_source("foo/bar/baz").unwrap_err().to_string();
        assert!(err.contains("unsupported"));
    }

    /// A single bareword (no slash) isn't a complete npm spec and we don't
    /// want to guess whether it's a registry name or typo.
    #[test]
    fn detect_source_single_bareword_is_error() {
        assert!(detect_source("lark-base").is_err());
    }

    #[test]
    fn detect_source_empty_is_error() {
        assert!(detect_source("").is_err());
        assert!(detect_source("   ").is_err());
    }

    // ── default_name_for ──────────────────────────────────────────────────

    #[test]
    fn default_name_for_local_uses_basename() {
        let kind = SourceKind::LocalPath(PathBuf::from("/home/u/skills/lark-base"));
        assert_eq!(default_name_for(&kind).unwrap(), "lark-base");
    }

    #[test]
    fn default_name_for_https_git_url_strips_dot_git() {
        let kind = SourceKind::GitUrl("https://github.com/u/lark-calendar.git".into());
        assert_eq!(default_name_for(&kind).unwrap(), "lark-calendar");
    }

    #[test]
    fn default_name_for_ssh_git_url_uses_repo_name() {
        let kind = SourceKind::GitUrl("git@github.com:u/lark-im.git".into());
        assert_eq!(default_name_for(&kind).unwrap(), "lark-im");
    }

    #[test]
    fn default_name_for_trailing_slash_url_still_works() {
        let kind = SourceKind::GitUrl("https://github.com/u/skill/".into());
        assert_eq!(default_name_for(&kind).unwrap(), "skill");
    }

    /// v0.0.3 Wave 4: npm specs derive their directory name from the tail
    /// (everything after the last `/`), stripping any `@scope/` prefix.
    #[test]
    fn default_name_for_npm_scope_strips_scope() {
        let kind = SourceKind::NpmStyle("@larksuite/lark-base".into());
        assert_eq!(default_name_for(&kind).unwrap(), "lark-base");
    }

    #[test]
    fn default_name_for_npm_owner_repo_uses_repo() {
        let kind = SourceKind::NpmStyle("pbakaus/impeccable".into());
        assert_eq!(default_name_for(&kind).unwrap(), "impeccable");
    }

    /// Review P2: `.` and `..` used to fail with "cannot derive name"
    /// because `Path::file_name` returns None on them. Now they resolve
    /// via canonicalize to a real directory name.
    #[tokio::test]
    async fn default_name_for_dot_resolves_to_current_dir_basename() {
        let tmp = tempfile::TempDir::new().unwrap();
        let inner = tmp.path().join("my-skill-dir");
        std::fs::create_dir_all(&inner).unwrap();
        // Create SKILL.md so the dir looks like a skill — canonicalize
        // needs the path to exist.
        std::fs::write(inner.join("SKILL.md"), "---\nname: x\n---\nbody").unwrap();

        // Run from inside the skill dir and feed `.`.
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&inner).unwrap();
        let name_res = default_name_for(&SourceKind::LocalPath(PathBuf::from(".")));
        std::env::set_current_dir(&prev).unwrap();

        let name = name_res.expect("dot path must derive a name via canonicalize");
        assert_eq!(name, "my-skill-dir");
    }

    // ── install_skill (local only — git requires real git binary) ─────────

    fn write_skill(dir: &Path, name: &str, description: &str, body: &str) {
        let d = dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        let content = format!(
            "---\nname: {name}\ndescription: \"{description}\"\n---\n\n{body}"
        );
        std::fs::write(d.join("SKILL.md"), content).unwrap();
    }

    #[tokio::test]
    async fn install_local_skill_copies_skill_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src_root = tmp.path().join("external");
        write_skill(&src_root, "lark-base", "base ops", "body here");
        let src = src_root.join("lark-base");
        let ws = tmp.path().join("ws").join("skills");

        let installed = install_skill(&ws, src.to_str().unwrap(), None).await.unwrap();
        assert_eq!(installed.name, "lark-base");
        assert!(installed.dir.join("SKILL.md").exists());
        let body = std::fs::read_to_string(installed.dir.join("SKILL.md")).unwrap();
        assert!(body.contains("body here"));
    }

    #[tokio::test]
    async fn install_local_skill_copies_references_subdir() {
        // SKILL.md plus a references/<file>.md subdir must all land in dst.
        let tmp = tempfile::TempDir::new().unwrap();
        let src_root = tmp.path().join("external");
        std::fs::create_dir_all(src_root.join("my-skill").join("references")).unwrap();
        std::fs::write(
            src_root.join("my-skill").join("SKILL.md"),
            "---\nname: my-skill\ndescription: \"hi\"\n---\n",
        )
        .unwrap();
        std::fs::write(
            src_root.join("my-skill").join("references").join("api.md"),
            "## api ref",
        )
        .unwrap();

        let src = src_root.join("my-skill");
        let ws = tmp.path().join("ws").join("skills");
        let installed = install_skill(&ws, src.to_str().unwrap(), None).await.unwrap();

        assert!(installed.dir.join("SKILL.md").exists());
        assert!(installed.dir.join("references").join("api.md").exists());
    }

    #[tokio::test]
    async fn install_honors_name_override() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src_root = tmp.path().join("external");
        write_skill(&src_root, "ugly-upstream-name", "", "body");
        let src = src_root.join("ugly-upstream-name");
        let ws = tmp.path().join("ws").join("skills");

        let installed = install_skill(
            &ws,
            src.to_str().unwrap(),
            Some("nicer-local-name"),
        )
        .await
        .unwrap();

        assert_eq!(installed.name, "nicer-local-name");
        assert!(ws.join("nicer-local-name").join("SKILL.md").exists());
        assert!(!ws.join("ugly-upstream-name").exists());
    }

    /// Review P3: if the fetch step itself fails (source unreadable,
    /// git clone failure, npx failure) we must roll back `dst` — the
    /// previous code only rolled back on the post-fetch SKILL.md check,
    /// leaving the dir on retry. Simulate via a LocalPath that points
    /// to a file (not a directory): `copy_dir_recursive` bails inside
    /// `metadata().is_dir()` check, but only after `create_dir_all(dst)`
    /// already ran.
    #[tokio::test]
    async fn install_rolls_back_when_fetch_step_fails_midway() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Source is a regular file, not a directory — triggers the
        // "must be a directory" branch inside copy_dir_recursive.
        let bogus_file = tmp.path().join("not-a-dir");
        std::fs::write(&bogus_file, b"hi").unwrap();
        let ws = tmp.path().join("ws").join("skills");

        let err = install_skill(&ws, bogus_file.to_str().unwrap(), None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("must be a directory"),
            "error must surface the underlying fetch failure: {err}"
        );
        // Rolled back — the half-created dst (if any) must not leak.
        assert!(
            !ws.join("not-a-dir").exists(),
            "dst must be removed when fetch step fails"
        );
    }

    #[tokio::test]
    async fn install_rolls_back_when_source_lacks_skill_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src_root = tmp.path().join("external").join("bogus");
        std::fs::create_dir_all(&src_root).unwrap();
        std::fs::write(src_root.join("README.md"), "no SKILL here").unwrap();
        let ws = tmp.path().join("ws").join("skills");

        let err = install_skill(&ws, src_root.to_str().unwrap(), None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("SKILL.md"),
            "error must mention SKILL.md: {err}"
        );
        // Rolled back — dst dir must not leak.
        assert!(!ws.join("bogus").exists(), "dst must be removed on failure");
    }

    #[tokio::test]
    async fn install_rejects_existing_destination() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src_root = tmp.path().join("external");
        write_skill(&src_root, "dup", "", "body");
        let src = src_root.join("dup");
        let ws = tmp.path().join("ws").join("skills");
        install_skill(&ws, src.to_str().unwrap(), None).await.unwrap();

        let second = install_skill(&ws, src.to_str().unwrap(), None).await;
        assert!(
            second.is_err(),
            "second install of same name must fail, not silently overwrite"
        );
    }

    #[tokio::test]
    async fn install_appends_new_entry_to_index_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src_root = tmp.path().join("external");
        write_skill(&src_root, "fresh", "fresh description", "body");
        let src = src_root.join("fresh");
        let ws = tmp.path().join("ws").join("skills");
        // Seed existing INDEX.md so we test the append-not-overwrite path.
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("INDEX.md"), "# Skills\n").unwrap();

        install_skill(&ws, src.to_str().unwrap(), None).await.unwrap();
        let index = std::fs::read_to_string(ws.join("INDEX.md")).unwrap();
        assert!(
            index.contains("**fresh**"),
            "new skill must appear as bold token: {index}"
        );
        assert!(
            index.contains("fresh description"),
            "description must be pulled from frontmatter: {index}"
        );
        assert!(
            index.starts_with("# Skills"),
            "pre-existing seed content must be preserved: {index}"
        );
    }

    #[tokio::test]
    async fn install_leaves_index_alone_when_name_already_present() {
        // Idempotency: re-describing a skill by hand in INDEX.md must
        // survive subsequent installs. The installer checks for
        // `**<name>**` and skips the append.
        let tmp = tempfile::TempDir::new().unwrap();
        let src_root = tmp.path().join("external");
        write_skill(&src_root, "stable", "generic upstream desc", "body");
        let src = src_root.join("stable");
        let ws = tmp.path().join("ws").join("skills");
        std::fs::create_dir_all(&ws).unwrap();
        let user_edited = "# Skills\n- **stable** — user's hand-curated description\n";
        std::fs::write(ws.join("INDEX.md"), user_edited).unwrap();
        // Simulate a re-install attempt (dst already exists → error).
        // Trigger the index codepath via a separate install with --name.
        install_skill(&ws, src.to_str().unwrap(), Some("fresh")).await.unwrap();
        let index = std::fs::read_to_string(ws.join("INDEX.md")).unwrap();
        // User's entry still there, verbatim.
        assert!(
            index.contains("user's hand-curated description"),
            "user-maintained INDEX.md line must be preserved"
        );
        // Plus the newly-installed `fresh` appended once.
        assert_eq!(
            index.matches("**fresh**").count(),
            1,
            "fresh entry must appear exactly once: {index}"
        );
    }

    // ── list_installed ────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_installed_scans_subdirs_with_skill_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().join("skills");
        write_skill(&ws, "alpha", "a desc", "body");
        write_skill(&ws, "beta", "b desc", "body");
        // Dir without SKILL.md — must be skipped.
        std::fs::create_dir_all(ws.join(".trash")).unwrap();

        let list = list_installed(&ws).await.unwrap();
        let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert_eq!(list[0].description, "a desc");
    }

    #[tokio::test]
    async fn list_installed_missing_dir_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let list = list_installed(&tmp.path().join("nonexistent")).await.unwrap();
        assert!(list.is_empty());
    }
}

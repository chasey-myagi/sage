//! Package manager — resolves and installs npm/git/local packages.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/package-manager.ts`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::config::CONFIG_DIR_NAME;
use crate::core::settings_manager::{PackageSource, SettingsManager};

const NETWORK_TIMEOUT_MS: u64 = 10_000;

fn is_offline_mode_enabled() -> bool {
    match std::env::var("SAGE_OFFLINE").or_else(|_| std::env::var("PI_OFFLINE")) {
        Ok(val) => {
            val == "1" || val.eq_ignore_ascii_case("true") || val.eq_ignore_ascii_case("yes")
        }
        Err(_) => false,
    }
}

// ============================================================================
// Public types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceScope {
    User,
    Project,
    Temporary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathMetadata {
    pub source: String,
    pub scope: String,
    pub origin: String,
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedResource {
    pub path: PathBuf,
    pub enabled: bool,
    pub metadata: PathMetadata,
}

#[derive(Debug, Default)]
pub struct ResolvedPaths {
    pub extensions: Vec<ResolvedResource>,
    pub skills: Vec<ResolvedResource>,
    pub prompts: Vec<ResolvedResource>,
    pub themes: Vec<ResolvedResource>,
}

pub type MissingSourceAction = String; // "install" | "skip" | "error"

#[derive(Debug, Clone)]
pub struct ProgressEvent {
    pub kind: String,   // "start" | "progress" | "complete" | "error"
    pub action: String, // "install" | "remove" | "update" | "clone" | "pull"
    pub source: String,
    pub message: Option<String>,
}

pub type ProgressCallback = Box<dyn Fn(ProgressEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct PackageUpdate {
    pub source: String,
    pub display_name: String,
    pub kind: String, // "npm" | "git"
    pub scope: SourceScope,
}

// ============================================================================
// Internal source types
// ============================================================================

#[derive(Debug, Clone)]
struct NpmSource {
    spec: String,
    name: String,
    pinned: bool,
}

#[derive(Debug, Clone)]
struct GitSource {
    repo: String,
    host: String,
    path: String,
    ref_: Option<String>,
    pinned: bool,
}

#[derive(Debug, Clone)]
struct LocalSource {
    path: String,
}

#[derive(Debug, Clone)]
enum ParsedSource {
    Npm(NpmSource),
    Git(GitSource),
    Local(LocalSource),
}

// ============================================================================
// Pi manifest
// ============================================================================

#[derive(Debug, Default, Deserialize)]
struct PiManifest {
    extensions: Option<Vec<String>>,
    skills: Option<Vec<String>>,
    prompts: Option<Vec<String>>,
    themes: Option<Vec<String>>,
}

// ============================================================================
// Resource accumulator
// ============================================================================

struct ResourceAccumulator {
    extensions: HashMap<PathBuf, (PathMetadata, bool)>,
    skills: HashMap<PathBuf, (PathMetadata, bool)>,
    prompts: HashMap<PathBuf, (PathMetadata, bool)>,
    themes: HashMap<PathBuf, (PathMetadata, bool)>,
}

impl ResourceAccumulator {
    fn new() -> Self {
        Self {
            extensions: HashMap::new(),
            skills: HashMap::new(),
            prompts: HashMap::new(),
            themes: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceType {
    Extensions,
    Skills,
    Prompts,
    Themes,
}

// ============================================================================
// File collection helpers
// ============================================================================

fn collect_files(dir: &Path, pattern: fn(&str) -> bool) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if !dir.exists() {
        return files;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }
        let ft = entry
            .file_type()
            .unwrap_or_else(|_| entry.file_type().unwrap());
        if ft.is_dir() {
            files.extend(collect_files(&path, pattern));
        } else if ft.is_file() && pattern(&name_str) {
            files.push(path);
        }
    }
    files
}

fn is_extension_file(name: &str) -> bool {
    name.ends_with(".ts") || name.ends_with(".js")
}

fn is_skill_file(name: &str) -> bool {
    name.ends_with(".md")
}

fn is_prompt_file(name: &str) -> bool {
    name.ends_with(".md")
}

fn is_theme_file(name: &str) -> bool {
    name.ends_with(".json")
}

fn collect_skill_entries(dir: &Path, include_root_files: bool) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    if !dir.exists() {
        return entries;
    }
    let Ok(dir_entries) = std::fs::read_dir(dir) else {
        return entries;
    };
    for entry in dir_entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            entries.extend(collect_skill_entries(&path, false));
        } else if ft.is_file() {
            let is_root_md = include_root_files && name_str.ends_with(".md");
            let is_skill_md = !include_root_files && name_str == "SKILL.md";
            if is_root_md || is_skill_md {
                entries.push(path);
            }
        }
    }
    entries
}

fn collect_resource_files(dir: &Path, resource_type: ResourceType) -> Vec<PathBuf> {
    match resource_type {
        ResourceType::Skills => collect_skill_entries(dir, true),
        ResourceType::Extensions => collect_extension_entries(dir),
        ResourceType::Prompts => collect_files(dir, is_prompt_file),
        ResourceType::Themes => collect_files(dir, is_theme_file),
    }
}

fn collect_extension_entries(dir: &Path) -> Vec<PathBuf> {
    if !dir.exists() {
        return Vec::new();
    }
    // Check if this dir has index.ts / index.js directly
    if let Some(entries) = resolve_extension_entries(dir) {
        return entries;
    }
    let mut result = Vec::new();
    let Ok(dir_entries) = std::fs::read_dir(dir) else {
        return result;
    };
    for entry in dir_entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_file() && is_extension_file(&name_str) {
            result.push(path);
        } else if ft.is_dir()
            && let Some(sub_entries) = resolve_extension_entries(&path)
        {
            result.extend(sub_entries);
        }
    }
    result
}

fn resolve_extension_entries(dir: &Path) -> Option<Vec<PathBuf>> {
    // Check package.json for pi.extensions
    let pkg_path = dir.join("package.json");
    if pkg_path.exists()
        && let Ok(content) = std::fs::read_to_string(&pkg_path)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
        && let Some(exts) = val
            .get("pi")
            .and_then(|p| p.get("extensions"))
            .and_then(|e| e.as_array())
    {
        let mut entries = Vec::new();
        for ext in exts {
            if let Some(ext_str) = ext.as_str() {
                let resolved = dir.join(ext_str);
                if resolved.exists() {
                    entries.push(resolved);
                }
            }
        }
        if !entries.is_empty() {
            return Some(entries);
        }
    }
    // Check for index.ts / index.js
    let index_ts = dir.join("index.ts");
    if index_ts.exists() {
        return Some(vec![index_ts]);
    }
    let index_js = dir.join("index.js");
    if index_js.exists() {
        return Some(vec![index_js]);
    }
    None
}

// ============================================================================
// Source parsing
// ============================================================================

fn parse_npm_spec(spec: &str) -> (String, Option<String>) {
    // "@scope/name@version" or "name@version" or "name"
    let re = regex::Regex::new(r"^(@?[^@]+(?:/[^@]+)?)(?:@(.+))?$").unwrap();
    if let Some(caps) = re.captures(spec) {
        let name = caps
            .get(1)
            .map(|m| m.as_str().to_owned())
            .unwrap_or_else(|| spec.to_owned());
        let version = caps.get(2).map(|m| m.as_str().to_owned());
        (name, version)
    } else {
        (spec.to_owned(), None)
    }
}

fn parse_git_url(source: &str) -> Option<GitSource> {
    // GitHub shorthand: "owner/repo" or "owner/repo@ref"
    let trimmed = source.trim();

    // SSH: git@github.com:owner/repo.git
    if trimmed.starts_with("git@")
        && let Some(colon_pos) = trimmed.find(':')
    {
        let host_part = &trimmed[4..colon_pos]; // after "git@"
        let path_part = &trimmed[colon_pos + 1..];
        let (path, ref_) = split_ref(path_part);
        let repo = if trimmed.contains("git@github.com") {
            format!("https://github.com/{}", path)
        } else {
            source.to_owned()
        };
        return Some(GitSource {
            repo,
            host: host_part.to_owned(),
            path: path.trim_end_matches(".git").to_owned(),
            ref_,
            pinned: false,
        });
    }

    // HTTPS: https://github.com/owner/repo(.git)(@ref)
    if (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
        && let Ok(url) = url::Url::parse(trimmed)
    {
        let host = url.host_str().unwrap_or("").to_owned();
        let path = url
            .path()
            .trim_start_matches('/')
            .trim_end_matches(".git")
            .to_owned();
        let (path_no_ref, ref_) = split_ref(&path);
        let pinned = ref_.is_some();
        return Some(GitSource {
            repo: format!("https://{}/{}", host, path_no_ref),
            host,
            path: path_no_ref,
            ref_,
            pinned,
        });
    }

    // Shorthand: "owner/repo" or "owner/repo@ref" (GitHub)
    let parts: Vec<&str> = trimmed.splitn(2, '/').collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        // Ensure it looks like a GitHub shorthand (no dots in owner, etc.)
        let owner = parts[0];
        let (repo_ref, ref_) = split_ref(parts[1]);
        if !owner.contains('.') && !repo_ref.is_empty() {
            let pinned = ref_.is_some();
            return Some(GitSource {
                repo: format!("https://github.com/{}/{}", owner, repo_ref),
                host: "github.com".to_owned(),
                path: format!("{}/{}", owner, repo_ref),
                ref_,
                pinned,
            });
        }
    }

    None
}

fn split_ref(s: &str) -> (String, Option<String>) {
    if let Some(pos) = s.find('@') {
        let path = s[..pos].to_owned();
        let ref_ = s[pos + 1..].to_owned();
        (path, Some(ref_))
    } else {
        (s.to_owned(), None)
    }
}

fn parse_source(source: &str) -> ParsedSource {
    if let Some(spec) = source.strip_prefix("npm:") {
        let spec = spec.trim().to_owned();
        let (name, version) = parse_npm_spec(&spec);
        return ParsedSource::Npm(NpmSource {
            spec,
            name,
            pinned: version.is_some(),
        });
    }

    let trimmed = source.trim();
    let is_local = trimmed.starts_with('.')
        || trimmed.starts_with('/')
        || trimmed == "~"
        || trimmed.starts_with("~/");

    if is_local {
        return ParsedSource::Local(LocalSource {
            path: source.to_owned(),
        });
    }

    if let Some(git) = parse_git_url(source) {
        return ParsedSource::Git(git);
    }

    ParsedSource::Local(LocalSource {
        path: source.to_owned(),
    })
}

// ============================================================================
// PackageManager
// ============================================================================

pub struct DefaultPackageManager {
    cwd: PathBuf,
    agent_dir: PathBuf,
    settings_manager: SettingsManager,
    global_npm_root: Option<String>,
    progress_callback: Option<ProgressCallback>,
}

impl DefaultPackageManager {
    pub fn new(
        cwd: impl Into<PathBuf>,
        agent_dir: impl Into<PathBuf>,
        settings_manager: SettingsManager,
    ) -> Self {
        Self {
            cwd: cwd.into(),
            agent_dir: agent_dir.into(),
            settings_manager,
            global_npm_root: None,
            progress_callback: None,
        }
    }

    pub fn set_progress_callback(&mut self, callback: Option<ProgressCallback>) {
        self.progress_callback = callback;
    }

    fn emit_progress(&self, event: ProgressEvent) {
        if let Some(cb) = &self.progress_callback {
            cb(event);
        }
    }

    fn scope_str(scope: &SourceScope) -> &'static str {
        match scope {
            SourceScope::User => "user",
            SourceScope::Project => "project",
            SourceScope::Temporary => "temporary",
        }
    }

    // ─── Settings helpers ─────────────────────────────────────────────────────

    pub fn add_source_to_settings(&mut self, source: &str, local: bool) -> bool {
        let scope = if local {
            SourceScope::Project
        } else {
            SourceScope::User
        };
        let current = if local {
            self.settings_manager.get_project_settings()
        } else {
            self.settings_manager.get_global_settings()
        };
        let current_packages = current.packages.unwrap_or_default();
        let exists = current_packages
            .iter()
            .any(|p| self.package_sources_match(p, source, &scope));
        if exists {
            return false;
        }
        let normalized = self.normalize_source_for_settings(source, &scope);
        let mut next = current_packages;
        next.push(PackageSource::Simple(normalized));
        if local {
            self.settings_manager.set_project_packages(next);
        } else {
            self.settings_manager.set_packages(next);
        }
        true
    }

    pub fn remove_source_from_settings(&mut self, source: &str, local: bool) -> bool {
        let scope = if local {
            SourceScope::Project
        } else {
            SourceScope::User
        };
        let current = if local {
            self.settings_manager.get_project_settings()
        } else {
            self.settings_manager.get_global_settings()
        };
        let current_packages = current.packages.unwrap_or_default();
        let next: Vec<_> = current_packages
            .into_iter()
            .filter(|p| !self.package_sources_match(p, source, &scope))
            .collect();
        let changed = next.len() != self.settings_manager.get_packages().len();
        if !changed {
            return false;
        }
        if local {
            self.settings_manager.set_project_packages(next);
        } else {
            self.settings_manager.set_packages(next);
        }
        true
    }

    pub fn get_installed_path(&self, source: &str, scope: &SourceScope) -> Option<PathBuf> {
        let parsed = parse_source(source);
        match &parsed {
            ParsedSource::Npm(npm) => {
                let path = self.npm_install_path(npm, scope);
                if path.exists() { Some(path) } else { None }
            }
            ParsedSource::Git(git) => {
                let path = self.git_install_path(git, scope);
                if path.exists() { Some(path) } else { None }
            }
            ParsedSource::Local(local) => {
                let base = self.base_dir_for_scope(scope);
                let path = self.resolve_path_from_base(&local.path, &base);
                if path.exists() { Some(path) } else { None }
            }
        }
    }

    // ─── Resolve ──────────────────────────────────────────────────────────────

    pub fn resolve(&self) -> anyhow::Result<ResolvedPaths> {
        let mut acc = ResourceAccumulator::new();
        let global = self.settings_manager.get_global_settings();
        let project = self.settings_manager.get_project_settings();

        // Build deduped package list (project wins over user for same identity)
        let mut all: Vec<(PackageSource, SourceScope)> = Vec::new();
        for pkg in project.packages.as_deref().unwrap_or(&[]) {
            all.push((pkg.clone(), SourceScope::Project));
        }
        for pkg in global.packages.as_deref().unwrap_or(&[]) {
            all.push((pkg.clone(), SourceScope::User));
        }
        let deduped = self.dedupe_packages(all);
        self.resolve_package_sources(&deduped, &mut acc)?;

        let global_base = self.agent_dir.clone();
        let project_base = self.cwd.join(CONFIG_DIR_NAME);

        for resource_type in [
            ResourceType::Extensions,
            ResourceType::Skills,
            ResourceType::Prompts,
            ResourceType::Themes,
        ] {
            let target = Self::get_target_map_mut(&mut acc, resource_type);
            let global_entries: Vec<String> = match resource_type {
                ResourceType::Extensions => global.extensions.clone().unwrap_or_default(),
                ResourceType::Skills => global.skills.clone().unwrap_or_default(),
                ResourceType::Prompts => global.prompts.clone().unwrap_or_default(),
                ResourceType::Themes => global.themes.clone().unwrap_or_default(),
            };
            let project_entries: Vec<String> = match resource_type {
                ResourceType::Extensions => project.extensions.clone().unwrap_or_default(),
                ResourceType::Skills => project.skills.clone().unwrap_or_default(),
                ResourceType::Prompts => project.prompts.clone().unwrap_or_default(),
                ResourceType::Themes => project.themes.clone().unwrap_or_default(),
            };
            self.resolve_local_entries(
                &project_entries,
                resource_type,
                target,
                &PathMetadata {
                    source: "local".into(),
                    scope: "project".into(),
                    origin: "top-level".into(),
                    base_dir: Some(project_base.to_string_lossy().into()),
                },
                &project_base,
            );
            self.resolve_local_entries(
                &global_entries,
                resource_type,
                target,
                &PathMetadata {
                    source: "local".into(),
                    scope: "user".into(),
                    origin: "top-level".into(),
                    base_dir: Some(global_base.to_string_lossy().into()),
                },
                &global_base,
            );
        }

        // Auto-discover resources from conventional directories.
        self.add_auto_discovered_resources(
            &mut acc,
            &global,
            &project,
            &global_base,
            &project_base,
        );

        Ok(self.to_resolved_paths(acc))
    }

    fn resolve_package_sources(
        &self,
        sources: &[(PackageSource, SourceScope)],
        acc: &mut ResourceAccumulator,
    ) -> anyhow::Result<()> {
        for (pkg, scope) in sources {
            let source_str = pkg.source().to_owned();
            let filter = match pkg {
                PackageSource::Filtered {
                    extensions,
                    skills,
                    prompts,
                    themes,
                    ..
                } => Some((
                    extensions.clone(),
                    skills.clone(),
                    prompts.clone(),
                    themes.clone(),
                )),
                _ => None,
            };
            let parsed = parse_source(&source_str);
            let metadata = PathMetadata {
                source: source_str.clone(),
                scope: Self::scope_str(scope).to_owned(),
                origin: "package".to_owned(),
                base_dir: None,
            };

            match &parsed {
                ParsedSource::Local(local) => {
                    let base = self.base_dir_for_scope(scope);
                    self.resolve_local_extension_source(
                        local,
                        acc,
                        filter.as_ref(),
                        &metadata,
                        &base,
                    );
                }
                ParsedSource::Npm(npm) => {
                    let install_path = self.npm_install_path(npm, scope);
                    if install_path.exists() {
                        let mut m = metadata.clone();
                        m.base_dir = Some(install_path.to_string_lossy().into());
                        self.collect_package_resources(&install_path, acc, filter.as_ref(), &m);
                    }
                }
                ParsedSource::Git(git) => {
                    let install_path = self.git_install_path(git, scope);
                    if install_path.exists() {
                        let mut m = metadata.clone();
                        m.base_dir = Some(install_path.to_string_lossy().into());
                        self.collect_package_resources(&install_path, acc, filter.as_ref(), &m);
                    }
                }
            }
        }
        Ok(())
    }

    // ─── Install / Remove / Update ────────────────────────────────────────────

    pub fn install(&mut self, source: &str, local: bool) -> anyhow::Result<()> {
        let parsed = parse_source(source);
        let scope = if local {
            SourceScope::Project
        } else {
            SourceScope::User
        };
        self.emit_progress(ProgressEvent {
            kind: "start".into(),
            action: "install".into(),
            source: source.to_owned(),
            message: Some(format!("Installing {}...", source)),
        });
        let result = match &parsed {
            ParsedSource::Npm(npm) => self.install_npm(npm, &scope, false),
            ParsedSource::Git(git) => self.install_git(git, &scope),
            ParsedSource::Local(local_src) => {
                let resolved = self.resolve_path(&local_src.path);
                if !resolved.exists() {
                    Err(anyhow::anyhow!(
                        "Path does not exist: {}",
                        resolved.display()
                    ))
                } else {
                    Ok(())
                }
            }
        };
        match result {
            Ok(()) => {
                self.emit_progress(ProgressEvent {
                    kind: "complete".into(),
                    action: "install".into(),
                    source: source.to_owned(),
                    message: None,
                });
                Ok(())
            }
            Err(e) => {
                self.emit_progress(ProgressEvent {
                    kind: "error".into(),
                    action: "install".into(),
                    source: source.to_owned(),
                    message: Some(e.to_string()),
                });
                Err(e)
            }
        }
    }

    pub fn remove(&mut self, source: &str, local: bool) -> anyhow::Result<()> {
        let parsed = parse_source(source);
        let scope = if local {
            SourceScope::Project
        } else {
            SourceScope::User
        };
        self.emit_progress(ProgressEvent {
            kind: "start".into(),
            action: "remove".into(),
            source: source.to_owned(),
            message: Some(format!("Removing {}...", source)),
        });
        let result = match &parsed {
            ParsedSource::Npm(npm) => self.uninstall_npm(npm, &scope),
            ParsedSource::Git(git) => self.remove_git(git, &scope),
            ParsedSource::Local(_) => Ok(()),
        };
        match result {
            Ok(()) => {
                self.emit_progress(ProgressEvent {
                    kind: "complete".into(),
                    action: "remove".into(),
                    source: source.to_owned(),
                    message: None,
                });
                Ok(())
            }
            Err(e) => {
                self.emit_progress(ProgressEvent {
                    kind: "error".into(),
                    action: "remove".into(),
                    source: source.to_owned(),
                    message: Some(e.to_string()),
                });
                Err(e)
            }
        }
    }

    // ─── npm helpers ──────────────────────────────────────────────────────────

    fn npm_command(&self) -> (String, Vec<String>) {
        let configured = self.settings_manager.get_npm_command();
        match configured.as_deref() {
            Some([cmd, rest @ ..]) if !cmd.is_empty() => (cmd.to_owned(), rest.to_vec()),
            _ => ("npm".to_owned(), vec![]),
        }
    }

    fn run_npm_command(&self, args: &[&str], cwd: Option<&Path>) -> anyhow::Result<()> {
        let (cmd, prefix_args) = self.npm_command();
        let all_args: Vec<&str> = prefix_args
            .iter()
            .map(|s| s.as_str())
            .chain(args.iter().copied())
            .collect();
        let mut command = Command::new(&cmd);
        command
            .args(&all_args)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let status = command.status()?;
        if !status.success() {
            return Err(anyhow::anyhow!(
                "{} {} failed: {:?}",
                cmd,
                args.join(" "),
                status
            ));
        }
        Ok(())
    }

    fn run_npm_command_capture(&self, args: &[&str]) -> anyhow::Result<String> {
        let (cmd, prefix_args) = self.npm_command();
        let all_args: Vec<&str> = prefix_args
            .iter()
            .map(|s| s.as_str())
            .chain(args.iter().copied())
            .collect();
        let output = Command::new(&cmd)
            .args(&all_args)
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "{} {} failed: {}",
                cmd,
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn install_npm(
        &self,
        source: &NpmSource,
        scope: &SourceScope,
        temporary: bool,
    ) -> anyhow::Result<()> {
        if *scope == SourceScope::User && !temporary {
            return self.run_npm_command(&["install", "-g", &source.spec], None);
        }
        let install_root = self.npm_install_root(scope, temporary);
        self.ensure_npm_project(&install_root)?;
        self.run_npm_command(
            &[
                "install",
                &source.spec,
                "--prefix",
                &install_root.to_string_lossy(),
            ],
            None,
        )
    }

    fn uninstall_npm(&self, source: &NpmSource, scope: &SourceScope) -> anyhow::Result<()> {
        if *scope == SourceScope::User {
            return self.run_npm_command(&["uninstall", "-g", &source.name], None);
        }
        let install_root = self.npm_install_root(scope, false);
        if !install_root.exists() {
            return Ok(());
        }
        self.run_npm_command(
            &[
                "uninstall",
                &source.name,
                "--prefix",
                &install_root.to_string_lossy(),
            ],
            None,
        )
    }

    fn npm_install_root(&self, scope: &SourceScope, temporary: bool) -> PathBuf {
        if temporary {
            self.temp_dir("npm", None)
        } else if *scope == SourceScope::Project {
            self.cwd.join(CONFIG_DIR_NAME).join("npm")
        } else {
            // For user scope, install next to global npm root.
            let root = self.global_npm_root();
            PathBuf::from(&root)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(&root))
        }
    }

    fn global_npm_root(&self) -> String {
        if let Some(ref root) = self.global_npm_root {
            return root.clone();
        }
        self.run_npm_command_capture(&["root", "-g"])
            .unwrap_or_else(|_| "/usr/local/lib/node_modules".to_owned())
    }

    fn npm_install_path(&self, source: &NpmSource, scope: &SourceScope) -> PathBuf {
        match scope {
            SourceScope::Temporary => self
                .temp_dir("npm", None)
                .join("node_modules")
                .join(&source.name),
            SourceScope::Project => self
                .cwd
                .join(CONFIG_DIR_NAME)
                .join("npm")
                .join("node_modules")
                .join(&source.name),
            SourceScope::User => PathBuf::from(self.global_npm_root()).join(&source.name),
        }
    }

    fn ensure_npm_project(&self, install_root: &Path) -> anyhow::Result<()> {
        if !install_root.exists() {
            std::fs::create_dir_all(install_root)?;
        }
        self.ensure_gitignore(install_root)?;
        let pkg_path = install_root.join("package.json");
        if !pkg_path.exists() {
            let pkg = serde_json::json!({"name": "sage-extensions", "private": true});
            std::fs::write(&pkg_path, serde_json::to_string_pretty(&pkg)?)?;
        }
        Ok(())
    }

    fn ensure_gitignore(&self, dir: &Path) -> anyhow::Result<()> {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }
        let ignore_path = dir.join(".gitignore");
        if !ignore_path.exists() {
            std::fs::write(&ignore_path, "*\n!.gitignore\n")?;
        }
        Ok(())
    }

    // ─── git helpers ──────────────────────────────────────────────────────────

    fn install_git(&self, source: &GitSource, scope: &SourceScope) -> anyhow::Result<()> {
        let target_dir = self.git_install_path(source, scope);
        if target_dir.exists() {
            return Ok(());
        }
        if let Some(git_root) = self.git_install_root(scope) {
            let _ = self.ensure_gitignore(&git_root);
        }
        if let Some(parent) = target_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let status = Command::new("git")
            .args(["clone", &source.repo, &target_dir.to_string_lossy()])
            .status()?;
        if !status.success() {
            return Err(anyhow::anyhow!("git clone failed"));
        }
        if let Some(ref ref_) = source.ref_ {
            let status = Command::new("git")
                .args(["checkout", ref_])
                .current_dir(&target_dir)
                .status()?;
            if !status.success() {
                return Err(anyhow::anyhow!("git checkout failed"));
            }
        }
        // npm install if package.json exists
        if target_dir.join("package.json").exists() {
            self.run_npm_command(&["install"], Some(&target_dir))?;
        }
        Ok(())
    }

    fn remove_git(&self, source: &GitSource, scope: &SourceScope) -> anyhow::Result<()> {
        let target_dir = self.git_install_path(source, scope);
        if !target_dir.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(&target_dir)?;
        self.prune_empty_git_parents(&target_dir, self.git_install_root(scope).as_deref());
        Ok(())
    }

    fn prune_empty_git_parents(&self, target_dir: &Path, install_root: Option<&Path>) {
        let Some(root) = install_root else { return };
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut current = target_dir.parent().map(|p| p.to_path_buf());
        while let Some(dir) = current {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            if !canonical.starts_with(&root) || canonical == root {
                break;
            }
            let entries = std::fs::read_dir(&dir).map(|r| r.count()).unwrap_or(1);
            if entries > 0 {
                break;
            }
            let _ = std::fs::remove_dir(&dir);
            current = dir.parent().map(|p| p.to_path_buf());
        }
    }

    fn git_install_path(&self, source: &GitSource, scope: &SourceScope) -> PathBuf {
        match scope {
            SourceScope::Temporary => {
                self.temp_dir(&format!("git-{}", source.host), Some(&source.path))
            }
            SourceScope::Project => self
                .cwd
                .join(CONFIG_DIR_NAME)
                .join("git")
                .join(&source.host)
                .join(&source.path),
            SourceScope::User => self
                .agent_dir
                .join("git")
                .join(&source.host)
                .join(&source.path),
        }
    }

    fn git_install_root(&self, scope: &SourceScope) -> Option<PathBuf> {
        match scope {
            SourceScope::Temporary => None,
            SourceScope::Project => Some(self.cwd.join(CONFIG_DIR_NAME).join("git")),
            SourceScope::User => Some(self.agent_dir.join("git")),
        }
    }

    // ─── Path helpers ─────────────────────────────────────────────────────────

    fn temp_dir(&self, prefix: &str, suffix: Option<&str>) -> PathBuf {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        format!("{}-{}", prefix, suffix.unwrap_or("")).hash(&mut h);
        let hash = format!("{:016x}", h.finish());
        let dir = std::env::temp_dir()
            .join("sage-extensions")
            .join(prefix)
            .join(&hash[..8]);
        if let Some(suf) = suffix {
            dir.join(suf)
        } else {
            dir
        }
    }

    fn base_dir_for_scope(&self, scope: &SourceScope) -> PathBuf {
        match scope {
            SourceScope::Project => self.cwd.join(CONFIG_DIR_NAME),
            SourceScope::User => self.agent_dir.clone(),
            SourceScope::Temporary => self.cwd.clone(),
        }
    }

    fn resolve_path(&self, input: &str) -> PathBuf {
        let trimmed = input.trim();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        if trimmed == "~" {
            return home;
        }
        if let Some(rest) = trimmed.strip_prefix("~/") {
            return home.join(rest);
        }
        self.cwd.join(trimmed)
    }

    fn resolve_path_from_base(&self, input: &str, base: &Path) -> PathBuf {
        let trimmed = input.trim();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        if trimmed == "~" {
            return home;
        }
        if let Some(rest) = trimmed.strip_prefix("~/") {
            return home.join(rest);
        }
        base.join(trimmed)
    }

    // ─── Resource collection ──────────────────────────────────────────────────

    #[allow(clippy::type_complexity)]
    fn collect_package_resources(
        &self,
        package_root: &Path,
        acc: &mut ResourceAccumulator,
        filter: Option<&(
            Option<Vec<String>>,
            Option<Vec<String>>,
            Option<Vec<String>>,
            Option<Vec<String>>,
        )>,
        metadata: &PathMetadata,
    ) -> bool {
        if let Some(f) = filter {
            for resource_type in [
                ResourceType::Extensions,
                ResourceType::Skills,
                ResourceType::Prompts,
                ResourceType::Themes,
            ] {
                let patterns = match resource_type {
                    ResourceType::Extensions => f.0.as_deref(),
                    ResourceType::Skills => f.1.as_deref(),
                    ResourceType::Prompts => f.2.as_deref(),
                    ResourceType::Themes => f.3.as_deref(),
                };
                let target = Self::get_target_map_mut(acc, resource_type);
                if let Some(pats) = patterns {
                    let dir = package_root.join(resource_type_dir_name(resource_type));
                    let all_files = collect_resource_files(&dir, resource_type);
                    if pats.is_empty() {
                        for f in &all_files {
                            Self::add_resource(target, f, metadata, false);
                        }
                    } else {
                        for f in &all_files {
                            Self::add_resource(target, f, metadata, true);
                        }
                    }
                } else {
                    let dir = package_root.join(resource_type_dir_name(resource_type));
                    let files = collect_resource_files(&dir, resource_type);
                    for f in &files {
                        Self::add_resource(target, f, metadata, true);
                    }
                }
            }
            return true;
        }

        // Check for pi manifest in package.json
        if let Some(manifest) = self.read_pi_manifest(package_root) {
            for resource_type in [
                ResourceType::Extensions,
                ResourceType::Skills,
                ResourceType::Prompts,
                ResourceType::Themes,
            ] {
                let entries = match resource_type {
                    ResourceType::Extensions => manifest.extensions.as_deref(),
                    ResourceType::Skills => manifest.skills.as_deref(),
                    ResourceType::Prompts => manifest.prompts.as_deref(),
                    ResourceType::Themes => manifest.themes.as_deref(),
                };
                if let Some(e) = entries {
                    let target = Self::get_target_map_mut(acc, resource_type);
                    for entry in e {
                        let resolved = package_root.join(entry);
                        if resolved.exists() {
                            Self::add_resource(target, &resolved, metadata, true);
                        }
                    }
                }
            }
            return true;
        }

        // Convention-based discovery
        let mut found_any = false;
        for resource_type in [
            ResourceType::Extensions,
            ResourceType::Skills,
            ResourceType::Prompts,
            ResourceType::Themes,
        ] {
            let dir = package_root.join(resource_type_dir_name(resource_type));
            if dir.exists() {
                let files = collect_resource_files(&dir, resource_type);
                let target = Self::get_target_map_mut(acc, resource_type);
                for f in &files {
                    Self::add_resource(target, f, metadata, true);
                }
                found_any = true;
            }
        }
        found_any
    }

    #[allow(clippy::type_complexity)]
    fn resolve_local_extension_source(
        &self,
        source: &LocalSource,
        acc: &mut ResourceAccumulator,
        filter: Option<&(
            Option<Vec<String>>,
            Option<Vec<String>>,
            Option<Vec<String>>,
            Option<Vec<String>>,
        )>,
        metadata: &PathMetadata,
        base: &Path,
    ) {
        let resolved = self.resolve_path_from_base(&source.path, base);
        if !resolved.exists() {
            return;
        }
        let Ok(stat) = std::fs::metadata(&resolved) else {
            return;
        };
        if stat.is_file() {
            let mut m = metadata.clone();
            m.base_dir = resolved.parent().map(|p| p.to_string_lossy().into());
            Self::add_resource(&mut acc.extensions, &resolved, &m, true);
        } else if stat.is_dir() {
            let mut m = metadata.clone();
            m.base_dir = Some(resolved.to_string_lossy().into());
            let found = self.collect_package_resources(&resolved, acc, filter, &m);
            if !found {
                Self::add_resource(&mut acc.extensions, &resolved, &m, true);
            }
        }
    }

    fn resolve_local_entries(
        &self,
        entries: &[String],
        resource_type: ResourceType,
        target: &mut HashMap<PathBuf, (PathMetadata, bool)>,
        metadata: &PathMetadata,
        base_dir: &Path,
    ) {
        if entries.is_empty() {
            return;
        }
        for entry in entries {
            let resolved = self.resolve_path_from_base(entry, base_dir);
            if !resolved.exists() {
                continue;
            }
            let Ok(stat) = std::fs::metadata(&resolved) else {
                continue;
            };
            if stat.is_file() {
                Self::add_resource(target, &resolved, metadata, true);
            } else if stat.is_dir() {
                let files = collect_resource_files(&resolved, resource_type);
                for f in files {
                    Self::add_resource(target, &f, metadata, true);
                }
            }
        }
    }

    fn add_auto_discovered_resources(
        &self,
        acc: &mut ResourceAccumulator,
        _global: &crate::core::settings_manager::Settings,
        _project: &crate::core::settings_manager::Settings,
        global_base: &Path,
        project_base: &Path,
    ) {
        let user_metadata = PathMetadata {
            source: "auto".into(),
            scope: "user".into(),
            origin: "top-level".into(),
            base_dir: Some(global_base.to_string_lossy().into()),
        };
        let project_metadata = PathMetadata {
            source: "auto".into(),
            scope: "project".into(),
            origin: "top-level".into(),
            base_dir: Some(project_base.to_string_lossy().into()),
        };

        for resource_type in [
            ResourceType::Extensions,
            ResourceType::Skills,
            ResourceType::Prompts,
            ResourceType::Themes,
        ] {
            let dir_name = resource_type_dir_name(resource_type);
            let project_dir = project_base.join(dir_name);
            let user_dir = global_base.join(dir_name);

            let project_files = collect_resource_files(&project_dir, resource_type);
            let target = Self::get_target_map_mut(acc, resource_type);
            for f in project_files {
                Self::add_resource(target, &f, &project_metadata, true);
            }
            let user_files = collect_resource_files(&user_dir, resource_type);
            for f in user_files {
                Self::add_resource(target, &f, &user_metadata, true);
            }
        }

        // Auto-discover from ~/.agents/skills
        if let Some(home) = dirs::home_dir() {
            let agents_skills = home.join(".agents").join("skills");
            if agents_skills.exists() {
                let files = collect_skill_entries(&agents_skills, true);
                let target = Self::get_target_map_mut(acc, ResourceType::Skills);
                for f in files {
                    Self::add_resource(target, &f, &user_metadata, true);
                }
            }
        }
    }

    fn read_pi_manifest(&self, root: &Path) -> Option<PiManifest> {
        let pkg_path = root.join("package.json");
        if !pkg_path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&pkg_path).ok()?;
        let val: serde_json::Value = serde_json::from_str(&content).ok()?;
        let pi = val.get("pi")?;
        serde_json::from_value(pi.clone()).ok()
    }

    fn get_target_map_mut(
        acc: &mut ResourceAccumulator,
        resource_type: ResourceType,
    ) -> &mut HashMap<PathBuf, (PathMetadata, bool)> {
        match resource_type {
            ResourceType::Extensions => &mut acc.extensions,
            ResourceType::Skills => &mut acc.skills,
            ResourceType::Prompts => &mut acc.prompts,
            ResourceType::Themes => &mut acc.themes,
        }
    }

    fn add_resource(
        map: &mut HashMap<PathBuf, (PathMetadata, bool)>,
        path: &Path,
        metadata: &PathMetadata,
        enabled: bool,
    ) {
        if !map.contains_key(path) {
            map.insert(path.to_path_buf(), (metadata.clone(), enabled));
        }
    }

    fn to_resolved_paths(&self, acc: ResourceAccumulator) -> ResolvedPaths {
        let to_vec = |map: HashMap<PathBuf, (PathMetadata, bool)>| -> Vec<ResolvedResource> {
            map.into_iter()
                .map(|(path, (metadata, enabled))| ResolvedResource {
                    path,
                    enabled,
                    metadata,
                })
                .collect()
        };
        ResolvedPaths {
            extensions: to_vec(acc.extensions),
            skills: to_vec(acc.skills),
            prompts: to_vec(acc.prompts),
            themes: to_vec(acc.themes),
        }
    }

    // ─── Deduplication ────────────────────────────────────────────────────────

    fn dedupe_packages(
        &self,
        packages: Vec<(PackageSource, SourceScope)>,
    ) -> Vec<(PackageSource, SourceScope)> {
        let mut seen: HashMap<String, (PackageSource, SourceScope)> = HashMap::new();
        for (pkg, scope) in packages {
            let source_str = pkg.source().to_owned();
            let identity = self.package_identity(&source_str, Some(&scope));
            let entry = seen.entry(identity);
            match entry {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert((pkg, scope));
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    if scope == SourceScope::Project && e.get().1 == SourceScope::User {
                        e.insert((pkg, scope));
                    }
                }
            }
        }
        seen.into_values().collect()
    }

    fn package_identity(&self, source: &str, scope: Option<&SourceScope>) -> String {
        let parsed = parse_source(source);
        match &parsed {
            ParsedSource::Npm(npm) => format!("npm:{}", npm.name),
            ParsedSource::Git(git) => format!("git:{}/{}", git.host, git.path),
            ParsedSource::Local(local) => {
                let path = if let Some(s) = scope {
                    let base = self.base_dir_for_scope(s);
                    self.resolve_path_from_base(&local.path, &base)
                } else {
                    self.resolve_path(&local.path)
                };
                format!("local:{}", path.to_string_lossy())
            }
        }
    }

    fn package_sources_match(
        &self,
        existing: &PackageSource,
        input: &str,
        scope: &SourceScope,
    ) -> bool {
        let left = self.package_identity(existing.source(), Some(scope));
        let right = self.package_identity(input, Some(scope));
        left == right
    }

    fn normalize_source_for_settings(&self, source: &str, scope: &SourceScope) -> String {
        let parsed = parse_source(source);
        if let ParsedSource::Local(local) = &parsed {
            let base = self.base_dir_for_scope(scope);
            let resolved = self.resolve_path(&local.path);
            let rel = pathdiff::diff_paths(&resolved, &base)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| resolved.to_string_lossy().into_owned());
            return if rel.is_empty() { ".".to_owned() } else { rel };
        }
        source.to_owned()
    }
}

fn resource_type_dir_name(rt: ResourceType) -> &'static str {
    match rt {
        ResourceType::Extensions => "extensions",
        ResourceType::Skills => "skills",
        ResourceType::Prompts => "prompts",
        ResourceType::Themes => "themes",
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::settings_manager::SettingsManager;
    use tempfile::TempDir;

    fn make_pm(tmp: &TempDir) -> DefaultPackageManager {
        let cwd = tmp.path().join("project");
        let agent_dir = tmp.path().join("agent");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&agent_dir).unwrap();
        let mgr = SettingsManager::create(&cwd, &agent_dir);
        DefaultPackageManager::new(cwd, agent_dir, mgr)
    }

    #[test]
    fn parse_npm_source() {
        let s = parse_source("npm:my-pkg@1.0.0");
        match s {
            ParsedSource::Npm(npm) => {
                assert_eq!(npm.name, "my-pkg");
                assert!(npm.pinned);
            }
            _ => panic!("expected npm"),
        }
    }

    #[test]
    fn parse_local_source_relative() {
        let s = parse_source("./my-ext");
        match s {
            ParsedSource::Local(l) => assert_eq!(l.path, "./my-ext"),
            _ => panic!("expected local"),
        }
    }

    #[test]
    fn parse_git_shorthand() {
        let s = parse_source("owner/repo");
        match s {
            ParsedSource::Git(git) => {
                assert_eq!(git.host, "github.com");
                assert!(git.path.contains("owner"));
            }
            _ => panic!("expected git"),
        }
    }

    #[test]
    fn parse_npm_scoped_package() {
        let s = parse_source("npm:@scope/pkg");
        match s {
            ParsedSource::Npm(npm) => {
                assert_eq!(npm.name, "@scope/pkg");
                assert!(!npm.pinned);
            }
            _ => panic!("expected npm"),
        }
    }

    #[test]
    fn resolve_returns_empty_for_no_packages() {
        let tmp = TempDir::new().unwrap();
        // Override HOME so that dirs::home_dir() doesn't pick up real ~/.agents/skills
        let fake_home = tmp.path().join("home");
        std::fs::create_dir_all(&fake_home).unwrap();
        // SAFETY: single-threaded context for this test; HOME change is scoped.
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", &fake_home) };
        let pm = make_pm(&tmp);
        let result = pm.resolve().unwrap();
        // Restore HOME
        match orig_home {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        assert!(result.extensions.is_empty());
        assert!(result.skills.is_empty());
    }

    #[test]
    fn add_source_to_settings_deduplication() {
        let tmp = TempDir::new().unwrap();
        let mut pm = make_pm(&tmp);
        let added = pm.add_source_to_settings("npm:my-pkg", false);
        assert!(added);
        let added_again = pm.add_source_to_settings("npm:my-pkg", false);
        assert!(!added_again);
    }

    #[test]
    fn remove_source_from_settings() {
        let tmp = TempDir::new().unwrap();
        let mut pm = make_pm(&tmp);
        pm.add_source_to_settings("npm:my-pkg", false);
        let removed = pm.remove_source_from_settings("npm:my-pkg", false);
        assert!(removed);
        let packages = pm.settings_manager.get_packages();
        assert!(packages.is_empty());
    }

    #[test]
    fn local_source_resolves_path() {
        let tmp = TempDir::new().unwrap();
        let pm = make_pm(&tmp);
        let path = pm.get_installed_path("./nonexistent", &SourceScope::User);
        assert!(path.is_none());
    }

    #[test]
    fn progress_callback_called_on_install_error() {
        let tmp = TempDir::new().unwrap();
        let mut pm = make_pm(&tmp);
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let events_clone = events.clone();
        pm.set_progress_callback(Some(Box::new(move |e: ProgressEvent| {
            events_clone.lock().unwrap().push(e.kind.clone());
        })));
        let _ = pm.install("./nonexistent-path", false);
        let events = events.lock().unwrap();
        assert!(events.contains(&"start".to_string()));
    }
}

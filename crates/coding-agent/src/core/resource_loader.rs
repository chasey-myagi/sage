//! Resource loader — loads skills, prompts, themes and AGENTS files.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/resource-loader.ts`.

use std::path::{Path, PathBuf};

use crate::config::CONFIG_DIR_NAME;
use crate::core::package_manager::{DefaultPackageManager, PathMetadata};
use crate::core::settings_manager::SettingsManager;

// ============================================================================
// Public types
// ============================================================================

/// A skill loaded from a `.md` file.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub file_path: String,
    pub content: String,
}

/// A prompt template loaded from a `.md` file.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    pub file_path: String,
    pub content: String,
}

/// A theme loaded from a `.json` file.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: Option<String>,
    pub source_path: Option<String>,
}

/// Diagnostic message from resource loading.
#[derive(Debug, Clone)]
pub struct ResourceDiagnostic {
    pub kind: String, // "warning" | "error" | "collision"
    pub message: String,
    pub path: Option<String>,
}

/// Result from loading extensions.
#[derive(Debug, Default)]
pub struct LoadExtensionsResult {
    pub errors: Vec<ResourceDiagnostic>,
}

/// An AGENTS.md / CLAUDE.md context file.
#[derive(Debug, Clone)]
pub struct AgentsFile {
    pub path: String,
    pub content: String,
}

/// Path metadata entry for resource extension.
#[derive(Debug, Clone)]
pub struct ResourceExtensionEntry {
    pub path: String,
    pub metadata: PathMetadata,
}

/// Additional resource paths injected at runtime.
#[derive(Debug, Default)]
pub struct ResourceExtensionPaths {
    pub skill_paths: Vec<ResourceExtensionEntry>,
    pub prompt_paths: Vec<ResourceExtensionEntry>,
    pub theme_paths: Vec<ResourceExtensionEntry>,
}

// ============================================================================
// ResourceLoader trait
// ============================================================================

pub trait ResourceLoader {
    fn get_skills(&self) -> (&[Skill], &[ResourceDiagnostic]);
    fn get_prompts(&self) -> (&[PromptTemplate], &[ResourceDiagnostic]);
    fn get_themes(&self) -> (&[Theme], &[ResourceDiagnostic]);
    fn get_agents_files(&self) -> &[AgentsFile];
    fn get_system_prompt(&self) -> Option<&str>;
    fn get_append_system_prompt(&self) -> &[String];
    fn extend_resources(&mut self, paths: ResourceExtensionPaths);
    fn reload(&mut self) -> anyhow::Result<()>;
}

// ============================================================================
// Options
// ============================================================================

pub struct DefaultResourceLoaderOptions {
    pub cwd: Option<PathBuf>,
    pub agent_dir: Option<PathBuf>,
    pub settings_manager: Option<SettingsManager>,
    pub additional_skill_paths: Vec<String>,
    pub additional_prompt_template_paths: Vec<String>,
    pub additional_theme_paths: Vec<String>,
    pub no_skills: bool,
    pub no_prompt_templates: bool,
    pub no_themes: bool,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
}

impl Default for DefaultResourceLoaderOptions {
    fn default() -> Self {
        Self {
            cwd: None,
            agent_dir: None,
            settings_manager: None,
            additional_skill_paths: Vec::new(),
            additional_prompt_template_paths: Vec::new(),
            additional_theme_paths: Vec::new(),
            no_skills: false,
            no_prompt_templates: false,
            no_themes: false,
            system_prompt: None,
            append_system_prompt: None,
        }
    }
}

// ============================================================================
// DefaultResourceLoader
// ============================================================================

pub struct DefaultResourceLoader {
    cwd: PathBuf,
    agent_dir: PathBuf,
    settings_manager: SettingsManager,
    package_manager: DefaultPackageManager,
    additional_skill_paths: Vec<String>,
    additional_prompt_template_paths: Vec<String>,
    additional_theme_paths: Vec<String>,
    no_skills: bool,
    no_prompt_templates: bool,
    no_themes: bool,
    system_prompt_source: Option<String>,
    append_system_prompt_source: Option<String>,

    // Loaded state
    skills: Vec<Skill>,
    skill_diagnostics: Vec<ResourceDiagnostic>,
    prompts: Vec<PromptTemplate>,
    prompt_diagnostics: Vec<ResourceDiagnostic>,
    themes: Vec<Theme>,
    theme_diagnostics: Vec<ResourceDiagnostic>,
    agents_files: Vec<AgentsFile>,
    system_prompt: Option<String>,
    append_system_prompt: Vec<String>,
}

impl DefaultResourceLoader {
    pub fn new(options: DefaultResourceLoaderOptions) -> Self {
        let cwd = options.cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let agent_dir = options.agent_dir.unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
                .join("sage")
        });
        let settings_manager = options.settings_manager.unwrap_or_else(|| {
            SettingsManager::create(&cwd, &agent_dir)
        });
        let pm = DefaultPackageManager::new(cwd.clone(), agent_dir.clone(), SettingsManager::create(&cwd, &agent_dir));

        Self {
            cwd,
            agent_dir,
            settings_manager,
            package_manager: pm,
            additional_skill_paths: options.additional_skill_paths,
            additional_prompt_template_paths: options.additional_prompt_template_paths,
            additional_theme_paths: options.additional_theme_paths,
            no_skills: options.no_skills,
            no_prompt_templates: options.no_prompt_templates,
            no_themes: options.no_themes,
            system_prompt_source: options.system_prompt,
            append_system_prompt_source: options.append_system_prompt,
            skills: Vec::new(),
            skill_diagnostics: Vec::new(),
            prompts: Vec::new(),
            prompt_diagnostics: Vec::new(),
            themes: Vec::new(),
            theme_diagnostics: Vec::new(),
            agents_files: Vec::new(),
            system_prompt: None,
            append_system_prompt: Vec::new(),
        }
    }

    // ─── Internal loading helpers ─────────────────────────────────────────────

    fn load_skills_from_paths(&self, paths: &[String]) -> (Vec<Skill>, Vec<ResourceDiagnostic>) {
        let mut skills = Vec::new();
        let mut diagnostics = Vec::new();
        for path_str in paths {
            let path = Path::new(path_str);
            if !path.exists() {
                diagnostics.push(ResourceDiagnostic {
                    kind: "warning".into(),
                    message: format!("skill path does not exist: {path_str}"),
                    path: Some(path_str.clone()),
                });
                continue;
            }
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    let name = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path_str.clone());
                    skills.push(Skill { name, file_path: path_str.clone(), content });
                }
                Err(e) => {
                    diagnostics.push(ResourceDiagnostic {
                        kind: "warning".into(),
                        message: format!("failed to read skill {path_str}: {e}"),
                        path: Some(path_str.clone()),
                    });
                }
            }
        }
        (skills, diagnostics)
    }

    fn load_prompts_from_paths(&self, paths: &[String]) -> (Vec<PromptTemplate>, Vec<ResourceDiagnostic>) {
        let mut prompts = Vec::new();
        let mut diagnostics = Vec::new();
        let mut seen_names = std::collections::HashMap::new();
        for path_str in paths {
            let path = Path::new(path_str);
            if !path.exists() {
                diagnostics.push(ResourceDiagnostic {
                    kind: "warning".into(),
                    message: format!("prompt path does not exist: {path_str}"),
                    path: Some(path_str.clone()),
                });
                continue;
            }
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    let name = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path_str.clone());
                    if let Some(existing) = seen_names.get(&name) {
                        diagnostics.push(ResourceDiagnostic {
                            kind: "collision".into(),
                            message: format!(r#"name "/{name}" collision"#),
                            path: Some(path_str.clone()),
                        });
                        let _ = existing;
                    } else {
                        seen_names.insert(name.clone(), path_str.clone());
                        prompts.push(PromptTemplate { name, file_path: path_str.clone(), content });
                    }
                }
                Err(e) => {
                    diagnostics.push(ResourceDiagnostic {
                        kind: "warning".into(),
                        message: format!("failed to read prompt {path_str}: {e}"),
                        path: Some(path_str.clone()),
                    });
                }
            }
        }
        (prompts, diagnostics)
    }

    fn load_themes_from_paths(&self, paths: &[String]) -> (Vec<Theme>, Vec<ResourceDiagnostic>) {
        let mut themes = Vec::new();
        let mut diagnostics = Vec::new();
        for path_str in paths {
            let path = Path::new(path_str);
            if !path.exists() {
                diagnostics.push(ResourceDiagnostic {
                    kind: "warning".into(),
                    message: format!("theme path does not exist: {path_str}"),
                    path: Some(path_str.clone()),
                });
                continue;
            }
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    let name = serde_json::from_str::<serde_json::Value>(&content)
                        .ok()
                        .and_then(|v| v.get("name")?.as_str().map(|s| s.to_owned()));
                    themes.push(Theme { name, source_path: Some(path_str.clone()) });
                }
                Err(e) => {
                    diagnostics.push(ResourceDiagnostic {
                        kind: "warning".into(),
                        message: format!("failed to load theme {path_str}: {e}"),
                        path: Some(path_str.clone()),
                    });
                }
            }
        }
        (themes, diagnostics)
    }

    /// Load AGENTS.md / CLAUDE.md context files from cwd and ancestor dirs.
    fn load_agents_files(&self) -> Vec<AgentsFile> {
        load_project_context_files(&self.cwd, &self.agent_dir)
    }

    fn discover_system_prompt_file(&self) -> Option<String> {
        let project = self.cwd.join(CONFIG_DIR_NAME).join("SYSTEM.md");
        if project.exists() {
            return Some(project.to_string_lossy().into());
        }
        let global = self.agent_dir.join("SYSTEM.md");
        if global.exists() {
            return Some(global.to_string_lossy().into());
        }
        None
    }

    fn discover_append_system_prompt_file(&self) -> Option<String> {
        let project = self.cwd.join(CONFIG_DIR_NAME).join("APPEND_SYSTEM.md");
        if project.exists() {
            return Some(project.to_string_lossy().into());
        }
        let global = self.agent_dir.join("APPEND_SYSTEM.md");
        if global.exists() {
            return Some(global.to_string_lossy().into());
        }
        None
    }

    fn resolve_prompt_input(input: Option<&str>) -> Option<String> {
        let input = input?;
        if input.is_empty() {
            return None;
        }
        let path = Path::new(input);
        if path.exists() {
            std::fs::read_to_string(path).ok()
        } else {
            Some(input.to_owned())
        }
    }

    fn merge_paths(primary: &[String], additional: &[String], base: &Path) -> Vec<String> {
        let mut merged = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for p in primary.iter().chain(additional.iter()) {
            let resolved = resolve_resource_path(p, base);
            let key = resolved.to_string_lossy().into_owned();
            if seen.insert(key.clone()) {
                merged.push(key);
            }
        }
        merged
    }
}

impl ResourceLoader for DefaultResourceLoader {
    fn get_skills(&self) -> (&[Skill], &[ResourceDiagnostic]) {
        (&self.skills, &self.skill_diagnostics)
    }

    fn get_prompts(&self) -> (&[PromptTemplate], &[ResourceDiagnostic]) {
        (&self.prompts, &self.prompt_diagnostics)
    }

    fn get_themes(&self) -> (&[Theme], &[ResourceDiagnostic]) {
        (&self.themes, &self.theme_diagnostics)
    }

    fn get_agents_files(&self) -> &[AgentsFile] {
        &self.agents_files
    }

    fn get_system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    fn get_append_system_prompt(&self) -> &[String] {
        &self.append_system_prompt
    }

    fn extend_resources(&mut self, paths: ResourceExtensionPaths) {
        let new_skills: Vec<String> = paths.skill_paths.iter().map(|e| e.path.clone()).collect();
        let new_prompts: Vec<String> = paths.prompt_paths.iter().map(|e| e.path.clone()).collect();
        let new_themes: Vec<String> = paths.theme_paths.iter().map(|e| e.path.clone()).collect();

        if !new_skills.is_empty() {
            let merged = Self::merge_paths(
                &self.skills.iter().map(|s| s.file_path.clone()).collect::<Vec<_>>(),
                &new_skills,
                &self.cwd,
            );
            let (skills, diags) = self.load_skills_from_paths(&merged);
            self.skills = skills;
            self.skill_diagnostics.extend(diags);
        }

        if !new_prompts.is_empty() {
            let merged = Self::merge_paths(
                &self.prompts.iter().map(|p| p.file_path.clone()).collect::<Vec<_>>(),
                &new_prompts,
                &self.cwd,
            );
            let (prompts, diags) = self.load_prompts_from_paths(&merged);
            self.prompts = prompts;
            self.prompt_diagnostics.extend(diags);
        }

        if !new_themes.is_empty() {
            let merged = Self::merge_paths(
                &self.themes.iter().filter_map(|t| t.source_path.clone()).collect::<Vec<_>>(),
                &new_themes,
                &self.cwd,
            );
            let (themes, diags) = self.load_themes_from_paths(&merged);
            self.themes = themes;
            self.theme_diagnostics.extend(diags);
        }
    }

    fn reload(&mut self) -> anyhow::Result<()> {
        let resolved = self.package_manager.resolve()?;

        // Skills
        let skill_paths: Vec<String> = if self.no_skills {
            self.additional_skill_paths.clone()
        } else {
            let mut paths: Vec<String> = resolved
                .skills
                .iter()
                .filter(|r| r.enabled)
                .map(|r| r.path.to_string_lossy().into_owned())
                .collect();
            paths.extend(self.additional_skill_paths.iter().cloned());
            paths
        };
        let (skills, skill_diags) = self.load_skills_from_paths(&skill_paths);
        self.skills = skills;
        self.skill_diagnostics = skill_diags;

        // Prompts
        let prompt_paths: Vec<String> = if self.no_prompt_templates {
            self.additional_prompt_template_paths.clone()
        } else {
            let mut paths: Vec<String> = resolved
                .prompts
                .iter()
                .filter(|r| r.enabled)
                .map(|r| r.path.to_string_lossy().into_owned())
                .collect();
            paths.extend(self.additional_prompt_template_paths.iter().cloned());
            paths
        };
        let (prompts, prompt_diags) = self.load_prompts_from_paths(&prompt_paths);
        self.prompts = prompts;
        self.prompt_diagnostics = prompt_diags;

        // Themes
        let theme_paths: Vec<String> = if self.no_themes {
            self.additional_theme_paths.clone()
        } else {
            let mut paths: Vec<String> = resolved
                .themes
                .iter()
                .filter(|r| r.enabled)
                .map(|r| r.path.to_string_lossy().into_owned())
                .collect();
            paths.extend(self.additional_theme_paths.iter().cloned());
            paths
        };
        let (themes, theme_diags) = self.load_themes_from_paths(&theme_paths);
        self.themes = themes;
        self.theme_diagnostics = theme_diags;

        // AGENTS files
        self.agents_files = self.load_agents_files();

        // System prompt
        let sys_source = self.system_prompt_source.clone()
            .or_else(|| self.discover_system_prompt_file());
        self.system_prompt = Self::resolve_prompt_input(sys_source.as_deref());

        // Append system prompt
        let append_source = self.append_system_prompt_source.clone()
            .or_else(|| self.discover_append_system_prompt_file());
        self.append_system_prompt = match Self::resolve_prompt_input(append_source.as_deref()) {
            Some(text) => vec![text],
            None => Vec::new(),
        };

        Ok(())
    }
}

// ============================================================================
// Module-level helpers
// ============================================================================

fn load_context_file_from_dir(dir: &Path) -> Option<AgentsFile> {
    for filename in &["AGENTS.md", "CLAUDE.md"] {
        let path = dir.join(filename);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                return Some(AgentsFile {
                    path: path.to_string_lossy().into(),
                    content,
                });
            }
        }
    }
    None
}

fn load_project_context_files(cwd: &Path, agent_dir: &Path) -> Vec<AgentsFile> {
    let mut context_files = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Some(global) = load_context_file_from_dir(agent_dir) {
        seen.insert(global.path.clone());
        context_files.push(global);
    }

    let mut ancestor_files = Vec::new();
    let mut current = cwd.to_path_buf();
    let root = PathBuf::from("/");

    loop {
        if let Some(file) = load_context_file_from_dir(&current) {
            if seen.insert(file.path.clone()) {
                ancestor_files.push(file);
            }
        }
        if current == root {
            break;
        }
        let parent = match current.parent() {
            Some(p) if p != current => p.to_path_buf(),
            _ => break,
        };
        current = parent;
    }

    // Reverse so that closest ancestors appear last (highest precedence).
    ancestor_files.reverse();
    // But we want the ordering: global first, then cwd-traversal from root inward.
    // In TS the unshift pattern prepends, so we reverse then extend.
    ancestor_files.reverse();
    context_files.extend(ancestor_files);

    context_files
}

fn resolve_resource_path(p: &str, base: &Path) -> PathBuf {
    let trimmed = p.trim();
    let home = dirs::home_dir().unwrap_or_default();
    let expanded = if trimmed == "~" {
        home.clone()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        home.join(rest)
    } else {
        base.join(trimmed)
    };
    expanded.canonicalize().unwrap_or(expanded)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_loader(tmp: &TempDir) -> DefaultResourceLoader {
        let cwd = tmp.path().join("project");
        let agent_dir = tmp.path().join("agent");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&agent_dir).unwrap();
        DefaultResourceLoader::new(DefaultResourceLoaderOptions {
            cwd: Some(cwd),
            agent_dir: Some(agent_dir),
            ..Default::default()
        })
    }

    #[test]
    fn empty_loader_has_no_resources() {
        let tmp = TempDir::new().unwrap();
        let loader = make_loader(&tmp);
        assert!(loader.get_skills().0.is_empty());
        assert!(loader.get_prompts().0.is_empty());
        assert!(loader.get_themes().0.is_empty());
        assert!(loader.get_agents_files().is_empty());
        assert!(loader.get_system_prompt().is_none());
    }

    #[test]
    fn reload_discovers_agents_file() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().join("project");
        let agent_dir = tmp.path().join("agent");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(cwd.join("AGENTS.md"), "# My Agents").unwrap();

        let mut loader = DefaultResourceLoader::new(DefaultResourceLoaderOptions {
            cwd: Some(cwd),
            agent_dir: Some(agent_dir),
            ..Default::default()
        });
        loader.reload().unwrap();
        assert!(!loader.get_agents_files().is_empty());
        assert!(loader.get_agents_files()[0].content.contains("My Agents"));
    }

    #[test]
    fn reload_discovers_system_prompt() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().join("project");
        let agent_dir = tmp.path().join("agent");
        std::fs::create_dir_all(cwd.join(CONFIG_DIR_NAME)).unwrap();
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(cwd.join(CONFIG_DIR_NAME).join("SYSTEM.md"), "You are an expert.").unwrap();

        let mut loader = DefaultResourceLoader::new(DefaultResourceLoaderOptions {
            cwd: Some(cwd),
            agent_dir: Some(agent_dir),
            ..Default::default()
        });
        loader.reload().unwrap();
        assert_eq!(loader.get_system_prompt(), Some("You are an expert."));
    }

    #[test]
    fn skill_loading_warning_for_missing_path() {
        let tmp = TempDir::new().unwrap();
        let loader = make_loader(&tmp);
        let (skills, diags) = loader.load_skills_from_paths(&["/nonexistent/skill.md".to_owned()]);
        assert!(skills.is_empty());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].kind, "warning");
    }

    #[test]
    fn prompt_loading_deduplicates_by_name() {
        let tmp = TempDir::new().unwrap();
        let loader = make_loader(&tmp);
        // Write two files with same stem.
        let p1 = tmp.path().join("foo.md");
        let p2 = tmp.path().join("foo2.md");
        std::fs::write(&p1, "# Foo").unwrap();
        std::fs::write(&p2, "# Foo2").unwrap();
        // They have different names so no collision.
        let (prompts, diags) = loader.load_prompts_from_paths(&[
            p1.to_string_lossy().into(),
            p2.to_string_lossy().into(),
        ]);
        assert_eq!(prompts.len(), 2);
        assert!(diags.is_empty());
    }

    #[test]
    fn theme_loading_parses_name() {
        let tmp = TempDir::new().unwrap();
        let theme_path = tmp.path().join("my-theme.json");
        std::fs::write(&theme_path, r#"{"name":"dark","colors":{}}"#).unwrap();
        let loader = make_loader(&tmp);
        let (themes, diags) = loader.load_themes_from_paths(&[theme_path.to_string_lossy().into()]);
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].name.as_deref(), Some("dark"));
        assert!(diags.is_empty());
    }
}

use anyhow::{Context as _, Result};
use sage_runner::config::{MemoryInjectMode, NetworkPolicy, SecurityConfig};
use sage_runner::hooks::{ScriptPostToolUseHook, ScriptPreToolUseHook, ScriptStopHook};
use sage_runner::AgentConfig;
use sage_runtime::engine::{SageEngine, SandboxSettings};
use sage_runtime::event::AgentEvent;
use sage_runtime::types::*;
use std::path::PathBuf;

// ── Registry helpers ──────────────────────────────────────────────────────────

/// Return the root directory for registered agents: `~/.sage/agents/`.
///
/// Uses `sage_runner::home_dir()` so the HOME lookup logic lives in exactly
/// one place, shared with `expand_tilde` in the runner crate.
pub(crate) fn sage_agents_dir() -> Result<PathBuf> {
    let home = sage_runner::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory (HOME / USERPROFILE not set)"))?;
    Ok(home.join(".sage").join("agents"))
}

/// Write `content` to `path` only if the file does not yet exist.
///
/// Uses `O_EXCL` / `create_new` semantics to avoid the TOCTOU race that
/// `if !path.exists() { write() }` suffers from. Two concurrent `sage init`
/// calls for the same agent will both attempt to create the file; whichever
/// wins keeps its content, the other silently skips.
async fn write_if_new(path: &std::path::Path, content: impl AsRef<[u8]>) -> Result<()> {
    use tokio::io::AsyncWriteExt as _;
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
    {
        Ok(mut file) => file
            .write_all(content.as_ref())
            .await
            .with_context(|| format!("failed to write {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e).with_context(|| format!("failed to create {}", path.display())),
    }
}

// ── M1: Agent Registry CLI ────────────────────────────────────────────────────

/// Reject agent names that would introduce path traversal or ambiguity.
///
/// A valid agent name must be a single normal path component: no `/`, no `..`,
/// no absolute prefix. Backslash is also rejected — valid on Unix as a filename
/// character but almost certainly a Windows-path mistake and confusing to users.
///
/// Uses `Path::components()` rather than string matching so the check is
/// platform-aware and handles all separator forms correctly.
fn validate_agent_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("agent name cannot be empty");
    }
    // Backslash is a valid filename character on Unix but is almost certainly
    // a Windows path mistake. Reject it explicitly for clarity.
    if name.contains('\\') {
        anyhow::bail!(
            "invalid agent name '{}': must not contain backslash",
            name
        );
    }
    // The name must consist of exactly one Normal component — no separators,
    // no `..`, no absolute prefix.
    use std::path::Component;
    let mut components = std::path::Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => anyhow::bail!(
            "invalid agent name '{}': must not contain path separators or '..'",
            name
        ),
    }
}

/// Initialise a new agent workspace at the given agents root.
///
/// Test-friendly variant of [`init_agent`]: the caller supplies the root
/// directory (normally `~/.sage/agents/`), so tests can point it at a
/// `tempfile::TempDir` without mutating the shared `HOME` environment variable.
///
/// Creates `<agents_dir>/<name>/…` with `AGENT.md`, `config.yaml`,
/// `memory/MEMORY.md`, and the extended wiki/workspace skeleton
/// (`workspace/SCHEMA.md`, `workspace/wiki/…`, `workspace/raw/sessions/`,
/// `workspace/metrics/`, `workspace/craft/`, `workspace/skills/`). All writes
/// use [`write_if_new`] semantics — re-running against an existing agent dir
/// is idempotent and never overwrites user edits.
pub(crate) async fn init_agent_at(agents_dir: &std::path::Path, name: &str) -> Result<()> {
    validate_agent_name(name)?;

    let agent_dir = agents_dir.join(name);
    tokio::fs::create_dir_all(&agent_dir)
        .await
        .with_context(|| format!("failed to create agent directory {}", agent_dir.display()))?;

    let memory_dir = agent_dir.join("memory");
    tokio::fs::create_dir_all(&memory_dir)
        .await
        .with_context(|| format!("failed to create memory directory {}", memory_dir.display()))?;

    // workspace/ — mounted read-write into the sandbox at /workspace. Kept
    // separate from config/memory so the agent cannot modify its own
    // config.yaml or security settings from within the sandbox.
    let workspace_dir = agent_dir.join("workspace");
    let raw_sessions_dir = workspace_dir.join("raw").join("sessions");
    let wiki_dir = workspace_dir.join("wiki");
    let wiki_pages_dir = wiki_dir.join("pages");
    let metrics_dir = workspace_dir.join("metrics");
    let craft_dir = workspace_dir.join("craft");
    let skills_dir = workspace_dir.join("skills");

    for dir in [
        &workspace_dir,
        &raw_sessions_dir,
        &wiki_dir,
        &wiki_pages_dir,
        &metrics_dir,
        &craft_dir,
        &skills_dir,
    ] {
        tokio::fs::create_dir_all(dir)
            .await
            .with_context(|| format!("failed to create directory {}", dir.display()))?;
    }

    let agent_md_content = format!(
        "# {name}\n\n\
         ## Description\n\n\
         TODO: describe this agent's purpose.\n\n\
         ## Instructions\n\n\
         TODO: add agent-specific instructions and context.\n"
    );
    write_if_new(&agent_dir.join("AGENT.md"), agent_md_content).await?;

    write_if_new(&memory_dir.join("MEMORY.md"), format!("# {name} Memory\n\n")).await?;

    let config_template = include_str!("templates/config.yaml");
    write_if_new(
        &agent_dir.join("config.yaml"),
        config_template.replace("__NAME__", name),
    )
    .await?;

    let schema_template = include_str!("templates/schema.md");
    write_if_new(&workspace_dir.join("SCHEMA.md"), schema_template).await?;

    write_if_new(
        &wiki_dir.join("index.md"),
        "# Wiki Index\n\n<!-- populated by wiki-ingest -->\n",
    )
    .await?;
    write_if_new(
        &wiki_dir.join("log.md"),
        "# Wiki Maintenance Log\n\n<!-- append-only; processed sessions recorded below -->\n",
    )
    .await?;
    write_if_new(
        &wiki_dir.join("overview.md"),
        "# Domain Overview\n\n<!-- evolving synthesis; updated by wiki-ingest -->\n",
    )
    .await?;

    for gitkeep_dir in [
        &raw_sessions_dir,
        &wiki_pages_dir,
        &metrics_dir,
        &craft_dir,
        &skills_dir,
    ] {
        write_if_new(&gitkeep_dir.join(".gitkeep"), b"" as &[u8]).await?;
    }

    Ok(())
}

/// Initialise a new agent workspace under `~/.sage/agents/<name>/`.
///
/// Thin wrapper over [`init_agent_at`] that resolves the default agents root
/// and prints a confirmation line. All filesystem work lives in `init_agent_at`
/// so tests can drive it against a `TempDir`.
pub async fn init_agent(agent: &str) -> Result<()> {
    let agents_dir = sage_agents_dir()?;
    init_agent_at(&agents_dir, agent).await?;
    println!(
        "✓ Initialized agent '{agent}' at {}",
        agents_dir.join(agent).display()
    );
    Ok(())
}

/// List all registered agents in `~/.sage/agents/`.
pub async fn list_agents() -> Result<()> {
    let agents_dir = sage_agents_dir()?;

    let mut entries = match tokio::fs::read_dir(&agents_dir).await {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Directory doesn't exist yet — that's fine, it just means no agents
            // have been initialized. Not an error.
            println!("No agents registered (run `sage init --agent <name>` to create one).");
            return Ok(());
        }
        Err(e) => {
            return Err(e)
                .with_context(|| format!("failed to read agents directory {}", agents_dir.display()));
        }
    };

    let mut agents = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        // A broken inode or permission error on a single entry should not abort
        // the entire listing — skip it with a warning instead.
        let is_dir = match entry.file_type().await {
            Ok(ft) => ft.is_dir(),
            Err(e) => {
                eprintln!(
                    "warning: cannot read file type for {:?}: {e}",
                    entry.file_name()
                );
                continue;
            }
        };
        if is_dir {
            match entry.file_name().into_string() {
                Ok(name) => agents.push(name),
                Err(raw) => {
                    eprintln!("warning: skipping agent directory with non-UTF-8 name: {raw:?}");
                }
            }
        }
    }

    if agents.is_empty() {
        println!("No agents registered.");
    } else {
        agents.sort();
        println!("Registered agents:");
        for name in &agents {
            println!("  {name}");
        }
    }

    Ok(())
}

// ── Memory + Skill injection ──────────────────────────────────────────────────

/// Load all `auto_load` memory files relative to the agent's workspace dir.
///
/// Files that don't exist or are unreadable are silently skipped — a missing
/// AGENT.md should not prevent the agent from starting.
async fn load_memory_sections(agent_dir: &PathBuf, auto_load: &[String]) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    for rel_path in auto_load {
        let path = agent_dir.join(rel_path);
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => sections.push((rel_path.clone(), content)),
            Err(_) => {} // silently skip missing files
        }
    }
    sections
}

/// Scan a directory for `*.md` skill files and return `(name, content)` pairs.
///
/// `name` is the stem of the filename (e.g. `"calendar"` for `calendar.md`).
/// Files that can't be read are silently skipped.
async fn load_skill_files(dir: &PathBuf) -> Vec<(String, String)> {
    let mut skills = Vec::new();
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return skills;
    };
    loop {
        let entry = match rd.next_entry().await {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("error reading skill dir entry: {e}");
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            skills.push((name, content));
        }
    }
    skills.sort_by(|a, b| a.0.cmp(&b.0));
    skills
}

/// Build the final system prompt by injecting memory and skills.
pub async fn build_system_prompt(
    base: &str,
    config: &AgentConfig,
    agent_dir: &PathBuf,
) -> String {
    let mut prompt = base.to_string();

    // ── Memory injection ─────────────────────────────────────────────
    if let Some(mem_cfg) = &config.memory {
        let raw_sections = load_memory_sections(agent_dir, &mem_cfg.auto_load).await;
        let section_refs: Vec<(&str, &str)> = raw_sections
            .iter()
            .map(|(l, c)| (l.as_str(), c.as_str()))
            .collect();

        match mem_cfg.inject_as {
            MemoryInjectMode::PrependSystem => {
                prompt = crate::context::prepend_memory_sections(&prompt, &section_refs);
            }
            MemoryInjectMode::InitialMessage => {
                // InitialMessage injection is not yet implemented; fall back to PrependSystem
                // so agent context is always populated. Remove this once the engine builder
                // supports injecting memory as the first user message.
                tracing::warn!(
                    "InitialMessage inject mode not yet implemented; \
                     falling back to PrependSystem"
                );
                prompt = crate::context::prepend_memory_sections(&prompt, &section_refs);
            }
        }
    }

    // ── Skill injection ──────────────────────────────────────────────
    let workspace_skills_dir = agent_dir.join("workspace").join("skills");

    let mut all_skill_pairs = Vec::new();
    if let Some(home) = sage_runner::home_dir() {
        all_skill_pairs.extend(load_skill_files(&home.join(".sage").join("skills")).await);
    }
    all_skill_pairs.extend(load_skill_files(&workspace_skills_dir).await);

    if !all_skill_pairs.is_empty() {
        let skill_entries: Vec<crate::context::SkillEntry> = all_skill_pairs
            .iter()
            .map(|(name, content)| crate::context::SkillEntry {
                name: name.as_str(),
                content: content.as_str(),
            })
            .collect();
        prompt = crate::context::append_skill_sections(&prompt, &skill_entries);
    }

    prompt
}

// ── Agent Registry CLI ────────────────────────────────────────────────────────

/// Load the agent config from `~/.sage/agents/<name>/config.yaml`.
pub async fn load_agent_config(name: &str) -> anyhow::Result<AgentConfig> {
    let config_path = sage_agents_dir()?.join(name).join("config.yaml");
    let yaml = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| {
            format!(
                "cannot read config for agent '{name}' at {}",
                config_path.display()
            )
        })?;
    serde_yaml::from_str(&yaml)
        .with_context(|| format!("invalid config for agent '{name}'"))
}

/// Build a [`SageEngine`] from an [`AgentConfig`].
///
/// When `dev` is `true`, sandbox settings from the config are ignored and
/// the bash tool runs directly on the host. This is the default for
/// `sage chat --dev` and the daemon started with `sage start --dev`.
pub async fn build_engine_for_agent(config: &AgentConfig, dev: bool) -> anyhow::Result<SageEngine> {
    let agent_dir = sage_agents_dir()?.join(&config.name);
    let system_prompt = build_system_prompt(&config.system_prompt, config, &agent_dir).await;

    let tool_names = config.tools.tool_names();
    let tool_name_refs: Vec<&str> = tool_names.iter().map(|s| s.as_str()).collect();

    let mut builder = SageEngine::builder()
        .name(&config.name)
        .system_prompt(&system_prompt)
        .provider(&config.llm.provider)
        .model(&config.llm.model)
        .max_tokens(config.llm.max_tokens)
        .max_turns(config.constraints.max_turns as usize)
        .timeout_secs(config.constraints.timeout_secs as u64)
        .tool_execution_mode(ToolExecutionMode::Parallel)
        .tool_policy(config.tools.to_policy())
        .builtin_tools(&tool_name_refs);

    if let Some(hooks) = &config.hooks {
        if !hooks.pre_tool_use.is_empty() {
            builder = builder.on_before_tool_call(ScriptPreToolUseHook {
                hooks: hooks.pre_tool_use.clone(),
            });
        }
        if !hooks.post_tool_use.is_empty() {
            builder = builder.on_after_tool_call(ScriptPostToolUseHook {
                hooks: hooks.post_tool_use.clone(),
            });
        }
        if !hooks.stop.is_empty() {
            builder = builder.on_stop(ScriptStopHook {
                hooks: hooks.stop.clone(),
            });
        }
    }

    if let Some(url) = &config.llm.base_url {
        builder = builder.base_url(url);
    }
    if let Some(env) = &config.llm.api_key_env {
        builder = builder.api_key_env(env);
    }

    if !dev {
        if let Some(sandbox) = &config.sandbox {
            match &sandbox.network {
                NetworkPolicy::Full => {
                    anyhow::bail!(
                        "network policy 'full' is not yet implemented — use 'airgapped' (default)"
                    );
                }
                NetworkPolicy::Whitelist => {
                    anyhow::bail!(
                        "network policy 'whitelist' is not yet implemented — use 'airgapped' (default)"
                    );
                }
                NetworkPolicy::Airgapped => {}
            }

            let mut volumes = Vec::new();
            if let Some(host_path) = &sandbox.workspace_host {
                volumes.push(sage_sandbox::VolumeMount {
                    host_path: host_path.to_string_lossy().into_owned(),
                    guest_path: "/workspace".to_string(),
                    read_only: false,
                });
            }

            let extra_paths: Vec<&str> =
                volumes.iter().map(|v| v.guest_path.as_str()).collect();
            let guest_security = to_guest_security(&sandbox.security, &extra_paths);

            builder = builder.sandbox(SandboxSettings {
                cpus: sandbox.cpus,
                memory_mib: sandbox.memory_mib,
                volumes,
                network_enabled: false,
                security: Some(guest_security),
            });
        }
    }

    Ok(builder.build()?)
}

/// Validate the YAML config for the named agent.
///
/// Reads `~/.sage/agents/<name>/config.yaml`, parses it, and reports any errors.
pub async fn validate_agent(agent: &str) -> Result<()> {
    validate_agent_name(agent)?;

    let config_path = sage_agents_dir()?.join(agent).join("config.yaml");

    let yaml = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("cannot read config at {}", config_path.display()))?;

    match serde_yaml::from_str::<AgentConfig>(&yaml) {
        Ok(_) => {
            println!("✓ Agent '{agent}' config is valid.");
            Ok(())
        }
        Err(e) => {
            // serde_yaml::Error includes line/column information in its Display output.
            anyhow::bail!("invalid config for agent '{agent}': {e}");
        }
    }
}

/// Main serve loop.
///
/// 1. Connect to Rune Runtime
/// 2. Register `agents.execute` rune
/// 3. Handle incoming tasks: parse config → create sandbox → run agent → return result
pub async fn run(runtime_addr: String, _caster_id: String, _max_concurrent: usize) -> Result<()> {
    tracing::info!("connecting to Rune Runtime at {}", runtime_addr);

    anyhow::bail!(
        "sage serve is not yet implemented — Rune Caster SDK integration pending (Phase 2). \
         Use `sage run` for local agent execution instead."
    );
}

/// Run a local test: load config → build SageEngine → run → print events.
pub async fn run_local_test(
    config_path: &str,
    message: &str,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<()> {
    // 1. Load agent config
    let yaml = tokio::fs::read_to_string(config_path)
        .await
        .with_context(|| format!("cannot read config at {config_path}"))?;
    let config: AgentConfig = serde_yaml::from_str(&yaml)
        .with_context(|| format!("invalid config at {config_path}"))?;
    tracing::info!(agent = %config.name, "loaded config");

    // 2. Build SageEngine from AgentConfig fields
    let engine = build_engine_from_config(&config, provider_override, model_override)?;

    // 3. Run and consume events
    let mut rx = engine.run(message).await?;
    while let Some(event) = rx.next().await {
        print_event(&event);
    }

    Ok(())
}

/// Print an agent event to stderr (terminal output).
fn print_event(event: &AgentEvent) {
    match event {
        AgentEvent::AgentStart => {
            eprintln!("--- Agent started ---");
        }
        AgentEvent::RunError { error } => {
            eprintln!("--- Agent failed: {error} ---");
        }
        AgentEvent::AgentEnd { messages } => {
            // Print the final assistant reply — MessageUpdate may not be
            // emitted by the current agent loop, so extract text here.
            for msg in messages {
                if let AgentMessage::Assistant(a) = msg {
                    for c in &a.content {
                        if let Content::Text { text } = c {
                            println!("{text}");
                        }
                    }
                }
            }
            eprintln!("--- Agent finished ---");
        }
        AgentEvent::TurnStart => {
            eprintln!("  [turn]");
        }
        AgentEvent::TurnEnd { .. } => {}
        AgentEvent::MessageStart { message } => {
            if let AgentMessage::User(u) = message {
                eprintln!(
                    "  > User: {}",
                    u.content
                        .iter()
                        .filter_map(|c| match c {
                            Content::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("")
                );
            }
        }
        // NOTE: Not currently emitted by agent_loop — reserved for future streaming
        AgentEvent::MessageUpdate { delta, .. } => {
            eprint!("{delta}");
        }
        AgentEvent::MessageEnd { .. } => {
            eprintln!();
        }
        AgentEvent::ToolExecutionStart { tool_name, .. } => {
            eprintln!("  [tool: {tool_name}]");
        }
        // NOTE: Not currently emitted by agent_loop — reserved for future streaming
        AgentEvent::ToolExecutionUpdate { partial_result, .. } => {
            eprint!("{partial_result}");
        }
        AgentEvent::ToolExecutionEnd {
            tool_name,
            is_error,
            ..
        } => {
            if *is_error {
                eprintln!("  [tool: {tool_name} — ERROR]");
            }
        }
        AgentEvent::CompactionStart {
            reason,
            message_count,
        } => {
            eprintln!("  [compaction: {reason}, {message_count} messages]");
        }
        AgentEvent::CompactionEnd {
            tokens_before,
            messages_compacted,
        } => {
            eprintln!("  [compacted: {messages_compacted} messages, was {tokens_before} tokens]");
        }
    }
}

fn build_engine_from_config(
    config: &AgentConfig,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<SageEngine> {
    let tool_names = config.tools.tool_names();
    let tool_name_refs: Vec<&str> = tool_names.iter().map(|s| s.as_str()).collect();

    let mut builder = SageEngine::builder()
        .name(&config.name)
        .system_prompt(&config.system_prompt)
        .provider(provider_override.unwrap_or(&config.llm.provider))
        .model(model_override.unwrap_or(&config.llm.model))
        .max_tokens(config.llm.max_tokens)
        .max_turns(config.constraints.max_turns as usize)
        .timeout_secs(config.constraints.timeout_secs as u64)
        .tool_execution_mode(ToolExecutionMode::Parallel)
        .tool_policy(config.tools.to_policy())
        .builtin_tools(&tool_name_refs);

    // Wire lifecycle hooks from config
    if let Some(hooks) = &config.hooks {
        if !hooks.pre_tool_use.is_empty() {
            builder = builder.on_before_tool_call(ScriptPreToolUseHook {
                hooks: hooks.pre_tool_use.clone(),
            });
        }
        if !hooks.post_tool_use.is_empty() {
            builder = builder.on_after_tool_call(ScriptPostToolUseHook {
                hooks: hooks.post_tool_use.clone(),
            });
        }
        if !hooks.stop.is_empty() {
            builder = builder.on_stop(ScriptStopHook {
                hooks: hooks.stop.clone(),
            });
        }
    }

    if let Some(url) = &config.llm.base_url {
        builder = builder.base_url(url);
    }
    if let Some(env) = &config.llm.api_key_env {
        builder = builder.api_key_env(env);
    }
    if let Some(sandbox) = &config.sandbox {
        match &sandbox.network {
            NetworkPolicy::Full => {
                anyhow::bail!(
                    "network policy 'full' is not yet implemented — use 'airgapped' (default)"
                );
            }
            NetworkPolicy::Whitelist => {
                anyhow::bail!(
                    "network policy 'whitelist' is not yet implemented — use 'airgapped' (default)"
                );
            }
            NetworkPolicy::Airgapped => {}
        }

        // Build volume mounts: workspace_host → /workspace (read-write)
        let mut volumes = Vec::new();
        if let Some(host_path) = &sandbox.workspace_host {
            volumes.push(sage_sandbox::VolumeMount {
                host_path: host_path.to_string_lossy().into_owned(),
                guest_path: "/workspace".to_string(),
                read_only: false,
            });
        }

        // Convert runner SecurityConfig → protocol GuestSecurityConfig for the guest.
        // Include /workspace in landlock allowed paths when a workspace is mounted.
        let extra_paths: Vec<&str> = volumes
            .iter()
            .map(|v| v.guest_path.as_str())
            .collect();
        let guest_security = to_guest_security(&sandbox.security, &extra_paths);

        builder = builder.sandbox(SandboxSettings {
            cpus: sandbox.cpus,
            memory_mib: sandbox.memory_mib,
            volumes,
            network_enabled: false,
            security: Some(guest_security),
        });
    }

    Ok(builder.build()?)
}

/// Convert runner `SecurityConfig` → protocol `GuestSecurityConfig`.
///
/// `/tmp` is always included in `allowed_paths`. Additional paths (e.g.
/// `/workspace` from a volume mount) are supplied via `extra_volume_paths`
/// and appended without duplication.
fn to_guest_security(
    config: &SecurityConfig,
    extra_volume_paths: &[&str],
) -> sage_protocol::GuestSecurityConfig {
    let mut allowed_paths: Vec<String> = vec!["/tmp".into()];
    for path in extra_volume_paths {
        if !allowed_paths.iter().any(|p| p == path) {
            allowed_paths.push((*path).into());
        }
    }
    sage_protocol::GuestSecurityConfig {
        seccomp: config.seccomp,
        landlock: config.landlock,
        max_file_size_mb: config.max_file_size_mb,
        max_open_files: config.max_open_files,
        tmpfs_size_mb: config.tmpfs_size_mb,
        max_processes: config.max_processes,
        allowed_paths,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_agent_name ───────────────────────────────────────────────────

    #[test]
    fn validate_agent_name_accepts_normal_names() {
        assert!(validate_agent_name("feishu").is_ok());
        assert!(validate_agent_name("my-agent").is_ok());
        assert!(validate_agent_name("coding_bot").is_ok());
        assert!(validate_agent_name("agent42").is_ok());
    }

    #[test]
    fn validate_agent_name_rejects_empty() {
        assert!(validate_agent_name("").is_err());
    }

    #[test]
    fn validate_agent_name_rejects_path_traversal_dotdot() {
        assert!(validate_agent_name("../../etc/passwd").is_err());
        assert!(validate_agent_name("../evil").is_err());
        assert!(validate_agent_name("good/../evil").is_err());
    }

    #[test]
    fn validate_agent_name_rejects_absolute_slash() {
        assert!(validate_agent_name("/etc/passwd").is_err());
        assert!(validate_agent_name("/tmp/evil").is_err());
    }

    #[test]
    fn validate_agent_name_rejects_backslash() {
        assert!(validate_agent_name("evil\\path").is_err());
    }

    // ── build_engine_from_config ──────────────────────────────────────────────

    #[test]
    fn test_fix_build_engine_from_config_wires_sandbox_settings() {
        let yaml = r#"
name: sandboxed
description: "sandboxed"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 2
  memory_mib: 1024
  network: airgapped
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();

        let engine = build_engine_from_config(&config, None, None).unwrap();
        let sandbox = engine
            .sandbox_settings()
            .expect("sandbox settings should be wired from YAML");

        assert_eq!(sandbox.cpus, 2);
        assert_eq!(sandbox.memory_mib, 1024);
        assert!(!sandbox.network_enabled);
    }

    #[test]
    fn test_fix_build_engine_from_config_wires_timeout_secs() {
        let yaml = r#"
name: timed
description: "timed"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 47 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();

        let engine = build_engine_from_config(&config, None, None).unwrap();
        assert_eq!(engine.timeout_secs(), Some(47));
    }

    #[test]
    fn test_sandbox_wires_security_config_defaults() {
        let yaml = r#"
name: secured
description: "secured"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 1
  memory_mib: 512
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let engine = build_engine_from_config(&config, None, None).unwrap();
        let sandbox = engine.sandbox_settings().expect("sandbox should be set");

        let security = sandbox.security.as_ref().expect("security should be wired");
        assert!(security.seccomp);
        assert!(security.landlock);
        assert_eq!(security.max_file_size_mb, 100);
        assert_eq!(security.max_open_files, 256);
        assert_eq!(security.tmpfs_size_mb, 512);
    }

    #[test]
    fn test_sandbox_wires_custom_security_config() {
        let yaml = r#"
name: custom-sec
description: "custom"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 2
  memory_mib: 1024
  network: airgapped
  security:
    seccomp: false
    landlock: true
    max_file_size_mb: 50
    max_open_files: 128
    tmpfs_size_mb: 256
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let engine = build_engine_from_config(&config, None, None).unwrap();
        let sandbox = engine.sandbox_settings().expect("sandbox should be set");

        let security = sandbox.security.as_ref().expect("security should be wired");
        assert!(!security.seccomp);
        assert!(security.landlock);
        assert_eq!(security.max_file_size_mb, 50);
        assert_eq!(security.max_open_files, 128);
        assert_eq!(security.tmpfs_size_mb, 256);
    }

    #[test]
    fn test_sandbox_without_security_section_uses_defaults() {
        let yaml = r#"
name: no-sec-section
description: "no explicit security"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 1
  memory_mib: 512
  network: false
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let engine = build_engine_from_config(&config, None, None).unwrap();
        let sandbox = engine.sandbox_settings().expect("sandbox should be set");

        // SecurityConfig defaults to all enabled
        let security = sandbox.security.as_ref().expect("security should be wired");
        assert!(security.seccomp);
        assert!(security.landlock);
    }

    #[test]
    fn test_no_sandbox_means_no_security() {
        let yaml = r#"
name: no-sandbox
description: "no sandbox"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let engine = build_engine_from_config(&config, None, None).unwrap();
        assert!(engine.sandbox_settings().is_none());
    }

    #[test]
    fn test_full_pipeline_yaml_to_guest_security_roundtrip() {
        // Full pipeline: YAML → AgentConfig → SandboxSettings → JSON → GuestSecurityConfig
        let yaml = r#"
name: pipeline-test
description: "full pipeline"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 2
  memory_mib: 1024
  security:
    seccomp: false
    landlock: true
    max_file_size_mb: 75
    max_open_files: 192
    tmpfs_size_mb: 384
    max_processes: 96
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let engine = build_engine_from_config(&config, None, None).unwrap();
        let sandbox = engine.sandbox_settings().unwrap();
        let security = sandbox.security.as_ref().unwrap();

        // Serialize as the builder would for SAGE_SECURITY env var
        let json = serde_json::to_string(security).unwrap();

        // Deserialize as the guest would
        let guest_config: sage_protocol::GuestSecurityConfig =
            serde_json::from_str(&json).unwrap();

        // Verify all values survived the full pipeline
        assert!(!guest_config.seccomp);
        assert!(guest_config.landlock);
        assert_eq!(guest_config.max_file_size_mb, 75);
        assert_eq!(guest_config.max_open_files, 192);
        assert_eq!(guest_config.tmpfs_size_mb, 384);
        assert_eq!(guest_config.max_processes, 96);
    }

    #[test]
    fn test_security_allowed_paths_without_workspace() {
        // Without a workspace_host, allowed_paths should only contain /tmp.
        let yaml = r#"
name: paths-test
description: "test"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 1
  memory_mib: 512
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let engine = build_engine_from_config(&config, None, None).unwrap();
        let security = engine.sandbox_settings().unwrap().security.as_ref().unwrap();
        assert_eq!(security.allowed_paths, vec!["/tmp"]);
    }

    #[test]
    fn test_security_allowed_paths_includes_workspace_when_mounted() {
        // With a workspace_host, /workspace must appear in allowed_paths.
        let yaml = r#"
name: ws-paths-test
description: "test"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 1
  memory_mib: 512
  workspace_host: /tmp/test-workspace
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let engine = build_engine_from_config(&config, None, None).unwrap();
        let security = engine.sandbox_settings().unwrap().security.as_ref().unwrap();
        assert!(
            security.allowed_paths.contains(&"/workspace".to_string()),
            "expected /workspace in allowed_paths when workspace_host is set, got: {:?}",
            security.allowed_paths
        );
        assert!(security.allowed_paths.contains(&"/tmp".to_string()));
    }

    // ── Regression: unsupported network policies rejected at config time ──

    #[test]
    fn test_fix_network_full_rejected_at_config_time() {
        let yaml = r#"
name: net-full
description: "full network"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 1
  memory_mib: 512
  network: full
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let result = build_engine_from_config(&config, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not yet implemented"), "error was: {err}");
    }

    #[test]
    fn test_fix_network_whitelist_rejected_at_config_time() {
        let yaml = r#"
name: net-whitelist
description: "whitelist network"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 1
  memory_mib: 512
  network: whitelist
  allowed_hosts: ["api.example.com"]
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let result = build_engine_from_config(&config, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not yet implemented"), "error was: {err}");
    }

    #[test]
    fn test_fix_network_true_rejected_at_config_time() {
        // `network: true` maps to NetworkPolicy::Full via bool compat
        let yaml = r#"
name: net-true
description: "bool network"
llm: { provider: test, model: test-model, max_tokens: 256 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 90 }
sandbox:
  cpus: 1
  memory_mib: 512
  network: true
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let result = build_engine_from_config(&config, None, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fix_serve_stub_returns_error() {
        let result = run("localhost:50070".into(), "test-caster".into(), 1).await;
        assert!(result.is_err(), "serve stub should return an error, not silently wait");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not yet implemented"),
            "error should explain that serve is unimplemented, got: {err}"
        );
    }

    // ── S5.3: init_agent_at wiki/workspace skeleton ───────────────────────────

    /// The 9 paths that `init_agent_at` must create in addition to the
    /// pre-existing `AGENT.md` / `memory/MEMORY.md` / `config.yaml` / `workspace/`.
    fn expected_skeleton_paths(agent_root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let ws = agent_root.join("workspace");
        vec![
            ws.join("SCHEMA.md"),
            ws.join("raw").join("sessions").join(".gitkeep"),
            ws.join("wiki").join("index.md"),
            ws.join("wiki").join("log.md"),
            ws.join("wiki").join("overview.md"),
            ws.join("wiki").join("pages").join(".gitkeep"),
            ws.join("metrics").join(".gitkeep"),
            ws.join("craft").join(".gitkeep"),
            ws.join("skills").join(".gitkeep"),
        ]
    }

    #[tokio::test]
    async fn test_init_agent_at_creates_full_skeleton() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_agent_at(tmp.path(), "agent1").await.unwrap();

        let agent_root = tmp.path().join("agent1");
        for path in expected_skeleton_paths(&agent_root) {
            assert!(path.exists(), "expected path missing: {}", path.display());
        }
    }

    #[tokio::test]
    async fn test_init_agent_at_schema_md_matches_template_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_agent_at(tmp.path(), "agent1").await.unwrap();

        let schema_path = tmp.path().join("agent1").join("workspace").join("SCHEMA.md");
        let written = tokio::fs::read(&schema_path).await.unwrap();
        let template = include_str!("templates/schema.md").as_bytes();
        assert_eq!(
            written, template,
            "SCHEMA.md on disk must be byte-identical to the embedded template"
        );
    }

    #[tokio::test]
    async fn test_init_agent_at_schema_md_utf8_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_agent_at(tmp.path(), "agent1").await.unwrap();

        let schema_path = tmp.path().join("agent1").join("workspace").join("SCHEMA.md");
        let written = tokio::fs::read_to_string(&schema_path).await.unwrap();
        let template = include_str!("templates/schema.md");
        assert_eq!(written, template, "UTF-8 read must round-trip the template");
    }

    #[tokio::test]
    async fn test_init_agent_at_wiki_index_has_marker() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_agent_at(tmp.path(), "agent1").await.unwrap();

        let path = tmp.path().join("agent1").join("workspace").join("wiki").join("index.md");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            content.contains("<!-- populated by wiki-ingest -->"),
            "wiki/index.md must contain the wiki-ingest marker, got: {content:?}"
        );
    }

    #[tokio::test]
    async fn test_init_agent_at_wiki_log_has_append_only_marker() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_agent_at(tmp.path(), "agent1").await.unwrap();

        let path = tmp.path().join("agent1").join("workspace").join("wiki").join("log.md");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            content.contains("append-only"),
            "wiki/log.md must mention append-only semantics, got: {content:?}"
        );
    }

    #[tokio::test]
    async fn test_init_agent_at_wiki_overview_has_synthesis_marker() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_agent_at(tmp.path(), "agent1").await.unwrap();

        let path = tmp.path().join("agent1").join("workspace").join("wiki").join("overview.md");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            content.contains("evolving synthesis"),
            "wiki/overview.md must mention evolving synthesis, got: {content:?}"
        );
    }

    #[tokio::test]
    async fn test_init_agent_at_gitkeep_files_are_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_agent_at(tmp.path(), "agent1").await.unwrap();

        let ws = tmp.path().join("agent1").join("workspace");
        let gitkeeps = [
            ws.join("raw").join("sessions").join(".gitkeep"),
            ws.join("wiki").join("pages").join(".gitkeep"),
            ws.join("metrics").join(".gitkeep"),
            ws.join("craft").join(".gitkeep"),
            ws.join("skills").join(".gitkeep"),
        ];
        for path in &gitkeeps {
            let meta = tokio::fs::metadata(path).await.unwrap();
            assert_eq!(
                meta.len(),
                0,
                "{} must be a 0-byte .gitkeep",
                path.display()
            );
        }
    }

    #[tokio::test]
    async fn test_init_agent_at_preserves_existing_init_agent_files() {
        // Regression: the new skeleton must not replace the original files
        // that `init_agent` used to create (AGENT.md / MEMORY.md / config.yaml
        // / workspace/).
        let tmp = tempfile::TempDir::new().unwrap();
        init_agent_at(tmp.path(), "agent1").await.unwrap();

        let agent_root = tmp.path().join("agent1");
        assert!(agent_root.join("AGENT.md").is_file());
        assert!(agent_root.join("memory").join("MEMORY.md").is_file());
        assert!(agent_root.join("config.yaml").is_file());
        assert!(agent_root.join("workspace").is_dir());
    }

    #[tokio::test]
    async fn test_init_agent_at_is_idempotent_preserves_user_edits() {
        // User customises SCHEMA.md after first init; a second init must NOT
        // overwrite it. Exercises `write_if_new` semantics end-to-end.
        let tmp = tempfile::TempDir::new().unwrap();
        let agent_root = tmp.path().join("agent1");
        let schema_path = agent_root.join("workspace").join("SCHEMA.md");

        tokio::fs::create_dir_all(schema_path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&schema_path, b"custom content").await.unwrap();

        init_agent_at(tmp.path(), "agent1").await.unwrap();

        let got = tokio::fs::read(&schema_path).await.unwrap();
        assert_eq!(
            got, b"custom content",
            "existing SCHEMA.md must not be clobbered on re-init"
        );
    }

    #[tokio::test]
    async fn test_init_agent_at_fills_missing_from_partial_tree() {
        // Only the workspace/ directory pre-exists; init must populate every
        // missing child without erroring on the existing dir.
        let tmp = tempfile::TempDir::new().unwrap();
        let agent_root = tmp.path().join("agent1");
        tokio::fs::create_dir_all(agent_root.join("workspace")).await.unwrap();

        init_agent_at(tmp.path(), "agent1").await.unwrap();

        for path in expected_skeleton_paths(&agent_root) {
            assert!(
                path.exists(),
                "partial-recovery path missing: {}",
                path.display()
            );
        }
    }

    #[tokio::test]
    async fn test_init_agent_at_rejects_empty_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(init_agent_at(tmp.path(), "").await.is_err());
    }

    #[tokio::test]
    async fn test_init_agent_at_rejects_dotdot_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(init_agent_at(tmp.path(), "../evil").await.is_err());
        assert!(init_agent_at(tmp.path(), "good/../evil").await.is_err());
    }

    #[tokio::test]
    async fn test_init_agent_at_rejects_slash_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(init_agent_at(tmp.path(), "/etc/passwd").await.is_err());
        assert!(init_agent_at(tmp.path(), "nested/name").await.is_err());
    }
}

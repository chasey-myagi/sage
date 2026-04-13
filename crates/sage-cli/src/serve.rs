use anyhow::Result;
use sage_runner::config::{NetworkPolicy, SecurityConfig};
use sage_runner::AgentConfig;
use sage_runtime::engine::{SageEngine, SandboxSettings};
use sage_runtime::event::AgentEvent;
use sage_runtime::types::*;

/// Main serve loop.
///
/// 1. Connect to Rune Runtime
/// 2. Register `agents.execute` rune
/// 3. Handle incoming tasks: parse config → create sandbox → run agent → return result
pub async fn run(runtime_addr: String, _caster_id: String, _max_concurrent: usize) -> Result<()> {
    tracing::info!("connecting to Rune Runtime at {}", runtime_addr);

    // TODO: Phase 2 — Rune Caster SDK integration
    tracing::warn!("serve command is a stub — Rune Caster SDK integration pending (Phase 2)");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    Ok(())
}

/// Run a local test: load config → build SageEngine → run → print events.
pub async fn run_local_test(
    config_path: &str,
    message: &str,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<()> {
    // 1. Load agent config
    let yaml = tokio::fs::read_to_string(config_path).await?;
    let config: AgentConfig = serde_yaml::from_str(&yaml)?;
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
        .system_prompt(&config.system_prompt)
        .provider(provider_override.unwrap_or(&config.llm.provider))
        .model(model_override.unwrap_or(&config.llm.model))
        .max_tokens(config.llm.max_tokens)
        .max_turns(config.constraints.max_turns as usize)
        .timeout_secs(config.constraints.timeout_secs as u64)
        .tool_execution_mode(ToolExecutionMode::Parallel)
        .tool_policy(config.tools.to_policy())
        .builtin_tools(&tool_name_refs);

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
        // Convert runner SecurityConfig → protocol GuestSecurityConfig for the guest.
        let guest_security = to_guest_security(&sandbox.security, &[]);

        builder = builder.sandbox(SandboxSettings {
            cpus: sandbox.cpus,
            memory_mib: sandbox.memory_mib,
            volumes: Vec::new(),
            network_enabled: false,
            security: Some(guest_security),
        });
    }

    Ok(builder.build()?)
}

/// Convert runner `SecurityConfig` → protocol `GuestSecurityConfig`.
///
/// `extra_volume_paths` are guest paths from volume mounts that Landlock
/// must allow read+write access to, in addition to `/workspace` and `/tmp`.
fn to_guest_security(
    config: &SecurityConfig,
    extra_volume_paths: &[&str],
) -> sage_protocol::GuestSecurityConfig {
    let mut allowed_paths: Vec<String> = vec!["/workspace".into(), "/tmp".into()];
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
    fn test_security_allowed_paths_always_set() {
        // Even without explicit allowed_hosts in YAML, allowed_paths should have defaults
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
        assert_eq!(security.allowed_paths, vec!["/workspace", "/tmp"]);
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
}

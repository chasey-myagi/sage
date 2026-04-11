use agent_runner::AgentConfig;
use agent_sandbox::SandboxBuilder;
use anyhow::Result;

/// Main serve loop.
///
/// 1. Connect to Rune Runtime
/// 2. Register `agents.execute` rune
/// 3. Handle incoming tasks: parse config → create sandbox → run agent → return result
pub async fn run(runtime_addr: String, _caster_id: String, _max_concurrent: usize) -> Result<()> {
    tracing::info!("connecting to Rune Runtime at {}", runtime_addr);

    // TODO: Phase 2 — Rune Caster SDK integration
    // let caster = Caster::builder()
    //     .runtime(&runtime_addr)
    //     .caster_id(&caster_id)
    //     .max_concurrent(max_concurrent)
    //     .build()
    //     .await?;
    //
    // caster.rune("agents.execute", |ctx, input| async {
    //     let req: AgentExecuteRequest = serde_json::from_slice(&input)?;
    //     let result = handle_execute(req).await?;
    //     Ok(serde_json::to_vec(&result)?)
    // }).await?;
    //
    // caster.run().await

    tracing::info!("agent caster running (stub mode)");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    Ok(())
}

/// Handle a single agent execution request.
#[allow(dead_code)]
async fn handle_execute(config: AgentConfig, message: String) -> Result<String> {
    let policy = config.tools.to_policy();
    tracing::info!(agent = %config.name, "executing agent task");

    // 1. Create sandbox with policy-derived configuration
    let mut builder = SandboxBuilder::new(&config.name)
        .cpus(1)
        .memory_mib(512)
        .idle_timeout_secs(config.constraints.timeout_secs.into());

    // Mount allowed read paths as read-only volumes
    for path in &policy.allowed_read_paths {
        builder = builder.mount(path, path, true);
    }
    // Mount allowed write paths as read-write volumes
    for path in &policy.allowed_write_paths {
        builder = builder.mount(path, path, false);
    }

    let sandbox = builder.create().await?;

    // 2. Run agent loop inside sandbox
    // TODO: Phase 3 — LLM integration + tool dispatch via sandbox.exec/fs_*
    // For now, just execute a simple command
    let output = sandbox
        .shell(&format!("echo 'Agent {} received: {}'", config.name, message), config.constraints.timeout_secs)
        .await?;

    tracing::info!(exit_code = output.exit_code, "agent task completed");

    // 3. Clean up
    sandbox.stop().await?;

    Ok(output.stdout)
}

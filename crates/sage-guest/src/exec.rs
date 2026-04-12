use anyhow::Result;
use sage_protocol::ExecRequest;
use std::process::Stdio;
use tokio::process::Command;

pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Execute a command request from the host.
pub async fn handle_exec(req: &ExecRequest) -> Result<ExecResult> {
    tracing::debug!(
        command = %req.command,
        args = ?req.args,
        cwd = %req.cwd,
        "executing command"
    );

    let mut cmd = Command::new(&req.command);
    cmd.args(&req.args)
        .current_dir(&req.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (k, v) in &req.env {
        cmd.env(k, v);
    }

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(req.timeout_secs.into()),
        cmd.output(),
    )
    .await??;

    Ok(ExecResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

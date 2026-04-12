use anyhow::Result;

/// Mount essential filesystems for the guest environment.
///
/// This only runs inside a Linux VM, so we use Linux-specific mount calls.
/// On non-Linux (host dev machine), this is a no-op.
pub fn mount_filesystems() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use nix::mount::{MsFlags, mount};
        let none: Option<&str> = None;

        mount(
            Some("proc"),
            "/proc",
            Some("proc"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            none,
        )
        .map_err(|e| anyhow::anyhow!("mount /proc: {e}"))?;

        mount(
            Some("sysfs"),
            "/sys",
            Some("sysfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            none,
        )
        .map_err(|e| anyhow::anyhow!("mount /sys: {e}"))?;

        mount(
            Some("devtmpfs"),
            "/dev",
            Some("devtmpfs"),
            MsFlags::MS_NOSUID,
            none,
        )
        .map_err(|e| anyhow::anyhow!("mount /dev: {e}"))?;

        mount(
            Some("tmpfs"),
            "/tmp",
            Some("tmpfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            none,
        )
        .map_err(|e| anyhow::anyhow!("mount /tmp: {e}"))?;

        tracing::debug!("essential filesystems mounted");
    }

    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!("mount_filesystems is a no-op on non-Linux (guest-agent only runs in VM)");
    }

    Ok(())
}
